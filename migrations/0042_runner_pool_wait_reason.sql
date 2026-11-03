-- Why a pool's next runner is waiting, in the runner manager's words: for a
-- Tart VM, its place in the queue for the Mac's VM slots and who holds them
-- (#95). A row exists only while the pool waits. A table of its own rather
-- than a column, so the upgrade leaves every existing runner_pools row and
-- its definition as they are.
CREATE TABLE runner_pool_waits (
  pool_id TEXT PRIMARY KEY REFERENCES runner_pools(id) ON DELETE CASCADE,
  reason TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
