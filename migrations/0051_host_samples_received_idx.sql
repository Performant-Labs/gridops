-- The reconciler prunes machine samples older than the retention window in
-- small batches inside SQLite's single write lock (#222). No index led with
-- `received_at`, so every batch read all of `host_samples` (628k rows on Mars)
-- while holding the lock: up to 13 s when the disk was busy, long enough for
-- the API's writers to give up with `database is locked`. With this index a
-- batch reads only the rows older than the cutoff.
CREATE INDEX host_samples_received_idx ON host_samples(received_at);
