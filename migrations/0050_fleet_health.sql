-- Fleet health (#197, epic #198): job categories, the hourly rollups, the
-- recompute list and the score snapshots. `docs/design/fleet-health.md` is the
-- contract for every table and column here; the wave-1 issues read and write
-- them as written.
--
-- Every new table is STRICT with explicit CHECKs, as 0047's are. Times are
-- epoch milliseconds. An hour is the epoch millisecond it starts at, always a
-- whole hour (`hour % 3600000 = 0`).

-- 1. The ordered rules that classify a job (#190, #199). The first rule, by
-- `position`, whose `pattern` (a case-insensitive regex) matches the job's
-- `field` gives the category; a job no rule matches is `other`, which has no
-- rule of its own but may be the category of one. `fix-agent` is never a job
-- category: #201 counts fix-agent work from fix placements.
CREATE TABLE job_category_rules (
  id INTEGER PRIMARY KEY,
  position INTEGER NOT NULL UNIQUE CHECK (position >= 0),
  category TEXT NOT NULL CHECK (category IN ('test','build','check','deploy','other')),
  field TEXT NOT NULL CHECK (field IN ('job_name','workflow_name','repository','labels')),
  pattern TEXT NOT NULL CHECK (length(pattern) > 0),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
) STRICT;
-- #197 section 2's defaults, all on the job name.
INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES
  (0,'test','job_name','test|spec|e2e|playwright|vitest|jest|pytest|cargo test|shard',0,0),
  (1,'build','job_name','build|compile|bundle|image|docker|package',0,0),
  (2,'check','job_name','lint|clippy|fmt|format|typecheck|tsc|check|audit',0,0),
  (3,'deploy','job_name','deploy|release|publish|pages',0,0);

-- 2. Each job's category. NULL means not classified yet: the writers bind it
-- on insert only (#190), and #190's backfill classifies the rest. The partial
-- indexes let the backfill find unclassified rows without a table scan once
-- it has caught up (they shrink to nothing as it does).
ALTER TABLE workflow_jobs ADD COLUMN category TEXT
  CHECK (category IS NULL OR category IN ('test','build','check','deploy','other'));
CREATE INDEX workflow_jobs_completed_idx ON workflow_jobs (completed_at);
CREATE INDEX workflow_jobs_unclassified_idx ON workflow_jobs (id) WHERE category IS NULL;

-- 3. GitLab jobs (D-1): the category, plus what classifying and timing a
-- GitLab job needs and 0047 did not record. `job_name` and `project_path` are
-- filled once from the webhook or the poll; `started_at` is the status time the
-- job first became `running`, `completed_at` the status time it first became
-- terminal. All NULL on existing rows.
ALTER TABLE gitlab_jobs ADD COLUMN category TEXT
  CHECK (category IS NULL OR category IN ('test','build','check','deploy','other'));
ALTER TABLE gitlab_jobs ADD COLUMN job_name TEXT;
ALTER TABLE gitlab_jobs ADD COLUMN project_path TEXT;
ALTER TABLE gitlab_jobs ADD COLUMN started_at INTEGER;
ALTER TABLE gitlab_jobs ADD COLUMN completed_at INTEGER;
CREATE INDEX gitlab_jobs_completed_idx ON gitlab_jobs (completed_at);
CREATE INDEX gitlab_jobs_unclassified_idx ON gitlab_jobs (id) WHERE category IS NULL;

-- 4. The hourly job rollup (#191): one row per hour, machine, pool and
-- category of the final-attempt jobs that completed in the hour. A job on a
-- runner GridOps doesn't manage is under host and pool `unmanaged`, so there
-- is no foreign key on either. A NULL job category is counted as `other`.
-- The histograms are JSON arrays of the 9 bucket counts (0-10 s, 10-30 s,
-- 30-60 s, 1-2, 2-5, 5-10, 10-20, 20-40 and 40+ min).
CREATE TABLE fleet_health_hourly (
  hour INTEGER NOT NULL CHECK (hour % 3600000 = 0),
  host_id TEXT NOT NULL CHECK (length(host_id) > 0),
  pool_id TEXT NOT NULL CHECK (length(pool_id) > 0),
  category TEXT NOT NULL CHECK (category IN ('test','build','check','deploy','other')),
  job_count INTEGER NOT NULL CHECK (job_count >= 0),
  completed_count INTEGER NOT NULL CHECK (completed_count >= 0 AND completed_count <= job_count),
  infra_failure_count INTEGER NOT NULL CHECK (infra_failure_count >= 0 AND infra_failure_count <= job_count),
  duration_ms_sum INTEGER NOT NULL CHECK (duration_ms_sum >= 0),
  cpu_seconds_sum REAL NOT NULL CHECK (cpu_seconds_sum >= 0),
  queue_wait_histogram TEXT NOT NULL CHECK (json_valid(queue_wait_histogram)
    AND json_type(queue_wait_histogram) = 'array' AND json_array_length(queue_wait_histogram) = 9),
  duration_histogram TEXT NOT NULL CHECK (json_valid(duration_histogram)
    AND json_type(duration_histogram) = 'array' AND json_array_length(duration_histogram) = 9),
  computed_at INTEGER NOT NULL,
  PRIMARY KEY (hour,host_id,pool_id,category)
) STRICT;

-- 5. The hourly machine rollup (#200): one row per machine and hour, written
-- for an hour with no samples too (0 online minutes, NULL usage).
-- `start_failures_json` counts the machine-fault start failures of the hour
-- by `PlacementFailureReason` code, for example {"agent_reported":1}.
CREATE TABLE fleet_host_hourly (
  hour INTEGER NOT NULL CHECK (hour % 3600000 = 0),
  host_id TEXT NOT NULL CHECK (length(host_id) > 0),
  online_minutes INTEGER NOT NULL CHECK (online_minutes BETWEEN 0 AND 60),
  cpu_used_p50_millis INTEGER CHECK (cpu_used_p50_millis >= 0),
  cpu_used_p95_millis INTEGER CHECK (cpu_used_p95_millis >= 0),
  memory_used_p95_mib INTEGER CHECK (memory_used_p95_mib >= 0),
  min_disk_free_bytes INTEGER CHECK (min_disk_free_bytes >= 0),
  start_attempts INTEGER NOT NULL CHECK (start_attempts >= 0),
  start_failures_json TEXT NOT NULL CHECK (json_valid(start_failures_json)
    AND json_type(start_failures_json) = 'object'),
  uncertain_placements INTEGER NOT NULL CHECK (uncertain_placements >= 0),
  computed_at INTEGER NOT NULL,
  PRIMARY KEY (hour,host_id)
) STRICT;

-- 6. Hours whose rollups must be computed again (#190's reclassify adds them;
-- #191 and #200 take them off once they have rolled the hour up).
CREATE TABLE fleet_health_recompute (
  hour INTEGER PRIMARY KEY NOT NULL CHECK (hour % 3600000 = 0),
  requested_at INTEGER NOT NULL
) STRICT;

-- 7. The fleet score over time (#204): one row per recorded hour and window,
-- with every machine's score as a JSON object of host id to score (or null).
CREATE TABLE fleet_health_snapshots (
  recorded_at INTEGER NOT NULL,
  "window" TEXT NOT NULL CHECK ("window" IN ('1h','24h','7d')),
  fleet_score INTEGER CHECK (fleet_score BETWEEN 0 AND 100),
  host_scores_json TEXT NOT NULL CHECK (json_valid(host_scores_json)
    AND json_type(host_scores_json) = 'object'),
  PRIMARY KEY (recorded_at,"window")
) STRICT;
