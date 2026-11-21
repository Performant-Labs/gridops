# [GitLab: plug into the shared connections and targets UI](https://github.com/Performant-Labs/gridops/issues/141)

**Issue:** Performant-Labs/gridops#141 (part of epic #136, wave 1). Depends on #145 (`e8b18bf`, on master) and #137 (`b22c451`, on master).
**Branch:** claude/gitlab-141-ui-3ae7f5 (this worktree)
**Design:** UI surface. No new layout: the GitLab content lands inside the existing Connections page, Targets picker, Platform options section, runner row and deliveries list.

## Need (issue text)

Operators manage GitLab connections and pools in the web UI. This issue must add **no new pages, nav items or pool-form rows**: GitLab plugs into the pattern from #145.

## Scope (issue text)

- Add a GitLab descriptor to the platform registry in `src/features/platform-connections/`:
  - **Connection fields:** base URL (default `https://gitlab.com`), project or group path, access token. The help text names the needed `create_runner` + `read_api` scopes and the Owner role.
  - **Target kinds:** GitLab project, GitLab group.
  - **Label rules:** GitLab tags. Any character but a comma, at most 255 characters.
  - **Platform options:** the `run_untagged` toggle, shown only in the Platform options section and only when the pool targets GitLab.
  - **Runner link:** `<base_url>/<scope_path>/-/runners/<id>`.
- After a GitLab connection is created, the connection page shows the webhook URL and secret once, with copy buttons and the setting to enable in GitLab (**Job events**). Its row menu gets a secret-rotation action. Both are built through descriptor hooks, not page-specific code.
- The operations view (`src/features/operations/operations.functions.ts`) lists GitLab webhook deliveries and polling errors in the existing list, with a platform badge.

## Done when (issue text)

- The only GitLab-specific frontend code is the descriptor and the API functions it calls. The diff touches no shared page or form layout except to fix gaps the descriptor exposes.
- Vitest covers the descriptor's validation (tags, URL, scope path).
- A headless walkthrough exercises every GitLab control: add, verify, error states (bad token, missing scope), edit, rotate the secret, delete, pick as a pool target, and toggle `run_untagged`.

## Hard limits

