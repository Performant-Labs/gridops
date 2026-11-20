-- An exclusive pool's runners register without the catch-all
-- self-hosted-linux / self-hosted-macos label, and queue ownership matches
-- the pool only on its own labels, so a pool can be reserved for a class of
-- jobs (#134). A row exists only for an exclusive pool. A table of its own
-- rather than a column, so the upgrade leaves every existing runner_pools row
-- and its definition as they are.
CREATE TABLE runner_pool_exclusive_labels (
  pool_id TEXT PRIMARY KEY REFERENCES runner_pools(id) ON DELETE CASCADE
);
