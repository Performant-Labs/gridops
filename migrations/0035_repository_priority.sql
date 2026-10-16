-- A prioritised repository's queued jobs get the pool's next runner first.
-- Any job that waits longer than the pool's priority window counts as
-- prioritised too, so a busy prioritised repository can't starve the others.
-- Both live beside the pool tables rather than in them, so the upgrade stays
-- additive. A repository leaving the pool loses its priority with it.
CREATE TABLE runner_pool_priority_repositories (
  pool_id TEXT NOT NULL,
  repository_id INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (pool_id,repository_id),
  FOREIGN KEY (pool_id,repository_id) REFERENCES runner_pool_repositories(pool_id,repository_id) ON DELETE CASCADE
) STRICT;
CREATE TABLE runner_pool_priorities (
  pool_id TEXT PRIMARY KEY NOT NULL REFERENCES runner_pools(id) ON DELETE CASCADE,
  wait_minutes INTEGER NOT NULL CHECK (wait_minutes BETWEEN 1 AND 1440),
  updated_at INTEGER NOT NULL
) STRICT;