- Frontend only: `src/`, `tests/`, `docs/handoffs/141*`. **Do not touch `crates/`, `migrations/` or `docs/openapi/`.** Several GitLab backend issues (#138 API, #139, #140, #148) are in progress in parallel; build against the spec and #138's in-progress wire names below, and test with mocked responses.
- **No new page, route, nav item or pool-form row.** If something cannot be expressed through the descriptor without a layout change, stop and report it; do not change shared layout.
- Shared files may change only to close a descriptor-contract gap generically (no `gitlab` string or GitLab branch in them): `types.ts`, `connections-section.tsx`, `platform-options-section.tsx`, `targets.ts`, the two pool routes (`runner-pools.new.tsx`, `runner-pools.$poolId.settings.tsx`), `schemas.ts`, `runner-row.tsx` / runner type, `webhooks.tsx`. `tests/platform-neutral-source.test.ts` must keep passing (extend it to the new generic code).
- Work headlessly. UI checks use the pipeline's headless Playwright walkthrough only.

## Contract (wins over the issue text where they differ)

### Connection API (`docs/openapi/gridops-v1.json`, from #137)

- `GET /api/v1/platform-connections/gitlab` → `GitlabConnectionList { canManage, items: GitlabConnection[] }`.
- `GitlabConnection { id, provider: "gitlab", name, baseUrl, scopeKind: "project"|"group", scopeId, scopePath, createdAt, updatedAt, canManage }`. Never carries the token, webhook secret or webhook URL.
- `POST …/gitlab` with `CreateGitlabConnectionRequest { name, baseUrl? (default https://gitlab.com), scopeKind, scopePath, accessToken }` → **201** `GitlabConnectionCreated { connection, webhookUrl, webhookSecret }`; 400 invalid input / token refused / missing scope or role; 403; 409 duplicate `(baseUrl, scope)`.
- `PATCH …/gitlab/{id}` with `{ name?, accessToken? }` (partial; **PATCH**, not Bitbucket's PUT) → 200 `GitlabConnection`; 400 when the new token lacks a scope/role; 404.
- `DELETE …/gitlab/{id}` → 204; 409 while a pool, fleet target or live runner references it.
- `POST …/gitlab/{id}/verify` → **204** on success; 400 whose message names the refused token, missing scope or missing role.
- `POST …/gitlab/{id}/webhook-secret/rotate` → 200 `GitlabWebhookSecret { webhookUrl, webhookSecret }`; the old secret stops working at once.
- Error bodies are `ApiErrorBody`; #138 maps every `GitLabError` to 400 with stable substrings: "rejected the access token", the missing scope name, "maintainer"/"owner", the path, "later", "GitLab request failed". The UI shows the server's message.
- Scope is fixed at create: the edit form changes only the name and (optionally) the token. Base URL, scope kind and path are not editable.

### Pool wire (from #138's branch `claude/gitlab-138-api-44d3a8`, `crates/gridops-core/src/models.rs`, `resources.rs`; not yet on master)

- Create: `scope: "gitlab"`, `gitlabConnectionIds: string[]` (UUIDs, ≥1, ≤100, unique, ≤ maxCount), `runUntagged: boolean`, `runnerGroupId: 1`, **and a positive `installationId`** (the create handler still calls `assert_installation_admin` for non-repository scopes). The GitLab payload partial takes the first installation in `context.options.installations`; if none exists the GitLab kinds are still offered but submit fails with the server's/validator's message — record what the UI shows.
- A GitLab pool cannot mix with GitHub repositories/runner groups or Bitbucket workspaces (server rejects); the GitLab kinds `exclude` every GitHub and Bitbucket kind and vice versa via `excludes`.
- Update: `gitlabConnectionIds?` (replaces the set), `runUntagged?`. A pool never changes between GitLab and other scopes.
- Pool options (`RunnerPoolOptions`) and pool detail (`RunnerPoolDetail`) gain `gitlabConnections: [{ id, name, baseUrl, scopeKind, scopePath }]`; detail also has `gitlabConnectionIds` and `runUntagged`. Treat them as optional in the frontend types (default `[]` / `false`) so the UI works against a server that has not shipped #138 yet.
- Labels on a GitLab pool are the runner's tags. Tag rule: any character but a comma, each at most 255 characters (zod rule in `schemas.ts`, exported like `bitbucketLabelError`).

### Target kinds

- Each GitLab connection is exactly one project or one group, so a target option **is a connection** (like Bitbucket workspaces). Two kinds: `gitlab.project` (connections with `scopeKind: "project"`) and `gitlab.group` (`scopeKind: "group"`), cardinality `many`, `standalone: true`, options from the already-loaded `gitlabConnections`; description `<baseUrl>/<scopePath>`.
- Offered on create and on settings (settings sends `gitlabConnectionIds` only for a GitLab pool).
- `run_untagged` default false; `readOnlySummary` lists connection names.

### Runner link

- `<baseUrl>/<scopePath>/-/runners/<id>`. The runners JSON does not carry GitLab fields yet; the frontend reads optional `gitlabRunnerId`, `gitlabBaseUrl`, `gitlabScopePath` on `Runner` (asked of #138) and returns null when any is missing. `Runner.platform` gains `"gitlab"`.

### Deliveries

- #140 stores GitLab deliveries in the shared `webhook_deliveries` table with ids `gitlab:<connection_id>:<key>`; the list API has no platform field. Today a row with no repository/installation is labelled "GitHub App" (`src/routes/_app/webhooks.tsx` ~:119, :146). Derive the platform generically (an id prefix matching a registered descriptor id, else GitHub) and show the platform badge from the registry. No new list or column.
- **Polling errors:** #148 only logs them; no API lists them. Out of reach for #141 — record it as a gap, add nothing.

## Descriptor-contract gaps to close generically

1. **Row actions.** `ConnectionBindings` has Edit and Remove only. Add an optional generic per-row action list (e.g. `actions?: (item) => Array<{ label; run(item): Promise<PostCreateReveal | null | void>; confirm?; success? }>`), rendered by `connections-section.tsx` beside Edit/Remove in the same cluster, a returned reveal shown in the existing `SecretRevealDialog`. GitLab uses it for **Verify** (toast success / show the server error) and **Rotate webhook secret** (confirm, then reveal the new URL and secret once).
2. **Platform option values.** `platformOptions.Component` receives no props, so `run_untagged` has no way into the payload. Add a generic value channel: the forms hold per-platform option values (initialised from the descriptor, e.g. `platformOptions.initialValues(pool?)`), `PlatformOptionsSection` passes `{ values, onChange }` to `Component`, and the payload partials receive them (extra argument to `toCreatePayload` / `toUpdatePayload` through `buildCreatePayload` / `buildUpdatePayload`). Existing descriptors ignore it.
3. **Post-create reveal.** `PostCreateReveal` has one secret value; the GitLab reveal shows the webhook URL and the secret (both with copy buttons) and says to enable **Job events** under the project/group's Settings → Webhooks. Use `details` for the URL and setting if `SecretRevealDialog` already supports copying inside it; otherwise extend the reveal type generically.

Record each gap and its fix in `docs/handoffs/141/decisions.md`.

## Validation (Vitest)

- Base URL: http(s) URL, no query/fragment, trailing slash removed; default `https://gitlab.com`.
- Scope path: non-empty, `segment(/segment)*`, no leading/trailing slash, no `..`, no whitespace.
- Tags: any character but a comma, each ≤ 255 characters (and the usual non-empty after trim).
- Payload partials: create/update for project-only, group-only, mixed project+group, none selected (empty partial that does not override GitHub's), `runUntagged` true/false.
- API functions: each calls the spec path and method with the spec body (mock `api`), including PATCH, 204 verify and rotate.
- Registry: GitLab appears in Add connection's chooser, the Targets picker and Platform options only when targeted (extend the forward-compat tests).

## Walkthrough (headless, against mocked API responses that match the spec)

Add a GitLab connection (reveal shows URL + secret once, copy buttons, Job events note); error states: bad token (400 "rejected the access token"), missing scope (400 naming `create_runner`), duplicate (409); verify success and failure; edit name and replace token; rotate secret (confirm, reveal); delete (and 409 while a pool uses it); pick a GitLab project and a group as pool targets on the new-pool form, see GitHub/Bitbucket kinds excluded; toggle `run_untagged` in Platform options and confirm the POST body carries `scope: "gitlab"`, `gitlabConnectionIds`, `runUntagged`; tag validation error on a comma; runner row link for a GitLab runner; a GitLab delivery row shows the GitLab badge.

## Rules

- Keep every GitLab string in `src/features/platform-connections/gitlab.tsx` and the GitLab functions in `platform-connections.functions.ts` (plus the tag rule in `schemas.ts` and test fixtures).
- Do not deploy anything. Builds and tests use at most 2 jobs (`CARGO_BUILD_JOBS=2`, `RUST_TEST_THREADS=2`); the Mac is shared.
