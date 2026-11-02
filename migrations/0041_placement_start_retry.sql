-- A runner whose placement failed to start is started again as a new runner,
-- placed with the machines it already failed on tried last. The new runner's
-- intent records which attempt it is, the placement it retries and the
-- machines to try last, so retries stop after a few attempts and one failed
-- placement is retried at most once (the unique index).
ALTER TABLE workload_intents ADD COLUMN start_attempt INTEGER NOT NULL DEFAULT 1
  CHECK (start_attempt > 0);
ALTER TABLE workload_intents ADD COLUMN retry_of_placement_id TEXT
  REFERENCES workload_placements(id);
ALTER TABLE workload_intents ADD COLUMN avoid_host_ids_json TEXT NOT NULL DEFAULT '[]'
  CHECK (json_valid(avoid_host_ids_json) AND json_type(avoid_host_ids_json) = 'array');
CREATE UNIQUE INDEX workload_intents_retry_of_idx ON workload_intents(retry_of_placement_id)
  WHERE retry_of_placement_id IS NOT NULL;

-- A machine's recent start failures and its latest successful start are read
-- for every admission; these keep those reads to the machine's own rows.
CREATE INDEX fleet_events_host_kind_time_idx ON fleet_events(host_id,kind,created_at);
CREATE INDEX host_commands_host_kind_issued_idx ON host_commands(host_id,kind,issued_at);
