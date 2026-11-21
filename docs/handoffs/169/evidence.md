# Evidence for #169 (diff gate)

## F (Phase 5)

- **Fact:** a pool's `scope` column is never NULL, so the runners list can read `p.scope` with `row.get::<String,_>` (no `Option`).
  **Source:** `migrations/0001_initial.sql:97-102`
  **Verbatim excerpt:**
  > CREATE TABLE `runner_pools` (
  > 	`id` text PRIMARY KEY NOT NULL,
  > 	`installation_id` integer NOT NULL,
  > 	`repository_id` integer,
  > 	`name` text NOT NULL,
  > 	`scope` text NOT NULL,

- **Fact:** a GitLab connection's `name` is never NULL, so a null `gitlabConnectionName` comes only from the LEFT JOIN (a GitHub or Bitbucket runner, or a removed connection).
  **Source:** `migrations/0047_gitlab.sql:25-27`
  **Verbatim excerpt:**
  > CREATE TABLE `gitlab_connections` (
  >   `id` text PRIMARY KEY NOT NULL,
  >   `name` text NOT NULL,

- **Fact:** the runners list already joins the runner's GitLab connection as a LEFT JOIN, which is where `gc.name` is read from.
  **Source:** `crates/gridops-api/src/resources.rs:1973`
  **Verbatim excerpt:**
  > LEFT JOIN gitlab_connections gc ON gc.id=r.gitlab_connection_id

- **Fact:** the pool list's counts come from a LEFT JOIN of runners onto pools, so a pool with no runners joins one all-NULL `r` row (the row the `total_runners` fix excludes with `r.id IS NOT NULL`).
  **Source:** `crates/gridops-api/src/resources.rs:1337`
  **Verbatim excerpt:**
  > LEFT JOIN runners r ON r.pool_id=p.id

- **Fact:** the registry already treats "no platform said" as GitHub's for webhook deliveries; the absent-`poolScope` default in `platformForPool` follows this precedent.
  **Source:** `src/features/platform-connections/registry.tsx:47-49`
  **Verbatim excerpt:**
  > export function platformForDeliveryId(deliveryId: string): PlatformDescriptor {
  >   return platforms.find((platform) => deliveryId.startsWith(`${platform.id}:`)) ?? githubPlatform;
  > }

- **Fact:** a Bitbucket workspace cannot stand alone as a pool target; it requires a GitHub target, so a Bitbucket runner's pool has a GitHub scope and `platformForPool` resolves it to GitHub (the Bitbucket descriptor has no `pool` key).
  **Source:** `src/features/platform-connections/bitbucket.tsx:43-46`
  **Verbatim excerpt:**
  >   // A pool still needs a GitHub installation and scope (createRunnerPoolSchema), so a workspace
  >   // shares a GitHub target's pool rather than standing alone.
  >   standalone: false,
  >   requires: ["github.repository", "github.runner-group"],
