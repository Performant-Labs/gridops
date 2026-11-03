-- A runner pool stores no image for a provider it does not use (issue #103).
--
-- 0017 gave every pool that existed then both providers' default images, and
-- the web form sent both images for every new pool, so a Docker-only pool
-- usually holds the default Tart base VM, and a Tart-only pool the default
-- Docker image. Adding that provider to the pool later would use the default
-- without anyone choosing it.
--
-- This clears an image only when it is the built-in default (0017's column
-- default, also the server's fallback when GRIDOPS_RUNNER_IMAGE or
-- GRIDOPS_TART_RUNNER_IMAGE is not set) and the pool does not use that
-- provider. Any other value may have been typed by an operator and is kept; the
-- API drops it the next time the pool is edited. A pool that uses the provider
-- keeps its image whatever it is. No runner ever used a cleared value, so the
-- pool's configuration version and runners are left alone.
UPDATE runner_pools
   SET tart_image = ''
 WHERE tart_image = 'gridops-macos-tahoe-base'
   AND provider <> 'tart'
   AND NOT EXISTS (SELECT 1 FROM json_each(runner_pools.providers) WHERE value = 'tart');

UPDATE runner_pools
   SET docker_image = ''
 WHERE docker_image = 'ghcr.io/actions/actions-runner:latest'
   AND provider <> 'docker'
   AND NOT EXISTS (SELECT 1 FROM json_each(runner_pools.providers) WHERE value = 'docker');
