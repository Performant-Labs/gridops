-- One-time runner registration material for a placement's start command.
-- The material is sealed with the vault key, readable only by the placement's
-- host while the start can still happen, and erased once it no longer can.
CREATE TABLE fleet_bootstrap_material (
  reference TEXT PRIMARY KEY NOT NULL,
  placement_id TEXT NOT NULL REFERENCES workload_placements(id),
  host_id TEXT NOT NULL REFERENCES fleet_hosts(id),
  sealed TEXT,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
  claimed_at INTEGER,
  erased_at INTEGER,
  CHECK ((erased_at IS NULL) = (sealed IS NOT NULL))
) STRICT;
CREATE INDEX fleet_bootstrap_material_placement_idx ON fleet_bootstrap_material(placement_id);

-- Placements whose runner the reconciler should track.
CREATE INDEX workload_placements_state_idx ON workload_placements(state,updated_at);
