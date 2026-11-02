-- The built-in runner host is the machine GridOps itself runs on, where the
-- runner manager starts Docker runners. This records whether it may be used.
--
-- An existing install keeps working: the host starts ENABLED when something
-- could still be using it, and DISABLED otherwise. A fresh install has no pools
-- and no runners, so it starts DISABLED. Something could still be using it when
--   * a pool that can start Docker runners has no enabled fleet execution
--     profile, so the host is its only place to run;
--   * a pool's enabled fleet profile lets the host take runners
--     (placement "machines_first" or "host_first"); or
--   * a live runner has a container on the host.
-- The row is only written when absent, so re-running this changes nothing.
INSERT OR IGNORE INTO settings (key, value, updated_by, updated_at)
SELECT 'builtinRunnerHostEnabled',
       CASE WHEN
         EXISTS (
           SELECT 1 FROM runner_pools p
            WHERE p.state <> 'deleting'
              AND (p.provider = 'docker'
                   OR EXISTS (SELECT 1 FROM json_each(p.providers) j WHERE j.value = 'docker'))
              AND NOT EXISTS (
                SELECT 1 FROM pool_execution_profiles x
                 WHERE x.pool_id = p.id AND x.enabled = 1 AND x.archived_at IS NULL)
         )
         OR EXISTS (
           SELECT 1 FROM pool_execution_profiles x
             JOIN runner_pools p ON p.id = x.pool_id
            WHERE p.state <> 'deleting'
              AND x.enabled = 1 AND x.archived_at IS NULL
              AND x.placement IN ('machines_first', 'host_first')
         )
         OR EXISTS (
           SELECT 1 FROM runners r
            WHERE r.deleted_at IS NULL
              AND r.container_id IS NOT NULL
              AND r.status IN ('starting', 'online', 'idle', 'busy', 'paused', 'stopped')
         )
       THEN 'true' ELSE 'false' END,
       NULL,
       CAST(strftime('%s', 'now') AS INTEGER) * 1000;
