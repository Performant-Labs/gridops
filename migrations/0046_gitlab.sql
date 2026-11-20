-- no-transaction
-- GitLab (#137): connections, pool membership, untagged-job opt-in, the job
-- record, and GitLab identities on fleet targets and runners.
--
-- This is the first migration that rebuilds tables. SQLite cannot widen a
-- CHECK in place, so `fleet_ci_targets` (new kinds) and `runners` (the
-- `ci_platform` CHECK) are rebuilt: create the new table, copy every row, drop
-- the old one, rename the new one, then recreate every index and trigger.
--
-- Foreign keys are on for every connection, and `runner_events.runner_id`
-- cascades on delete, so dropping `runners` with them on would delete the
-- runner event history (0020 records the same hazard). The migration therefore
-- runs outside sqlx's transaction, turns foreign keys off (a no-op inside a
-- transaction), does all of its work in one transaction of its own, and checks
-- every reference with `pragma_foreign_key_check` before it commits. That check
-- reports violations whatever the foreign_keys setting is, and a violation makes
-- the guard's CHECK fail, so the transaction never commits a dangling
-- reference. Foreign keys are turned back on for the pooled connection.
PRAGMA foreign_keys=OFF;

BEGIN;

-- 1. One GitLab project or group on one GitLab instance. `base_url` is
-- normalised by #138 (scheme, host, optional port and path, no trailing slash).
CREATE TABLE `gitlab_connections` (
  `id` text PRIMARY KEY NOT NULL,
  `name` text NOT NULL,
  `base_url` text NOT NULL,
  `scope_kind` text NOT NULL CHECK (`scope_kind` IN ('project','group')),
  `scope_id` integer NOT NULL CHECK (`scope_id` > 0),
  `scope_path` text NOT NULL,
  `access_token_key` text NOT NULL,
  `webhook_secret_key` text NOT NULL,
  `created_by` text,
  `created_at` integer NOT NULL,
  `updated_at` integer NOT NULL,
  FOREIGN KEY (`created_by`) REFERENCES `users`(`id`) ON UPDATE no action ON DELETE set null
);
CREATE UNIQUE INDEX `gitlab_connections_scope_unique` ON `gitlab_connections` (`base_url`,`scope_kind`,`scope_id`);
CREATE UNIQUE INDEX `gitlab_connections_access_token_key_unique` ON `gitlab_connections` (`access_token_key`);
CREATE UNIQUE INDEX `gitlab_connections_webhook_secret_key_unique` ON `gitlab_connections` (`webhook_secret_key`);

-- 2. The pools a connection serves, shaped like runner_pool_bitbucket_connections.
CREATE TABLE `runner_pool_gitlab_connections` (
  `pool_id` text NOT NULL,
  `connection_id` text NOT NULL,
  `created_at` integer NOT NULL,
  PRIMARY KEY (`pool_id`,`connection_id`),
  FOREIGN KEY (`pool_id`) REFERENCES `runner_pools`(`id`) ON UPDATE no action ON DELETE cascade,
  FOREIGN KEY (`connection_id`) REFERENCES `gitlab_connections`(`id`) ON UPDATE no action ON DELETE cascade
);
CREATE INDEX `runner_pool_gitlab_connections_connection_idx`
  ON `runner_pool_gitlab_connections` (`connection_id`,`pool_id`);

-- 3. Whether a pool's GitLab runners also pick up untagged jobs.
ALTER TABLE runner_pools ADD COLUMN run_untagged INTEGER NOT NULL DEFAULT 0;

