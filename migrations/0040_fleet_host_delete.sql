-- A machine that can never take work again can be deleted from the Fleet page.
-- Deleting is soft: the row stays, because placements, commands, events,
-- samples and grants reference the host and must keep rendering. `deleted_at`
-- hides the machine from the list, counts and capacity. A deleted host is also
-- left retired and revoked, so every existing credential, recovery and
-- approval check refuses it without needing to know about this column.
ALTER TABLE fleet_hosts ADD COLUMN deleted_at INTEGER;
CREATE INDEX fleet_hosts_live_idx ON fleet_hosts (deleted_at) WHERE deleted_at IS NULL;

-- The registry checks these conditions with a reason for each; the triggers
-- make them hold even for a caller that skips the registry.
CREATE TRIGGER fleet_hosts_delete_requires_idle
BEFORE UPDATE OF deleted_at ON fleet_hosts
WHEN NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL AND (
  NEW.lifecycle_state != 'retired' OR NEW.enrollment_state != 'revoked'
  OR EXISTS (SELECT 1 FROM host_credentials c WHERE c.host_id = NEW.id AND c.revoked_at IS NULL)
  OR EXISTS (SELECT 1 FROM workload_placements p
              WHERE p.host_id = NEW.id AND p.state NOT IN ('cleaned','failed'))
  OR EXISTS (SELECT 1 FROM capacity_allocations a
              WHERE a.host_id = NEW.id AND a.state != 'released')
)
BEGIN SELECT RAISE(ABORT,'a machine with live work or a live credential cannot be deleted'); END;

-- Deleting is final: a deleted machine is never revived. Re-enrolling creates
-- a new record.
CREATE TRIGGER fleet_hosts_deleted_is_final
BEFORE UPDATE ON fleet_hosts
WHEN OLD.deleted_at IS NOT NULL AND (
  NEW.deleted_at IS NOT OLD.deleted_at OR NEW.enrollment_state != 'revoked'
  OR NEW.lifecycle_state != 'retired'
)
BEGIN SELECT RAISE(ABORT,'a deleted machine cannot be changed back'); END;
