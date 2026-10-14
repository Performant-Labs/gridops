-- Console output of runners on fleet machines. The host agent ships it per
-- placement while the runner works; GridOps appends it to a runner log stream.
-- `received_bytes` is how much of the runner's output has arrived, so a
-- replayed chunk is appended only once; the stream's `size_bytes` is what its
-- file keeps.
CREATE TABLE fleet_output_streams (
  placement_id TEXT PRIMARY KEY NOT NULL,
  log_stream_id TEXT NOT NULL UNIQUE REFERENCES log_streams(id) ON DELETE CASCADE,
  received_bytes INTEGER NOT NULL DEFAULT 0 CHECK (received_bytes >= 0)
) STRICT;
CREATE INDEX log_streams_runner_idx ON log_streams(runner_id);