-- 4. GitLab jobs seen by the webhook (#140) or the pending-jobs poll (#148).
-- `id` is a surrogate UUID; a job's identity is (connection_id, job_id).
-- `status` is JobStatus::as_str(); `tag_list` is a compact JSON array of
-- strings in the order GitLab returned them.
CREATE TABLE `gitlab_jobs` (
  `id` text PRIMARY KEY NOT NULL,
  `connection_id` text NOT NULL REFERENCES `gitlab_connections`(`id`) ON DELETE cascade,
  `project_id` integer NOT NULL,
  `job_id` integer NOT NULL,
  `pipeline_id` integer NOT NULL,
  `status` text NOT NULL,
  `tag_list` text NOT NULL DEFAULT '[]',
  `status_changed_at` integer NOT NULL,
  `retries_count` integer NOT NULL DEFAULT 0,
  `gitlab_runner_id` integer,
  `pool_id` text REFERENCES `runner_pools`(`id`) ON DELETE set null,
  `created_at` integer NOT NULL,
  `updated_at` integer NOT NULL
);
CREATE UNIQUE INDEX `gitlab_jobs_connection_job_unique` ON `gitlab_jobs` (`connection_id`,`job_id`);
-- The active jobs (waiting for a runner or holding one), oldest first, for the
-- queue scan. Partial, so manual, scheduled and finished jobs stay out of it.
-- The status list must equal gitlab::GITLAB_ACTIVE_JOB_STATUSES.
CREATE INDEX `gitlab_jobs_active_created_idx` ON `gitlab_jobs` (`created_at`,`id`)
  WHERE `status` IN ('created','waiting_for_resource','preparing','pending','running');

-- 5. fleet_ci_targets gains the gitlab_project and gitlab_group kinds. Every
-- existing shape arm also requires the three GitLab columns to be NULL. The
-- GitLab connection FK is NO ACTION, like the Bitbucket one beside it.
CREATE TABLE fleet_ci_targets_new (
  id TEXT PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('github_repository','github_organization','bitbucket_workspace','bitbucket_repository','gitlab_project','gitlab_group')),
  canonical_key TEXT NOT NULL UNIQUE,
  installation_id INTEGER REFERENCES installations(id),
  repository_id INTEGER REFERENCES repositories(id),
  organization_id INTEGER,
  bitbucket_connection_id TEXT REFERENCES bitbucket_connections(id),
  workspace_uuid TEXT,
  repository_uuid TEXT,
  gitlab_connection_id TEXT REFERENCES gitlab_connections(id),
  gitlab_project_id INTEGER,
  gitlab_group_id INTEGER,
  CHECK (
    (kind = 'github_repository' AND installation_id IS NOT NULL AND installation_id > 0 AND repository_id IS NOT NULL AND repository_id > 0 AND organization_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL AND gitlab_connection_id IS NULL AND gitlab_project_id IS NULL AND gitlab_group_id IS NULL)
    OR (kind = 'github_organization' AND installation_id IS NOT NULL AND installation_id > 0 AND organization_id IS NOT NULL AND organization_id > 0 AND repository_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL AND gitlab_connection_id IS NULL AND gitlab_project_id IS NULL AND gitlab_group_id IS NULL)
    OR (kind = 'bitbucket_workspace' AND bitbucket_connection_id IS NOT NULL AND workspace_uuid IS NOT NULL AND repository_uuid IS NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL AND gitlab_connection_id IS NULL AND gitlab_project_id IS NULL AND gitlab_group_id IS NULL)
    OR (kind = 'bitbucket_repository' AND bitbucket_connection_id IS NOT NULL AND workspace_uuid IS NOT NULL AND repository_uuid IS NOT NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL AND gitlab_connection_id IS NULL AND gitlab_project_id IS NULL AND gitlab_group_id IS NULL)
    OR (kind = 'gitlab_project' AND gitlab_connection_id IS NOT NULL AND gitlab_project_id IS NOT NULL AND gitlab_project_id > 0 AND gitlab_group_id IS NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL)
    OR (kind = 'gitlab_group' AND gitlab_connection_id IS NOT NULL AND gitlab_group_id IS NOT NULL AND gitlab_group_id > 0 AND gitlab_project_id IS NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL)
  )
) STRICT;
INSERT INTO fleet_ci_targets_new
  (id,kind,canonical_key,installation_id,repository_id,organization_id,bitbucket_connection_id,workspace_uuid,repository_uuid)
  SELECT id,kind,canonical_key,installation_id,repository_id,organization_id,bitbucket_connection_id,workspace_uuid,repository_uuid
  FROM fleet_ci_targets;
DROP TABLE fleet_ci_targets;
ALTER TABLE fleet_ci_targets_new RENAME TO fleet_ci_targets;
CREATE TRIGGER fleet_ci_targets_immutable_identity
BEFORE UPDATE ON fleet_ci_targets
BEGIN SELECT RAISE(ABORT,'CI target identity is immutable'); END;

