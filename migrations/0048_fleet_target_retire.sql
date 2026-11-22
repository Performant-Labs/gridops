-- no-transaction
-- A GitLab connection's fleet targets retire when the connection is removed
-- (#183). The target row stays, because grants, platform links, workloads,
-- placements, events and runner observations reference it and fleet history
-- must keep rendering; `retired_at` hides it from every path that offers,
-- grants, admits or places new work.
--
-- Retiring is one UPDATE that sets `retired_at` and drops the GitLab
-- connection, so the connection can then be deleted while its foreign key
-- stays NO ACTION: an unretired target still blocks its connection's delete.
-- The shape CHECK accepts a GitLab row without a connection only once it is
-- retired. The identity trigger keeps its name and message and refuses every
-- UPDATE except that one transition, so a retired row never changes again
-- (no un-retiring, no re-retiring, no new connection).
--
-- SQLite cannot widen a CHECK in place, so the table is rebuilt as 0047 did:
-- outside sqlx's transaction, with foreign keys off (a no-op inside a
-- transaction), in one transaction of its own, and with every reference
-- checked by `pragma_foreign_key_check` before it commits. `retired_at` is
-- appended as the last column and is NULL on every existing target.
PRAGMA foreign_keys=OFF;

BEGIN;

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
  retired_at INTEGER,
  CHECK (
    (kind = 'github_repository' AND installation_id IS NOT NULL AND installation_id > 0 AND repository_id IS NOT NULL AND repository_id > 0 AND organization_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL AND gitlab_connection_id IS NULL AND gitlab_project_id IS NULL AND gitlab_group_id IS NULL)
    OR (kind = 'github_organization' AND installation_id IS NOT NULL AND installation_id > 0 AND organization_id IS NOT NULL AND organization_id > 0 AND repository_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL AND gitlab_connection_id IS NULL AND gitlab_project_id IS NULL AND gitlab_group_id IS NULL)
    OR (kind = 'bitbucket_workspace' AND bitbucket_connection_id IS NOT NULL AND workspace_uuid IS NOT NULL AND repository_uuid IS NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL AND gitlab_connection_id IS NULL AND gitlab_project_id IS NULL AND gitlab_group_id IS NULL)
    OR (kind = 'bitbucket_repository' AND bitbucket_connection_id IS NOT NULL AND workspace_uuid IS NOT NULL AND repository_uuid IS NOT NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL AND gitlab_connection_id IS NULL AND gitlab_project_id IS NULL AND gitlab_group_id IS NULL)
    OR (kind = 'gitlab_project' AND (gitlab_connection_id IS NOT NULL OR retired_at IS NOT NULL) AND gitlab_project_id IS NOT NULL AND gitlab_project_id > 0 AND gitlab_group_id IS NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL)
    OR (kind = 'gitlab_group' AND (gitlab_connection_id IS NOT NULL OR retired_at IS NOT NULL) AND gitlab_group_id IS NOT NULL AND gitlab_group_id > 0 AND gitlab_project_id IS NULL AND installation_id IS NULL AND repository_id IS NULL AND organization_id IS NULL AND bitbucket_connection_id IS NULL AND workspace_uuid IS NULL AND repository_uuid IS NULL)
  )
) STRICT;
INSERT INTO fleet_ci_targets_new
  (id,kind,canonical_key,installation_id,repository_id,organization_id,bitbucket_connection_id,workspace_uuid,repository_uuid,gitlab_connection_id,gitlab_project_id,gitlab_group_id)
  SELECT id,kind,canonical_key,installation_id,repository_id,organization_id,bitbucket_connection_id,workspace_uuid,repository_uuid,gitlab_connection_id,gitlab_project_id,gitlab_group_id
  FROM fleet_ci_targets;
DROP TABLE fleet_ci_targets;
ALTER TABLE fleet_ci_targets_new RENAME TO fleet_ci_targets;
-- The only UPDATE allowed: an unretired row becomes retired, every identity
-- column stays as it was, and the GitLab connection stays or becomes NULL.
CREATE TRIGGER fleet_ci_targets_immutable_identity
BEFORE UPDATE ON fleet_ci_targets
WHEN NOT (
  OLD.retired_at IS NULL AND NEW.retired_at IS NOT NULL
  AND NEW.id IS OLD.id
  AND NEW.kind IS OLD.kind
  AND NEW.canonical_key IS OLD.canonical_key
  AND NEW.installation_id IS OLD.installation_id
  AND NEW.repository_id IS OLD.repository_id
  AND NEW.organization_id IS OLD.organization_id
  AND NEW.bitbucket_connection_id IS OLD.bitbucket_connection_id
  AND NEW.workspace_uuid IS OLD.workspace_uuid
  AND NEW.repository_uuid IS OLD.repository_uuid
  AND (NEW.gitlab_connection_id IS OLD.gitlab_connection_id OR NEW.gitlab_connection_id IS NULL)
  AND NEW.gitlab_project_id IS OLD.gitlab_project_id
  AND NEW.gitlab_group_id IS OLD.gitlab_group_id
)
BEGIN SELECT RAISE(ABORT,'CI target identity is immutable'); END;

-- Refuse to commit if any reference anywhere now dangles.
CREATE TEMP TABLE _fleet_0048_fk_guard (violations INTEGER NOT NULL CHECK (violations = 0));
INSERT INTO _fleet_0048_fk_guard SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE _fleet_0048_fk_guard;

COMMIT;

PRAGMA foreign_keys=ON;
