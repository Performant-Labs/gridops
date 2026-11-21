# Evidence: #159 pool list and pool detail take the destination from the platform registry

## F (Phase 6)

- **Fact:** the list's new `gitlabConnections` items have exactly the detail's shape, because both queries map their rows through the unchanged `pool_connection_json`, which reads only these five columns (the extra `membership.pool_id` column the shared SELECT now carries is never read into the JSON).
  **Source:** `crates/gridops-api/src/resources/gitlab.rs:584-592`
  **Verbatim excerpt:**
  > fn pool_connection_json(row: &sqlx::sqlite::SqliteRow) -> Value {
  >     json!({
  >         "id": row.get::<String, _>("id"),
  >         "name": row.get::<String, _>("name"),
  >         "baseUrl": row.get::<String, _>("base_url"),
  >         "scopeKind": row.get::<String, _>("scope_kind"),
  >         "scopePath": row.get::<String, _>("scope_path"),
  >     })
  > }

- **Fact:** the pool list query returns one row per pool (it groups by pool id), so the `pool_ids` collected from `rows` hold no duplicates and each pool's map entry is taken once by `remove`.
  **Source:** `crates/gridops-api/src/resources.rs:1343`
  **Verbatim excerpt:**
  > GROUP BY p.id ORDER BY p.created_at DESC

- **Fact:** a `PropertyRow` renders a `dt` label and a `dd` value. The GitLab connection items are plain `div`s, not `PropertyRow`s, so the GitLab Destination group has exactly one `dt` (Scope), which the tests read as the group's property rows.
  **Source:** `src/components/page.tsx:250-257`
  **Verbatim excerpt:**
  > export function PropertyRow({ label, children, title }: { label: string; children: React.ReactNode; title?: string }) {
  >   return (
  >     <div className="grid min-h-8 grid-cols-[96px_minmax(0,1fr)] items-center gap-2 rounded-md text-sm">
  >       <dt className="text-muted-foreground">{label}</dt>
  >       <dd className="flex min-w-0 items-center gap-1.5 truncate text-foreground" title={title}>{children}</dd>
  >     </div>
  >   );
  > }

- **Fact:** the unknown-scope pool fallback spreads `unknownPlatform("unknown")`, whose `name` is the id it is given. No pool hook of the fallback reads `name`, so the raw pool scope is never shown (A warn 2); the fallback is a separate object, not an entry of `platforms`.
  **Source:** `src/features/platform-connections/registry.tsx:20-35`
  **Verbatim excerpt:**
  > function unknownPlatform(id: string): PlatformDescriptor {
  >   return {
  >     id,
  >     name: id,
  >     icon: CircleHelp,

- **Fact:** the host agent reports the OS it was compiled for, so the `fleet_http` fixture's "Linux machine" reads as a macOS machine when the tests run on a Mac. This is why `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails locally on this Mac, independent of #159.
  **Source:** `crates/gridops-host-agent/src/native.rs:201-206`
  **Verbatim excerpt:**
  >         let os = if cfg!(target_os = "macos") {
  >             "macos"
  >         } else if cfg!(target_os = "linux") {
  >             "linux"
  >         } else if cfg!(target_os = "windows") {
  >             "windows"
