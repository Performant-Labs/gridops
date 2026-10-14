-- Where a pool with an execution profile runs: only on fleet machines, on
-- machines with the built-in runner host as fallback, or on the built-in host
-- with machines taking the overflow.
ALTER TABLE pool_execution_profiles ADD COLUMN placement TEXT NOT NULL DEFAULT 'machines'
  CHECK (placement IN ('machines','machines_first','host_first'));

-- Log lines shipped by fleet agents, kept for a day.
CREATE TABLE fleet_agent_logs (
  id INTEGER PRIMARY KEY,
  host_id TEXT NOT NULL REFERENCES fleet_hosts(id),
  logged_at INTEGER NOT NULL,
  level TEXT NOT NULL CHECK (level IN ('error','warn','info','debug','trace')),
  target TEXT NOT NULL CHECK (length(target) <= 128),
  message TEXT NOT NULL CHECK (length(message) <= 8192),
  received_at INTEGER NOT NULL
) STRICT;
CREATE INDEX fleet_agent_logs_host_idx ON fleet_agent_logs(host_id,id);
CREATE INDEX fleet_agent_logs_received_idx ON fleet_agent_logs(received_at);
