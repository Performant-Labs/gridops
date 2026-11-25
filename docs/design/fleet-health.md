# Fleet health

The fleet health page (`/fleet/health`, epic #198) answers "is the fleet doing its job, and if
not, where?" with one score for the fleet and one per machine, each explained by its components,
plus the areas of concern, what is queued and running right now, and how each machine's time is
spent by kind of work.

This document is the single source of truth for every number on the page: the job categories,
the rollup tables, the score formulas, the concern rules and the API shape. The issues of #198
implement it as written. A later change rewrites the section it changes; it never appends a
correction.

## Contents

- [How the parts fit](#how-the-parts-fit)
- [Fleet machines](#fleet-machines)
- [Job categories](#job-categories)
- [The hourly rollups](#the-hourly-rollups)
- [The relative speed index](#the-relative-speed-index)
- [Scores](#scores)
- [Areas of concern](#areas-of-concern)
- [The live half](#the-live-half)
- [The API](#the-api)
- [The page](#the-page)
- [Retention](#retention)
- [Testing: the speed check and the synthetic week](#testing-the-speed-check-and-the-synthetic-week)
- [Ownership and the guard](#ownership-and-the-guard)
- [Decisions](#decisions)

## How the parts fit

| Part | Where | Issue |
|---|---|---|
| Job categories: the rules, the classifier | `crates/gridops-core/src/fleet_health/category.rs` | #197 (done) |
| The category of a new job, the cached classifier | `crates/gridops-core/src/fleet_health/classifier_cache.rs` | #190 |
| The backfill and reclassification | `crates/gridops-reconciler/src/fleet_health_backfill.rs` | #190 |
| The hourly job rollup | `crates/gridops-reconciler/src/fleet_health_rollup.rs` | #191 |
| The hourly machine rollup | `crates/gridops-reconciler/src/fleet_host_rollup.rs` | #200 |
| The relative speed index | `crates/gridops-core/src/fleet_health/speed.rs` | #193 |
| The scores and the windowed concerns (pure, no I/O) | `crates/gridops-core/src/fleet_health/score.rs`, `concerns.rs` | #192 |
| The live stats and the live concerns | `crates/gridops-api/src/fleet/health_live.rs` (`collect_live`) | #201 |
| The endpoint: window check, cache, stale rules | `crates/gridops-api/src/fleet/health.rs` | #194 |
| One hour of jobs and machines, shared by the rollups and the source | `crates/gridops-core/src/fleet_health/hour.rs` | #204 |
| Which machines count, and for which part of a window | `crates/gridops-core/src/fleet_health/machines.rs` | #262 |
| The windowed half from the database, the usage by category | `crates/gridops-core/src/fleet_health/window.rs`, `usage.rs` | #204 |
| The database source and the score snapshots | `crates/gridops-api/src/fleet/health_source.rs`, `crates/gridops-reconciler/src/fleet_health_snapshot.rs` | #204 |
| The job category settings | `crates/gridops-api/src/fleet/job_categories.rs`, `src/features/settings/job-categories-section.tsx` | #199 |
| The page frame, Areas of concern, the fleet score card | `src/features/fleet-health/page.tsx`, `areas-of-concern.tsx`, `fleet-score-card.tsx` | #195 |
| The machine table and detail | `src/features/fleet-health/machine-score-table.tsx`, `machine-detail.tsx` | #202 |
| The live strip, the category legend, the Overview tile | `src/features/fleet-health/live-strip.tsx`, `category-legend.tsx` | #203 |

The shared types are in `crates/gridops-core/src/fleet_health/mod.rs` (Rust) and
`src/features/fleet-health/types.ts` (zod). The example response is
`src/features/fleet-health/fixture.json`, generated from `fleet_health::fake::fixture(Day)`; the
same JSON is the `OpenAPI` 200 example of `GET /api/v1/fleet/health`, and tests parse it with both
the Rust types and the zod schema, so the four can't drift apart.

The endpoint gets its data from a `FleetHealthSource` (`#[async_trait]`, held as
`Arc<dyn FleetHealthSource>` on the health state): `windowed(window, now)` (scores, speed, usage,
windowed concerns; the slow, cached part) and `live(now)` (a bounded read of now). `assemble`
joins them: each machine gets its live stats by id, and the concerns are merged by
`merge_concerns`. Production serves the database source (`DbFleetHealthSource`, #204), which
`FleetHealthState::new(db)` selects: its windowed half is `fleet_health::window::windowed_health`
with the speed indexes from its cache, and its live half is `collect_live`. The speed index reads
raw jobs (the 7 d one about 0.4 s on production's data, against the page's 300 ms), so it is kept
off the request path: a refresher the API starts reads every window's indexes every 4 minutes,
one window at a time, and a request is served the window's last indexes whatever their age
(refreshing them behind the call when they are over 5 minutes old). A request reads them itself
only when its window has never been read, the first page before the refresher's first round. A
refresh that fails or panics skips only its window until the next round. No field
of a production response comes from `fleet_health::fake`: the fake (`FakeFleetHealthSource`, the
fixture) is behind `test-support` and compiled only into tests, which inject it with
`AppState::with_fleet_health_source`.

## Fleet machines

One rule (`fleet_health::machines`, #262) decides which `fleet_hosts` rows the page, its snapshots
and its rollups count. A re-enrolled machine is a new row with its own `created_at`; its old row
stays behind, approved and `retired`, with the `retired_at` of the re-enrollment, so the page
must never read it as a machine that went offline.

- **A fleet machine** is approved, not deleted and not retired (`fleet_machine_now!`). The live
  half reads only these: the live machine list and every live concern (offline, start gate, start
  failures, disk, uncertain placements).
- **Over a window**, a machine counts for the part of its life inside the window's covered time
  `[ceil_hour(now − W), now)`: from its `created_at` until its `retired_at` (only while its
  lifecycle is `retired`). A machine retired before the window starts isn't on the page at all. A
  machine retired during the window counts only for the time before `retired_at`, and a machine
  created during the window only from `created_at`, so a new machine isn't scored offline for the
  hours before it existed. The window's machines are the approved, non-deleted ones whose life
  overlaps the covered time (`fleet_machine_overlaps!`); each one's covered time (its *span*,
  `HostInfo::span`) is the denominator of its availability and busy share, and of its share of
  the fleet's availability (see [Scores](#scores)), and only its rows (machine and job) of an hour
  its span reaches count for it. A retired row without a `retired_at` counts as retired before
  any window.
- **The machine rollup** writes a row for every `fleet_hosts` row, whatever its enrollment, that
  existed during the hour, so a revoked or deleted machine's starts before it went still count for
  the fleet's start reliability, and a retired machine gets no more rows (deleting a machine
  retires it at its deletion if it was not retired already).
- **The speed index** reads a job only when its machine is approved, not deleted, and existed when
  the job completed (`fleet_machine_existed_at!`).

## Job categories

A finished job is one of `test`, `build`, `check`, `deploy` or `other`. `fix-agent` is a sixth
category on the page that is never stored: #201 counts fix-agent work from fix placements, so it
only appears in the live counts.

The rules are ordered (`job_category_rules`, by `position`). Each matches one field of the job
with a case-insensitive regular expression (at most 256 KiB compiled): `job_name`,
`workflow_name`, `repository` (`owner/name`, or the GitLab project path) or `labels` (any one
label matches). The first rule that matches decides; a job no rule matches is `other`. A rule
whose pattern doesn't compile is skipped with a warning by `compile`, and refused by `validate`
(which names each failing position) when an administrator saves it.

The defaults, seeded by migration `0050`:

| Position | Category | Field | Pattern |
|---|---|---|---|
| 0 | `test` | `job_name` | `test\|spec\|e2e\|playwright\|vitest\|jest\|pytest\|cargo test\|shard` |
| 1 | `build` | `job_name` | `build\|compile\|bundle\|image\|docker\|package` |
| 2 | `check` | `job_name` | `lint\|clippy\|fmt\|format\|typecheck\|tsc\|check\|audit` |
| 3 | `deploy` | `job_name` | `deploy\|release\|publish\|pages` |

**When a job is classified.** The three writers bind `category` when they first insert a job row,
in the `INSERT`'s `VALUES` only; their `ON CONFLICT` updates never touch it, so a later event
never changes a stored category (W-15). They are `process_workflow_job` (GitHub webhook,
`crates/gridops-api/src/webhooks.rs`), `upsert_workflow_job` (GitHub poll,
`crates/gridops-reconciler/src/main.rs`) and `record_gitlab_job` (GitLab webhook and poll,
`crates/gridops-core/src/gitlab/jobs.rs`). Each calls
`classifier_cache::category_for_insert(&mut conn, &NewJob, now)` on its own connection (never a
pool inside a write transaction). A GitHub job reads its workflow name and repository from
`workflow_runs` by `run_id` (`JobOrigin::GitHubRun`); a GitLab job carries its connection,
project id and, when the writer has it, its project path (`JobOrigin::GitLabProject`; the poll has
no path, so #190 finds it from the ids, for example from another of the project's rows). `None` leaves the job unclassified; the backfill classifies what is
still NULL.

**The cache (#190).** One compiled classifier per process, rebuilt only when the rules change
(`MAX(updated_at)` and `COUNT(*)` of `job_category_rules`), checked at most once a minute.

**The backfill (#190).** Every reconciler pass, in batches of 500 rows, one short transaction
each: rows with `category IS NULL` (through the partial indexes `workflow_jobs_unclassified_idx`
and `gitlab_jobs_unclassified_idx`). When the rules version changes, rows completed in the last
7 days are classified again the same way, and their hours are listed in `fleet_health_recompute`.
The rules version last applied is kept in `settings` under `fleet_health.rules_version` (no table
of its own).

**GitLab jobs** had no name, project path or start and finish times. Migration `0050` adds them,
nullable, and the writer fills them:
- `job_name` and `project_path` are `COALESCE(stored, incoming)`: filled once, also by a write
  whose status is rejected (`FilledOnly`), and never overwritten or cleared. The webhook passes
  `build_name` and `project.path_with_namespace`; the poll passes the job API's `name` and no
  path (the webhook fills it).
- `started_at` is set from `status_changed_at` the first time an applied status is `running`,
  and `completed_at` the first time an applied status is terminal; neither is ever overwritten.
- Rows written before `0050` keep NULL in all of them; the backfill classifies them from what
  they have (their tags at least).

## The hourly rollups

The reconciler rolls each finished hour up once, so the page never reads raw jobs or samples
over a window (#179's lock rules). Both rollups run every pass, log-and-continue, before the
runner manager is asked (so an unavailable manager never stops them). Each pass of the job
rollup catches up its finished hours oldest first, as many as fit in 1 s of the pass (one at
least), then rolls up every hour listed in `fleet_health_recompute` (those the catch-up just
listed included, so a re-run across two hours settles in one pass); the machine rollup catches
up the same way (#204). On first start the catch-up covers the last 7 days, which takes a few
passes, not 168. An hour is written with one `INSERT … ON CONFLICT DO UPDATE` per row in
one short transaction (the job rollup also takes the hour off `fleet_health_recompute` in it), so
a re-run gives identical rows. Reads happen outside the write transaction, bounded by the
`completed_at` indexes (`workflow_jobs_completed_idx`, `gitlab_jobs_completed_idx`) or
`host_samples_host_time_idx`. The reads of an hour are one copy, in `gridops-core`
(`fleet_health::hour::job_hour` and `host_hours`, #204): the rollups write a finished hour from
them, and the source computes its live hours with them, so a live hour and a rolled one can't
differ in how they are computed.

**The grace (#239).** No hour, listed or caught up, is rolled before `hour_end + 15 min`
(`ROLLUP_GRACE_MILLIS` in `gridops_core::fleet_health`, the one copy of the rule, which #204's
source uses too). A job finishing at 10:59:40 whose webhook was missed is recorded by the poll
at about 11:00:40, and an agent can report samples late; both still count in their own hour,
and the catch-up still rolls each hour once. So an hour has rows only once it is past its grace,
and the hours of the window after that are computed live by the source (see
[Scores](#scores)).

**Histogram buckets** (queue wait and duration), lower bound inclusive, as
`HISTOGRAM_LOWER_BOUNDS_MILLIS`: 0–10 s, 10–30 s, 30–60 s, 1–2 min, 2–5 min, 5–10 min,
10–20 min, 20–40 min, 40+ min. A negative value (clock skew) counts as 0. Stored as a JSON array
of 9 counts.

### `fleet_health_hourly` (#191)

One row per (hour, host, pool, category) of the final-attempt jobs **completed** in the hour:

| Column | Meaning |
|---|---|
| `hour` | The hour's start, epoch ms, a multiple of 3,600,000 |
| `host_id`, `pool_id` | The job's placement: `workflow_jobs.runner_id` → `runners.github_runner_id` (GitLab: `runners.gitlab_runner_id`) → `workload_placements`. A job no managed placement ran is under `host_id = pool_id = 'unmanaged'` (non-NULL sentinels, because STRICT primary keys can't hold NULL; no foreign key on `host_id`). |
| `category` | The job's category; NULL counts as `other` |
| `job_count` | Every final-attempt job completed in the hour |
| `completed_count` | Those with conclusion `success` |
| `infra_failure_count` | Those whose placement ended `failed` or `uncertain`. A failing test is not an infrastructure failure. |
| `duration_ms_sum` | Σ (`completed_at − started_at`) |
| `cpu_seconds_sum` | Σ (`cpu_millis / 1000 × duration in seconds`); 0 for unmanaged |
| `queue_wait_histogram` | Of `started_at − created_at` (`created_at` is when GridOps first saw the job) |
| `duration_histogram` | Of the durations |
| `computed_at` | When the row was written |

Only the final attempt of a re-run counts, per job: a job counts unless a later attempt of its
run ran a job of the same name: a higher `workflow_job_sync.run_attempt`, or the higher job id
when either attempt is unknown and the run row knows a re-run (`run_attempt` above 1). A
"re-run failed jobs" attempt runs only the failed jobs, so the run's other jobs keep their
earlier attempt and still count, in their own hour (#239). When an hour is rolled, the other
hours of the last 7 days in which its re-run jobs' earlier attempts (same run, same job name)
completed
(through `workflow_jobs_run_status_idx`; never the hour itself) are listed in
`fleet_health_recompute` in its last write transaction, so they are rolled again without them
and a re-run whose attempts fall in different hours counts once (#239). Until the later
attempt's hour is rolled (at its end + 15 min, then one more pass for the listed hour), the
earlier attempt still counts in its own hour, and a live hour can count the later one: an
oracle computing final attempts from raw data has to model that interval, or avoid seeding a
re-run that crosses into an unrolled hour.

### `fleet_host_hourly` (#200)

One row per (hour, machine) for every machine that existed during the hour (see
[Fleet machines](#fleet-machines)), also for an hour with no samples (0 minutes, NULL percentiles):

| Column | Meaning |
|---|---|
| `online_minutes` | Minutes of the hour with at least one `host_samples` row (0–60) |
| `cpu_used_p50_millis`, `cpu_used_p95_millis`, `memory_used_p95_mib` | From the hour's `host_samples` (NULL without samples). Nearest-rank or interpolated; the tests' series give the same answer either way. |
| `min_disk_free_bytes` | The lowest `disk_free_bytes` of the hour |
| `start_attempts` | The machine's placements that reached `starting` in the hour |
| `start_failures_json` | `placement_failed` events of the hour that `start_health::is_start_failure(reason, agent_code)` puts on the machine, by `PlacementFailureReason` (`{"<reason>": n}`) |
| `uncertain_placements` | Placements of the machine that ended `uncertain` in the hour |

### `fleet_health_recompute` and `fleet_health_snapshots`

`fleet_health_recompute (hour, requested_at)` lists hours for the job rollup to roll up again:
the backfill adds them on a rules change, and the job rollup adds a re-run's earlier-attempt
hours. Only the job rollup reads it and takes hours off it; the machine rollup's rows don't
depend on job categories, so it never reads it (#239). `fleet_health_snapshots (recorded_at, "window", fleet_score,
host_scores_json)` records the fleet score and each machine's (`{"<host id>": score|null}`) once
an hour per window (1 h, 24 h and 7 d) for the page's trend (#204,
`crates/gridops-reconciler/src/fleet_health_snapshot.rs`). The snapshot pass runs after the
rollups every reconciler pass and writes a window's row at `recorded_at = now` when that window
has none yet in the current clock hour and the rollups have the hour that just ended: it is past
its grace, the job rollup's catch-up has gone past it, the machine rollup's has reached it, and no
hour up to it is listed in `fleet_health_recompute`. So the first snapshots of an hour come at
about a quarter past, never while the catch-up after downtime is still filling the window, and
past hours are never backfilled. The scores are the page's: `window::inputs` and `score_fleet` at
`now`, with the speed indexes read fresh.

## The relative speed index

How long a machine takes to run the jobs the rest of the fleet also runs, at one runner size,
against the fleet's median (#193, `speed::speed_indexes(db, window, now)`):

1. A job key is `repository / workflow / job name` (`speed::job_key`), with a trailing shard
   counter stripped (` (n/m)`, ` [n/m]` or ` n/m`, then trimmed), so a suite's shards compare
   together. A GitLab job's key is `gitlab:<instance>/<project path> / <job name>`
   (`speed::gitlab_job_key`, the same stripping: GitLab names a `parallel` job's copies
   `name 1/4`, …), the instance being its connection's `base_url` without the scheme or a
   trailing `/`, so a machine running only GitLab jobs has an index too (#253). A GitLab job name
   may hold ` / `, so without the prefix GitLab project `acme/app`'s job `deploy / prod` would be
   GitHub repository `acme/app`'s workflow `deploy`, job `prod`; the `gitlab:` prefix (a GitHub
   repository name holds no `:`) means no GitLab key ever equals a GitHub key, and the instance
   means two GitLab instances' projects of the same path never share one.
2. Only successful final-attempt runs completed inside the window (`[now − W, now)`) with a start
   time count (a GitLab job is its own only attempt), on a machine that is approved, not deleted,
   and existed when the job completed ([Fleet machines](#fleet-machines), #262). The speed index's rule is stricter than the
   rollup's, since a dropped sample only thins a key: a GitHub job whose attempt is known is left
   out when any later attempt of its run is known (another job of the run with a higher known
   attempt, or the run row's `run_attempt` above the job's), whatever the job's name. So a
   partial re-run's jobs that were not run again are not speed samples, though the rollup counts
   them (#239). A job whose own attempt is unknown is kept. They are grouped by key
   and by (machine, runner size), the runner size being the placement's `cpu_millis`. A job's
   machine is its runner's newest `runners` row and that row's newest `workload_placements` row,
   as the hourly rollup reads it, so duplicate rows never count a job twice (#239). GitLab jobs
   (#253) are read the same way, per hour through `gitlab_jobs_completed_idx`: `status='success'`
   is success and a job is its own only attempt, as in the rollup; a job without a name, a path or
   a start (written before `0050`) is left out; a job recorded under several connections of one
   GitLab instance counts once (its lowest row id); its runner is the newest `runners` row with
   its `gitlab_runner_id` on the same instance (`base_url`).
3. For each key, the fleet median is the true median duration of its runs on every machine, the
   machine's own runs included, so a machine running most of a key's jobs drifts toward 1.0 on
   it; each (machine, size) has its own median.
4. The index of a (machine, size) is the run-count-weighted geometric mean of (own median ÷ fleet
   median) over the keys with at least 5 runs on that machine at that size and at least 5 on the
   rest of the fleet. 1.0 is typical, 1.5 half again as slow.
   A key whose median on the machine or on the fleet is 0 or less (sub-second jobs at GitHub's
   second granularity) is left out.
5. Fewer than 3 such keys gives `index: null` ("not enough data").
6. `topKeys` are the keys that push the index up most, with both medians and both run counts:
   only keys with a ratio above 1.0 (slower than the fleet), worst first, at most 5 (#239). So a
   machine with an index but no key slower than on the rest of the fleet has none: the machine
   detail's "Slowest job keys" then says "No job runs slower here than on the rest of the fleet",
   and "Not enough data" only when no runner size has an index (#253).

## Scores

Pure functions over `FleetHealthInputs` (`score.rs`, #192; no `sqlx`, `tokio`, `reqwest` or
`std::fs`, which `the_scoring_engine_does_no_io` checks). Each component is linear between its
"100 at" and "0 at" values (`HealthTargets`, these are the defaults) and clamped to 0–100. A
component with too little data (fewer than 20 jobs, or no rows) has `score: null` and the
deduction "not enough data", and its weight is spread over the others in proportion (the weights
still sum to 100). A score with no component left is `null`.

**Rounding (#239).** Components are combined from their unrounded scores, and only the final
score is rounded. Each component on the wire carries its own rounded score. The busy-share gate
(the fleet's queue-wait component under 100) and the concern bands read the rounded component
score.

**The covered time (#239).** The window's rows are whole hours, from `ceil_hour(now − W)` to
`now`, the hour in progress included. Every time denominator is that covered time,
`now − ceil_hour(now − W)`, which is the full window only on the hour (and always more than
`W − 1 h`), cut for each machine to its span, the part of it the machine existed in (#262,
[Fleet machines](#fleet-machines)): from `max(ceil_hour(now − W), created_at)` to
`min(now, retired_at)`. Only a machine's rows of the hours its span reaches count for it.
- Availability, fleet and machine, divides by the whole covered minutes, the unit
  `online_minutes` counts in (minutes with at least one sample): the minutes from the one the span
  starts in to the one it ends in, that one excluded (`floor(end ÷ 60,000) − floor(start ÷
  60,000)`, which is `floor(covered ms ÷ 60,000)` for a machine that existed all window). A
  machine online every whole minute is fully available while the minute in progress is
  unsampled. No whole minute covered (the first 59 s of an hour on 1h) is not enough data, which
  is not 0.
- Busy share divides by the machine's covered seconds.
- Capacity fit counts the ended hours from `ceil_hour(now − W)` (the hour in progress is not yet
  one of them), as before. An hour is a misfit when one of its jobs waited at least 1 min (the
  1–2 min bucket and up) while the CPU used p50 of the machines with a sample that hour, summed,
  was under half of those same machines' CPU budgets; an hour with no sampled machine is not a
  misfit.

**The grace-hour contract with #204.** The source reads `fleet_health_hourly` and
`fleet_host_hourly` only for hours past their grace (`hour + 1 h + ROLLUP_GRACE_MILLIS <= now`).
It computes live, bounded, every other hour of `[ceil_hour(now − W), now)`: the hour in progress,
and the hour that just ended until `hour_end + 15 min` (`hour::live_hours`), each over
`[hour, min(hour + 1 h, now))` by the rollups' own `hour::job_hour` and `host_hours`. It detects an unwritten hour by time, with
the shared constant, not by the rollups' markers. A live hour has the same row shape as a rolled
one, and counts online minutes the same way. A past-grace hour the rollups haven't written yet is
covered time with nothing in it (its machines read offline, with no jobs): the pass lag, at most
one reconcile interval (30 s by default), or the catch-up after downtime, a few passes.

**Fleet:**

| Key | Label | Weight | Measures | 100 at | 0 at |
|---|---|---|---|---|---|
| `queue-wait` | Queue wait | 40 | p90 wait of every job in the window, from the merged histograms | ≤ 1 min | ≥ 10 min |
| `start-reliability` | Start reliability | 20 | successful starts ÷ start attempts (machine-fault failures only), on every machine that started runners in the window, one revoked or deleted since included | ≥ 99% | ≤ 80% |
| `infra-failures` | Infrastructure failures | 15 | infrastructure-failed jobs ÷ every completed final-attempt job (`job_count`: successes and failures alike, so an outage still scores) | 0% | ≥ 5% |
| `capacity-fit` | Capacity fit | 15 | share of hours with jobs waiting over 1 min while the fleet's CPU used p50 was under 50% of budget | 0% | ≥ 25% |
| `availability` | Availability | 10 | Σ (each machine's online minutes × its CPU budget) ÷ Σ (its covered minutes × its CPU budget), each machine's online minutes capped at its covered minutes first (its sampled minute in progress can put it one over) | 100% | ≤ 80% |

**Machine:**

| Key | Label | Weight | Measures | 100 at | 0 at |
|---|---|---|---|---|---|
| `availability` | Availability | 25 | online minutes ÷ its covered minutes, read as 100% when the sampled minute in progress puts it over | 100% | ≤ 80% |
| `start-reliability` | Start reliability | 25 | as above, this machine only | ≥ 99% | ≤ 80% |
| `relative-speed` | Relative speed | 25 | the speed index (#193) | ≤ 1.0 | ≥ 2.0 |
| `headroom` | Headroom | 15 | the lower of disk (minimum free against `disk::alarm_for`'s levels) and memory (p95 used ÷ budget: 100 under 80%, 0 at 100%) | | |
| `busy-share` | Busy share | 10 | CPU-seconds reserved ÷ (CPU budget × its covered time); deducts only while the fleet's queue-wait component has a score under 100 (without one, or at 100, it scores 100) | ≥ 50% | 0% |

Only a machine with no rows in the window has no availability data: rows with 0 online minutes,
which the machine rollup writes on purpose for an hour without samples, score availability 0.
A machine with a CPU budget of 0 has no score: every component is "not enough data" (saying it
has no CPU budget where the component had data), and it raises no `host-score` concern (#239).

**Percentile from a histogram:** find the bucket where the cumulative count crosses the
percentile and interpolate linearly inside it; the 40+ min bucket counts as 40 min.

**Bands:** 90–100 good, 70–89 fair, under 70 poor, and none without a score (`scoreBand` in
`src/components/charts/chart-theme.ts`, the same thresholds as the Overview's success tone). Each
band has one status tone (`bandTone`: good `success`, fair `warning`, poor `danger`, none
`neutral`), shared by the gauge, the cards, the machine table and the Overview tile.

**Deduction sentences:** one sentence with the measured value, the target and the points lost,
for example `Queue p90 was 7 min 12 s against a 1 min target: −28.` The points lost are
`round(weight × (100 − score) / 100)` from the unrounded score and the weight **after**
reweighting, so a score's deductions add up to what it lost, give or take rounding (#257). A
component with no deduction has `deduction: null`.

## Areas of concern

A concern is `{key, severity, title, detail, subject, link, since}`. The `key` is stable for the
same condition (for example `host-offline:<host id>`); `subject` is `{kind: "fleet"}`,
`{kind: "host", id}` or `{kind: "pool", id}`; `link` points to what explains it
(`/fleet/health#machine-<host id>`, `/fleet/health#fleet-component-<component key>`,
`/queued-jobs`, `/fleet/machines/<host id>`, `/settings/general`); `since` is when it started
(epoch ms), or `null`.

**Order** (`merge_concerns(windowed, live)`): one concern per key (a live concern replaces a
windowed one with the same key), worst severity first (`critical`, `warning`, `info`), then the
oldest `since` first, with no `since` after every dated concern of its severity; ties keep their
order (windowed before live).

**Windowed** (`concerns.rs`, #192), with the deduction sentence as `detail`. A component without
enough data never raises one.

| Key | Severity | When |
|---|---|---|
| `fleet-component:<component key>` | critical under 50, warning under 70 | a fleet component scores below that; links to the component's row on the fleet score card, `/fleet/health#fleet-component-<component key>` (#253) |
| `host-score:<host>` | critical under 50, warning under 70 | a machine scores below that; `detail` names its worst scored component |
| `host-slow:<host>:<cpu millis>` | warning | a speed index ≥ 1.3 at one runner size, naming the slowest job key |
| `host-idle-while-queued:<host>` | warning | busy share under 30% while the fleet's queue-wait component is under 70 |
| `category-other` | info | more than 20% of the window's jobs are `other` |

**Live** (`collect_live`, #201):

| Key | Severity | When |
|---|---|---|
| `host-offline:<host>` | critical | a fleet machine (not retired, #262) with a CPU budget above 0 has no `host_samples` row in the last 5 minutes; `since` is its last sample |
| `start-gate:<host>` | warning | `start_health::start_gate` sets the machine aside from new placements |
| `start-failures:<host>` | warning from 25 %, critical from 50 % | at least 3 start failures in the last hour (as the machine's live stats count them, below) that are at least 25 % of its starts in the hour. The denominator counts only the placements issued a `start_runner` command in the hour (`host_commands_host_kind_issued_idx`, as the machine rollup counts `start_attempts`): a failure before any start command (`prepare_deadline_expired`), or of a start issued before the hour, adds to the failures and not to the starts. So the starts are clamped to at least the failures, and the share is at most 100 % ("io failed 4 of 4 runner starts" for 4 such failures and 1 start). Title "Jupiter failed 9 of 24 runner starts in the last hour"; links to the machine's row (`/fleet/health#machine-<host>`); `since` is the oldest of those failures (#253) |
| `disk:<host>` | critical or warning | the Overview's disk alarm, from the same read (`crate::resources::fleet_root_disks`, #240): `disk::alarm_for` on the newest sample's free space against the root disk's observed size (`fleet_observed_domains`, not the admin-set budget), only for an approved machine that is not retired and whose heartbeat (`last_heartbeat_at`) is at most 120 s old |
| `unmatched-jobs` | critical | queued jobs no pool can run (the Overview's split, `fleet_queue_breakdown`, plus the waiting GitLab jobs no pool will run, #253) |
| `oldest-queued` | critical over 10 min, warning over 5 min | the oldest queued job's wait, GitHub or GitLab; `detail` names the job and the repository (a GitLab job's project path) |
| `uncertain:<host>` | warning | the machine has placements in state `uncertain` |

`title` is short ("europa is offline"); `detail` is one sentence with the numbers.

## The live half

`collect_live(db, now) -> (FleetLiveStats, Vec<HostLiveStats>, Vec<Concern>)` (#201) reads only
what is true now, with no principal, through the fleet-wide reads #197 added for it and the
Overview's disk read:

- `gridops_core::fleet::registry::host_live_reads(pool, now)`: every fleet machine (approved, not
  deleted, not retired: `fleet_machine_now!`, #262) in id order, built by the same helpers
  `/fleet/hosts` uses for an administrator (counts, `physicalBudget`, `physicalAllocation`, `latestRootMeasurement`, placement failures in the last
  hour, the start gate), in one read transaction. Its placement-failure count is every
  `placement_failed` event of the hour, as `/fleet/hosts` shows it, read as a count only (not the
  day of failures `/fleet/hosts` also lists); the page does not use it (#253). Its counts are over the machine's live
  placements only (#240): the unfinished ones through `workload_placements_host_idx(host_id,state)`
  and the finished ones still holding capacity, never its finished history, which retention
  never deletes; every count equals `/fleet/hosts`' but the known-runner count;
- `crate::resources::fleet_queue_breakdown(pool, now)`: the Overview's queue split
  (`split_queue` over `waiting_job_pools`) over every installation, with the queued jobs by
  category (NULL as `other`). For a user who sees every installation it gives exactly the
  Overview's numbers;
- `crate::resources::fleet_root_disks(pool, now)`: the read behind the Overview's fleet disk
  alarms (#240), so the two always agree, over the same fleet machines (`fleet_machine_now!`);
- `gridops_core::autoscaling::waiting_gitlab_jobs(pool)` (#253): the GitLab jobs waiting for a
  runner, which the Overview's split leaves out. One per GitLab instance and job id however many
  connections recorded it, `pending` as the furthest along of its rows (a `created` job waits for
  earlier stages; a running one has its runner), read on the partial index
  `gitlab_jobs_active_created_idx`, each under the pool `waiting_job_pools`' ownership rule gives
  it (an autoscaling pool, else the first matching manual-mode pool) or unmatched. They are added
  to the queued total, by category (NULL as `other`), by pool and to the unmatched count; one an
  autoscaling pool will run is a candidate for the oldest queued job.

**Per machine:** online (a sample in the last 5 minutes), or offline with the last-seen time;
runners running now by category (each running CI placement whose runner has a
`current_job_id` or is busy, `status='busy'` and `busy=1`, counted once: a GitLab runner is only
ever marked busy. A GitHub runner's `current_job_id` gives the job's category; a GitLab runner's
job is the `gitlab_jobs` row running on it (its `gitlab_runner_id` on the runner's instance,
status `running`, on the partial index of active jobs, #253); no job row, or a NULL category, is
`other`. Plus fix-agent runs from fix placements); CPU and memory reserved against the budget;
CPU, memory and disk used from the latest sample; start failures in the last hour: the
`placement_failed` events that `start_health::is_start_failure` puts on the machine, read on
`fleet_events_host_kind_time_idx` and judged as the start gate and the machine rollup judge them,
so an operator's release or a runner removed before its start never counts (#253). **Fleet:** queued jobs by
category and by pool, the unmatched count and the oldest queued job with its wait; running jobs
by category. The per-category counts add up to their totals. Every query is bounded (by state
indexes or `observed_at` within the last hour); the whole function should take under 50 ms on
the production database.

## The API

`GET /api/v1/fleet/health?window=1h|24h|7d` returns `FleetHealth`
(`docs/openapi/gridops-v1.json`; its 200 example is the fixture):

```text
{ window, generatedAt, stale,
  fleet: { score, components[], trend[{at, score}], categories[CategoryUsage] },
  live: { queuedTotal, queuedByCategory, queuedByPool[{poolId, poolName, queued}], queuedUnmatched,
          oldestQueued{jobId, jobName, repository, queuedAt, waitMs} | null,   // see below
          runningTotal, runningByCategory },
  hosts[{ id, name, cpuBudgetMillis, memoryBudgetMib, diskTotalBytes, score, components[],
          speed[{cpuMillis, index, topKeys[{key, hostMedianMs, fleetMedianMs, hostRuns, fleetRuns}]}],
          categories[{category, jobs, cpuHours, medianDurationMs, p90QueueWaitMs}],
          usage[{hour, cpuHours: {<category>: hours}}], live: HostLiveStats | null }],
  concerns[Concern] }
```

Every object refuses unknown fields on both sides (`deny_unknown_fields`, `z.strictObject`).
`live.oldestQueued.jobId` is the platform's own job id: a GitHub workflow job id, or a GitLab job
id (#253; `repository` is then the GitLab project path), so it is not unique across platforms and
is only shown, never looked up. The response does not carry the platform: adding it means changing
the fixture, the fake and #204's oracle with it, so it waits for a change that needs it.
Machines are in id order on the wire; the page sorts them worst score first.

What the database source (#204) puts in the windowed fields:
- **`hosts`:** the window's machines ([Fleet machines](#fleet-machines)): every approved,
  non-deleted machine whose life overlaps the covered time, so also one retired during the
  window (its `live` is `null`: the live half reads only machines not retired), with
  `cpuBudgetMillis` and `memoryBudgetMib` from its root budget (`host_resource_domains`, 0 without
  one) and `diskTotalBytes` the size of its root disk as its agent observed it
  (`fleet_observed_domains`, the size the disk alarm and the headroom score read; `null` before
  any observation), not the admin-set budget disk, which is a share of it.
- **`speed`:** one entry per runner size the machine ran successful jobs at in the window,
  smallest first.
- **`categories`** (fleet and machine): per category with jobs, in category order, the window's
  final-attempt jobs, CPU-hours (`cpu_seconds_sum ÷ 3600`, to the hundredth), the median duration
  (p50 of the merged duration histograms) and the p90 queue wait (of the merged wait histograms),
  both interpolated as the queue-wait score is and rounded to the millisecond
  (`fleet_health::usage`). The fleet's folds every row, unmanaged runners' included; a machine's
  folds its own.
- **`usage`:** the machine's CPU-hours by category per hour (to the hundredth), oldest first, one
  entry per hour with any CPU-hours and in it each category with some.
- **`trend`:** the window's snapshots recorded in `[now − W, now]`, oldest first; none yet is an
  empty list ("No trend yet"). Past snapshots are never made up.

- **Who:** system administrators only (D-4), like `GET /api/v1/fleet/workload`: `FleetUser<Read>`
  plus `require_admin`. A member gets `403 forbidden`; no session, `401` as `/fleet/hosts` gives
  it. An administrator's `fleet:read` API token is accepted (`TOKEN_SCOPED_ROUTES`,
  `docs/fleet-api.md`).
- **Window (#194):** anything but `1h`, `24h` or `7d` is a 400 naming the allowed values; a
  missing `window` means `24h`, as the workload chart's does.
- **Cache (#194):** per window for 30 s on the health state, read by its `RegistryClock`.
  Concurrent requests for an expired window wait for one computation. When the source fails, the
  last response is served with `"stale": true` while it is under 10 minutes old, and a 503
  `dependency_unavailable` after that.
- **Locks, timeout and retries (#240):** each window has its own lock, held only by a
  computation of that window; a cache hit takes no lock, so it never waits on a computation. A
  computation over 2 s is given up on and counts as a failure. After a failure a window is not
  computed again for 30 s, and serves stale or 503 (the rule above) meanwhile. The live part is
  read once per computation; computations of several windows in flight at once share one live
  reading, and nothing of it is kept once it is read (no cache across requests).

**Job category settings** (#199), session-only and administrators only (registered in
`session_routes()`, under `refuse_bearer`; a token gets `403 token_not_accepted`):

| Route | Body | Answer |
|---|---|---|
| `GET /api/v1/settings/job-categories` | | `{"rules": [{position, category, field, pattern}]}`, positions `0..n` |
| `PUT /api/v1/settings/job-categories` | `{"rules": [{category, field, pattern}]}`, the whole list in order (empty allowed), at most 200 rules and 1 KiB (UTF-8) per pattern | the saved list, with a `job_categories.updated` audit event (the lists before and after) in the same transaction; an invalid pattern (or one over 1 KiB) is a 422 `{error, code: "invalid_job_category_rules", details: [{position, message}]}`, a list over 200 rules a 422 with empty `details`, and nothing is saved |
| `POST /api/v1/settings/job-categories/preview` | the proposed list | `{"rules": [{position, matches}], "other": [{name, jobs}]}`: jobs completed in the last 7 days each rule would classify (first match wins), and the 20 commonest names left in `other` |

The 422 is `ApiError::Invalid { code, message, details }`; no other error body changed. The page checks the same limits before it sends anything, shows each 422 message on its row (from a save or a preview), and calls the preview unavailable, not invalid, on a server or network failure; a newer edit aborts the preview in flight (#240). The
browser's `ApiError` (`src/lib/api.ts`) keeps the body's `code` and `details`.

## The page

`/fleet/health` (`src/routes/_app/fleet.health.tsx` → `src/features/fleet-health/page.tsx`),
**Health** right after **Machines** in the sidebar (shortcut `G T`; Machines is not highlighted
on it). The page owns the window (1 h, 24 h, 7 d), remembered in `localStorage` under
`gridops.fleetHealth.window` (24 h by default and when storage fails; #195), and reloads at the
user's refresh setting from a 30 s base. A member sees "Administrators only" instead of an error.
A stale response shows a warning above the sections.

The sections, each taking `{ data, window }` and using only the router's `Link`: the live strip
and category legend (#203), Areas of concern and the fleet score card (#195), the machine table
and detail (#202). The machine table and detail also share the expanded machine, which the frame
owns (#241): the frame reads the router's hash (`useRouterState`), opens the machine a
`#machine-<host>` hash names on load and whenever the hash changes, and passes `expanded`,
`onExpandedChange` and `hash` to the table and `expandedId` to the detail, so the detail slot
shows the machine expanded in the table. Without those props the table keeps its own expansion.
Each section sits in its own slot, an element carrying `data-health-section`: `live-strip`,
`areas-of-concern`, `fleet-score`, `category-legend`, `machine-table` and `machine-detail`, six
in all (the stale warning is not a slot). The slots are part of the composition: wave 1 fills a
slot's content and may move the slot, but keeps the attribute. No slot renders while loading, on
a 403 or on a failure. #197's page test (`tests/fleet-health-page.test.ts`) asserts the
composition slots.

- **Areas of concern:** the first five concerns in the API's order, then all on "Show all N";
  "No areas of concern" when empty; rows keyed by `key` (`data-concern-key`), so a refresh keeps
  them, and a concern new since the previous data is `data-new="true"`.
- **Fleet score card:** the score in its band (`ScoreGauge`, `src/components/charts/score-gauge.tsx`,
  which carries `data-band`), the trend, and a row per component (`data-component-key`,
  `data-insufficient="true"` without a score) with its deduction. Each row has the id
  `fleet-component-<key>`, the anchor its `fleet-component:<key>` concern links to (#253).
- **Live strip:** totals in `data-testid="live-queued-total"` / `"live-running-total"`, one
  element per category with jobs (`data-queued-category` / `data-running-category`), and the
  oldest wait (`formatOldestWait`: "45 s", "12 min", "3 h 5 min", "1 h"). Under the queued side,
  the queue by pool as the API orders `queuedByPool` (`data-queued-pool`, each a link to its pool
  page, `/runner-pools/<pool id>`) and the jobs no pool will run (`queuedUnmatched`,
  `data-testid="live-queued-unmatched"`, a link to `/queued-jobs`); nothing when both are empty
  (#253).
- **Machine detail, slowest job keys:** the speed index's `topKeys` (see
  [The relative speed index](#the-relative-speed-index)): "No job runs slower here than on the
  rest of the fleet" when a runner size has an index but no key is slower, "Not enough data" when
  none has an index (#253).
- **Colours:** the theme's variables only, light and dark: the band tones, and
  `--category-<category>` (`categoryColor`).
- **The Overview tile (#203):** `metricTiles(metrics, health)` takes the 24-hour health (the
  Overview calls `useFleetHealth("24h")`, administrators only) and adds a "Fleet health" tile
  linking to `/fleet/health`: the score, its band's tone, and a hint counting critical concerns.
  The strip widens to six columns when it has six tiles.

## Retention

`cleanup_retention` keeps 90 days of `fleet_health_hourly` (by `hour`), `fleet_host_hourly` (by
`hour`) and `fleet_health_snapshots` (by `recorded_at`), each deleted through its primary key's
leading time column.

## Testing: the speed check and the synthetic week

`fleet_health::speed_check::check(&pool, work)` (test builds, `test-support`) runs `work` and
fails when any statement on that database takes over 100 ms (the #179 slow-statement log) or any
write transaction holds the write lock over 50 ms. In test builds `connect_database_path`
instruments every connection with SQLite's progress, update, commit and rollback hooks; a write
transaction is timed from its first row change to its commit or rollback, and while a check is
armed a connection's worker thread routes its `tracing` events to the check's own dispatcher
(set as that thread's default from a hook), which records sqlx's slow-statement warning. Limits:
a `BEGIN IMMEDIATE` held before any write, or a write to a `WITHOUT ROWID` table, is not timed;
a statement that has run fewer than 1,000 instructions and changed no row is timed by the
worker's CPU since the previous statement's log line (one instruction can walk a whole table:
`SELECT COUNT(*)`), so one slow only because it waited for the lock is not a slow statement; and
a check sees a worker's statements only once one of its hooks has run on that worker during the
check, so such a read as the first statement on a connection in a check is not seen at all.
Catching those is a non-goal for #197: a wave-1 rollup can't rely on the check to flag a lock
that `BEGIN IMMEDIATE` holds before its first write, a slow write to a `WITHOUT ROWID` table, or
an unbounded `SELECT COUNT(*)` that is a connection's first statement in a check.

The timing checks measure the work, not the machine (#247). Wall-clock time counts the time a
loaded host keeps the work's threads off the CPU, or its disk keeps a write waiting, as if the
work had spent it: at load 22+ the synthetic week failed with "held the write lock 71 ms" and
passed on a rerun, so a red check could not be told from a regression. The limits stay (100 ms a
statement, 50 ms a write transaction, 20 s to build the synthetic week). What changed is what is
measured, from Linux's `/proc/<thread>/schedstat` for the connection's worker thread and the
thread that called `check`:

- a statement, from its first progress callback or row change to sqlx's log line for it, is the
  CPU time those two threads used, plus the worker's time blocked in the kernel when the statement
  read from the disk (a scan on a cold database waits for its own pages): what the statement did,
  whatever else the host was doing;
- a write transaction is wall-clock time less the time those two threads spent runnable and
  waiting for a CPU (`run_delay`), less the time the worker spent blocked in the kernel inside
  statements that return no rows and read nothing from the disk (a write's disk I/O, which a busy
  disk stretches; a scan's reads from a cold database, such as a copy of Mars's, still count), and
  never less than the CPU time the two used.

A full scan counts in full in both: it holds the lock as long on a busy host as on an idle one. A
transaction still counts the caller's work between statements, a lock held while the caller sleeps
or waits on anything but a CPU, and a worker blocked inside a statement that returns rows (it may
be waiting for the caller to take them), except the statement the transaction commits in, whose
rows are not known until its log line after the commit: an autocommit `INSERT … RETURNING` whose
rows the caller takes slowly has that wait taken off, as an autocommit write's disk stall must be.
Timing transactions by CPU time alone, or by counting statements and rows, would miss a lock held
across a slow call. `speed_check::work_time(&pool,
work)` times any work the transaction's way (the synthetic week's 20 s budget). Limits: two
threads waiting for a CPU at once both have their waits taken off (the CPU-time floor bounds it);
a check whose work runs its statements from other threads (a multi-thread runtime moving the
task) keeps those threads' waits; CPU time itself still grows when the host slows the CPU (a busy
hyperthread sibling, a lower clock); where there is no `schedstat` (macOS) the timings are
wall-clock time, as before.

Two more ways the check went wrong under load or by test order (#247): it waited a fixed 25 ms
for the last statement's log line, which sqlx writes after the caller has its row, so a slow
statement on a busy CPU could be missed; it now waits until every statement it saw start has
logged, and fails if one has not within 10 s. And tracing-core, while only one dispatcher is registered, asks the default of the thread
that first reaches a callsite whether it is of interest, so once sqlx's statement log was first
reached outside a check, no check saw it again; the check registers a second, inert dispatcher so
that tracing-core asks the registered ones.

`work_slowed_by_a_busy_cpu_passes_and_real_work_still_fails` (`tests/fleet_health_speed_check.rs`)
pins a child of the test binary to one CPU with `taskset` (util-linux), beside 11 spinning
threads: about 15 ms of work passes there though the clock shows it past both limits, while ten
times that work, a 300 ms read, or a lock held through a 150 ms sleep still fails. Each timing
check was also shown to turn red with a full scan put back into its write transactions (#247's
pull request).

The agent tests in `crates/gridops-api/tests/fleet_http/busy_database.rs` read what the recovery
proxy recorded only after waiting for it: the proxy records a request once the API's reply
reaches it, so for a moment the database shows a new session or a delivered result that the proxy
has not recorded. A relay between the proxy and the API holds the handshake and result replies
for 500 ms, so a test that reads the proxy as soon as the database changes fails every time
instead of once in a while.

`fleet_health::testdata` seeds realistic data in batches: `seed_fleet` (the five fixture
machines with their budgets, one installation, repository `acme/app` and a pool labelled
`self-hosted`, `gridops`), `JobSeed` and `insert_jobs`, `insert_samples`,
`insert_placement_failure`, `insert_start_attempt`, and `build_week(&pool, end)`: 7 days, about
50,000 GitHub jobs on the four budgeted machines and a sample every 30 s for all five, in about
8 s, so the whole speed-check set stays under 60 s in CI.

**The end-to-end test and the oracle (#204).** `crates/gridops-api/src/fleet/health_e2e_tests.rs`
seeds two days of the five fixture machines (jobs, a re-run across an hour boundary, a job in the
hour still in its grace, a pathless GitLab job on a group-scoped connection, a machine with no
budget and one never online, a re-enrolled machine (a retired row named callisto, retired 30 h
before the evaluation hour, beside the active one: not on the 1 h and 24 h pages, on 7 d scored
only for its hours before `retired_at`, and never live nor with a live concern), a machine
created 6 h before the evaluation hour (scored only from its `created_at`), a nearly full disk with a fresh heartbeat and a low one with a stale
heartbeat, a busy GitLab runner with no job row and one whose running job has its row, GitLab
jobs on three machines, start attempts and failures (machine-fault failures in the last hour
beside an operator's release, and a revoked machine's starts), capacity allocations, stored rules
on the workflow name and the labels, and snapshots), runs the reconciler's own passes
(`gridops_reconciler::fleet_health_pass`, the reconciler's library target) until they have caught
up, and reads the endpoint for every window through the production source, at two moments (the
ended hour in its grace, then rolled). `scripts/fleet-health-oracle.py` (Python standard library
only, no GridOps code) computes the same page from the raw tables by this document; the test
requires the two to agree: scores and components to the integer, each component's deduction
sentence exactly, weights, CPU-hours and usage to 1e-6, speed indexes to 0.01 with their keys
exactly, the trend (and the scores the snapshot pass recorded, the fleet's and every machine's),
the live counts and each machine's reserved CPU and memory (its unreleased capacity allocations
on the root domain, seeded as admission writes them), and the concerns by key, severity, `since`
and order. The pass runs in the binary's order because both run `fleet_health_passes`, the one
copy of it. The oracle also classifies every job by the stored rules and
reports each job whose stored category differs: before the backfill can classify it the pathless
GitLab job is stored NULL and counts as `other` on the page and in the oracle alike, and after it
both count it as `test`. With `GRIDOPS_FLEET_HEALTH_E2E_KEEP=<dir>` the test keeps the seeded
database and the responses.

The wave-1 contract tests are written already (`*_contract_tests.rs` and
`tests/fleet-health-*.contract.test.ts`), marked `#[ignore = "contract: #N"]` or
`it.skip("[contract: #N] …")`. Each issue removes its own markers and makes them pass.

## Ownership and the guard

`docs/design/fleet-health-owners.json` lists, per wave-1 issue, the files it owns (globs allowed)
and the shared files where it may only add lines; every other file is #197's. The contract test
files stay #197's whatever glob matches them. The `Fleet health guard` CI job
(`scripts/fleet-health-guard.mjs`, tested by `node --test scripts/fleet-health-guard.test.mjs`)
checks every pull request whose description closes (`Closes`, `Fixes` or `Resolves #N`, any
case) a wave-1 issue: each changed file must be the issue's own, an add-only change to one of its
shared files, or the removal of its own markers from a contract test. A pull request closing no
wave-1 issue passes; one closing two fails. #204 is wave 2 and not checked; its files are listed
under `wave2` for the record: `health_source.rs`, `fleet_health_snapshot.rs`, the source selection
in `health.rs` and `state.rs` (`FleetHealthState::new(db)`), the shared hour module and the code
moved out of the rollups, `usage.rs` and `window.rs`, the reconciler's library target and its
shared pass order, the end-to-end test, the oracle and the UI walkthrough report. #253
(`wave2b`, also not checked) runs beside it and owns the live half's fixes: `health_live.rs`, the
speed index's GitLab read, the concern links, the pending-GitLab and failed-placement reads, and
the live strip, machine detail and score card anchors.

Rust issues add their own tests inside the module they own (`#[cfg(test)] mod tests`), because
every `mod` line is in a #197 file; frontend issues add theirs as
`tests/fleet-health-<issue>-*.test.ts`.

## Decisions

- **D-1, classification at the real writers.** The three writers above bind `category` from one
  seam #190 owns (`category_for_insert`), on the caller's connection, in `VALUES` only.
- **D-2, frontend tests in `tests/`.** Vitest collects `tests/**/*.test.ts` only.
- **D-3, every hook here.** The health state on `AppState` (its type and default source in
  `fleet/health.rs`, so #194 never edits `state.rs` or `main.rs`; #204 passes the pool to
  `FleetHealthState::new` in `state.rs`, since the database source needs it); the fleet-wide queue
  breakdown and host read for #201; `metricTiles`' second parameter and `useFleetHealth("24h")` on
  the Overview; the 422 `ApiError::Invalid`; every contract test's `mod` line in a #197 file.
- **D-4, administrators only,** with `fleet:read` tokens accepted, as the workload chart.
- **The cache's clock is on the health state** (`RegistryClock`, set with
  `AppState::with_fleet_health_clock`), not on the fake; the fake counts calls, fails on demand
  and holds calls.
- **The fixture** is `fake::fixture(Day)`: five machines (io, Jupiter, callisto, europa and
  MacBookPro, CPU budgets 22.8, 19.2, 8, 12.8 and 0), callisto slow at 2 CPUs, Jupiter with start
  failures, a queue backlog 3 hours before `FIXTURE_NOW`, every severity of concern. Its scores,
  deductions and concerns are written by hand for such a fleet; #192's contract tests check them
  against the engine run on `fake::fixture_inputs()`, whose hourly rows fold each machine's day
  into one hour per (pool, category) (the scores only read sums).
- **Rollup semantics** the contract tests pin: `job_count` is every final-attempt job completed in
  the hour and `completed_count` those that succeeded; unmanaged is `'unmanaged'` with 0
  CPU-seconds; a pass rolls up every listed hour and at least one catch-up hour (as many as fit in
  its time budget since #204).
- **The rules version** (#190) is a `settings` key, not a schema change.