-- 6. runners: ci_platform accepts 'gitlab', and a runner records its GitLab
-- connection and GitLab runner id. Every other column keeps its definition
-- and position; the two new nullable columns are appended after `runtime`.
CREATE TABLE `runners_new` (
	`id` text PRIMARY KEY NOT NULL,
	`pool_id` text NOT NULL,
	`github_runner_id` integer,
	`container_id` text,
	`container_name` text,
	`name` text NOT NULL,
	`os` text DEFAULT 'linux' NOT NULL,
	`architecture` text DEFAULT 'x64' NOT NULL,
	`status` text DEFAULT 'starting' NOT NULL,
	`busy` integer DEFAULT false NOT NULL,
	`ephemeral` integer DEFAULT true NOT NULL,
	`current_job_id` integer,
	`failure_reason` text,
	`last_heartbeat_at` integer,
	`registered_at` integer,
	`deleted_at` integer,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL, `last_job_id` integer REFERENCES `workflow_jobs`(`id`) ON UPDATE no action ON DELETE set null, `configuration_version` integer DEFAULT 1 NOT NULL, `target_repository_id` integer REFERENCES `repositories`(`id`) ON UPDATE no action ON DELETE set null, provider TEXT NOT NULL DEFAULT 'docker'
  CHECK (provider IN ('docker','tart')), `ci_platform` text NOT NULL DEFAULT 'github'
  CHECK (`ci_platform` IN ('github','bitbucket','gitlab')), `bitbucket_connection_id` text
  REFERENCES `bitbucket_connections`(`id`) ON UPDATE no action ON DELETE set null, `bitbucket_runner_uuid` text, runtime TEXT NOT NULL DEFAULT 'vm'
  CHECK (runtime IN ('vm','native')), `gitlab_connection_id` text
  REFERENCES `gitlab_connections`(`id`) ON UPDATE no action ON DELETE set null, `gitlab_runner_id` integer,
	FOREIGN KEY (`pool_id`) REFERENCES `runner_pools`(`id`) ON UPDATE no action ON DELETE cascade
);
INSERT INTO runners_new
  (id,pool_id,github_runner_id,container_id,container_name,name,os,architecture,status,busy,ephemeral,
   current_job_id,failure_reason,last_heartbeat_at,registered_at,deleted_at,created_at,updated_at,
   last_job_id,configuration_version,target_repository_id,provider,ci_platform,bitbucket_connection_id,
   bitbucket_runner_uuid,runtime)
  SELECT id,pool_id,github_runner_id,container_id,container_name,name,os,architecture,status,busy,ephemeral,
   current_job_id,failure_reason,last_heartbeat_at,registered_at,deleted_at,created_at,updated_at,
   last_job_id,configuration_version,target_repository_id,provider,ci_platform,bitbucket_connection_id,
   bitbucket_runner_uuid,runtime
  FROM runners;
DROP TABLE runners;
ALTER TABLE runners_new RENAME TO runners;
CREATE UNIQUE INDEX `runners_container_id_unique` ON `runners` (`container_id`);
CREATE INDEX `runners_pool_status_idx` ON `runners` (`pool_id`,`status`);
CREATE INDEX `runners_last_job_idx` ON `runners` (`last_job_id`);
CREATE INDEX `runners_pool_configuration_idx` ON `runners` (`pool_id`,`configuration_version`);
CREATE INDEX `runners_target_repository_idx` ON `runners` (`pool_id`,`target_repository_id`,`status`);
CREATE INDEX `runners_github_id_idx` ON `runners` (`github_runner_id`);
CREATE INDEX `runners_gitlab_runner_id_idx` ON `runners` (`gitlab_runner_id`);
CREATE INDEX `runners_bitbucket_connection_idx`
  ON `runners` (`pool_id`,`bitbucket_connection_id`,`status`);
CREATE INDEX `runners_gitlab_connection_idx`
  ON `runners` (`pool_id`,`gitlab_connection_id`,`status`);
CREATE INDEX runners_name_idx ON runners(name, created_at);

-- Refuse to commit if any reference anywhere now dangles.
CREATE TEMP TABLE _gitlab_0046_fk_guard (violations INTEGER NOT NULL CHECK (violations = 0));
INSERT INTO _gitlab_0046_fk_guard SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE _gitlab_0046_fk_guard;

COMMIT;

PRAGMA foreign_keys=ON;
