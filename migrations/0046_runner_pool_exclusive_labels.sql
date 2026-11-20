-- An exclusive pool's runners register without the catch-all
-- self-hosted-linux / self-hosted-macos label, and queue ownership matches
-- the pool only on its own labels, so a pool can be reserved for a class of
-- jobs (#134). Off for every existing pool.
ALTER TABLE runner_pools ADD COLUMN exclusive_labels INTEGER NOT NULL DEFAULT 0;
