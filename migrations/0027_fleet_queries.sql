-- Durable query retention boundaries are global stream facts, independent of
-- a reader's grants or filters. Future pruning must advance the matching floor
-- in the same transaction as deletion. This migration does not prune history.
CREATE TABLE fleet_query_retention_floors (
  stream TEXT PRIMARY KEY NOT NULL CHECK (stream IN ('samples','fleet_events')),
  first_retained INTEGER NOT NULL CHECK (first_retained >= 1)
) STRICT;

INSERT INTO fleet_query_retention_floors (stream,first_retained)
VALUES ('samples',1),('fleet_events',1);

CREATE TRIGGER fleet_query_retention_floor_monotonic
BEFORE UPDATE ON fleet_query_retention_floors
WHEN NEW.stream IS NOT OLD.stream OR NEW.first_retained < OLD.first_retained
BEGIN SELECT RAISE(ABORT,'query retention floor cannot move backwards'); END;

CREATE TRIGGER fleet_query_retention_floor_no_delete
BEFORE DELETE ON fleet_query_retention_floors
BEGIN SELECT RAISE(ABORT,'query retention floor cannot be deleted'); END;

CREATE INDEX host_samples_host_received_idx
ON host_samples(host_id,received_at,id);
CREATE INDEX host_samples_host_domain_cursor_idx
ON host_samples(host_id,domain_id,id);
CREATE INDEX host_backends_host_id_idx ON host_backends(host_id,id);
CREATE INDEX host_resource_domains_host_id_idx ON host_resource_domains(host_id,id);
