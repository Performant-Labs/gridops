-- How long each reconciler pass took. A runner whose machine has finished
-- preparing waits for the next pass to be told to start, so the overview
-- shows these beside the time runners take to start.
CREATE TABLE reconciler_passes (
  id INTEGER PRIMARY KEY,
  started_at INTEGER NOT NULL,
  duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0)
) STRICT;
CREATE INDEX reconciler_passes_started_idx ON reconciler_passes(started_at);
-- The overview times each runner start from its placement's commands and its
-- first job; these keep those lookups from scanning the whole tables.
CREATE INDEX host_commands_placement_idx ON host_commands(placement_id,kind);
CREATE INDEX runner_events_runner_idx ON runner_events(runner_id,event,created_at);
