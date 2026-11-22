-- The reconciler prunes old inventory snapshots in small batches inside
-- SQLite's single write lock (#152). Without this index every batch that found
-- fewer rows than its limit (every steady-state pass) read the whole retained
-- table, about 4 KB a row, while holding the lock; with it a batch reads only
-- the rows older than the cutoff.
CREATE INDEX fleet_inventory_snapshots_received_idx ON fleet_inventory_snapshots(received_at);
