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

**Amended Done-when (after A round 1; wins over the line above):** GitLab-specific code is limited to `gitlab.tsx`, the GitLab API functions in `platform-connections.functions.ts`, the GitLab pool fields, `validateGitlabConnections` refinement and `gitlabLabelError` in `schemas.ts`, and the optional GitLab fields on the pool types (`runner-pools.functions.ts`) and the runner type (`operations.functions.ts`). Every other changed shared file closes a descriptor gap generically, with no GitLab string.
- A headless walkthrough exercises every GitLab control: add, verify, error states (bad token, missing scope), edit, rotate the secret, delete, pick as a pool target, and toggle `run_untagged`.

## Hard limits

- Frontend only: `src/`, `tests/`, `docs/handoffs/141*`. **Do not touch `crates/`, `migrations/` or `docs/openapi/`.** Several GitLab backend issues (#138 API, #139, #140, #148) are in progress in parallel; build against the spec and #138's in-progress wire names below, and test with mocked responses.
- **No new page, route, nav item or pool-form row.** If something cannot be expressed through the descriptor without a layout change, stop and report it; do not change shared layout.
- Shared files may change only to close a descriptor-contract gap generically (no `gitlab` string or GitLab branch in them): `types.ts`, `connections-section.tsx`, `platform-options-section.tsx`, `targets.ts`, `registry.tsx`, the two pool routes (`runner-pools.new.tsx`, `runner-pools.$poolId.settings.tsx`), `runner-row.tsx`, `webhooks.tsx`, and `github.tsx` (gap 4 only). `tests/platform-neutral-source.test.ts` must keep passing; extend it to the new generic code and to `webhooks.tsx`.
- **Wire-field files (Bitbucket pattern, amended after A round 1, block 2):** `src/features/runner-pools/schemas.ts` and `src/features/runner-pools/runner-pools.functions.ts` carry the GitLab pool fields exactly as they carry Bitbucket's (see "Pool fields in the shared schema and types"), and `src/features/operations/operations.functions.ts` gets the optional GitLab runner fields. These are the only GitLab names allowed outside `gitlab.tsx` and the GitLab API functions.
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
- Labels on a GitLab pool are the runner's tags. **Tag rule (amended, A finding 8):** the shared 64-character label cap (`schemas.ts:13`, and the server's `models.rs`) wins over the issue's 255. `gitlabLabelError` checks only non-empty and no comma; the shared schema enforces the 64 cap. Raising the cap would be a separate server change.

### Target kinds

- Each GitLab connection is exactly one project or one group, so a target option **is a connection** (like Bitbucket workspaces). Two kinds: `gitlab.project` (connections with `scopeKind: "project"`) and `gitlab.group` (`scopeKind: "group"`), cardinality `many`, `standalone: true`, options from the already-loaded `gitlabConnections`; description `<baseUrl>/<scopePath>`.
- Offered on create and on settings (settings sends `gitlabConnectionIds` only for a GitLab pool).
- `run_untagged` default false; `readOnlySummary` lists connection names.

### Runner link

- `<baseUrl>/<scopePath>/-/runners/<id>`. The runners JSON does not carry GitLab fields yet; the frontend reads optional `gitlabRunnerId`, `gitlabBaseUrl`, `gitlabScopePath` on `Runner` (asked of #138) and returns null when any is missing. `Runner.platform` gains `"gitlab"`.

### Deliveries

- #140 stores GitLab deliveries in the shared `webhook_deliveries` table with ids `gitlab:<connection_id>:<key>`; the list API has no platform field. Today a row with no repository/installation is labelled "GitHub App" (`src/routes/_app/webhooks.tsx` ~:119, :146). Derive the platform generically (an id prefix matching a registered descriptor id, else GitHub) and show the platform badge from the registry. No new list or column.
- **Polling errors:** #148 only logs them; no API lists them. Out of reach for #141 — record it as a gap, add nothing.

## Pool fields in the shared schema and types (Bitbucket pattern; A round 1, block 2)

`createRunnerPoolAction` / `updateRunnerPoolAction` send the zod-parsed input (`runner-pools.functions.ts:127-138`), and `z.object` strips unknown keys, so the GitLab fields must be in the schema:

- **`schemas.ts`:** `scope` enum gains `"gitlab"`. `gitlabConnectionIds`: `z.string().uuid()` array, unique, at most 100; default `[]` on create, optional on update. `runUntagged`: boolean, default `false` on create, optional on update. A `validateGitlabConnections` refinement mirroring `validateBitbucketConnections` and #138's `validate_gitlab_connections`: scope is `gitlab` exactly when there is at least one GitLab connection; a GitLab pool has no repositories and no Bitbucket workspaces; count at most `maxCount`. Export `gitlabLabelError` (non-empty, no comma).
- **`runner-pools.functions.ts`:** optional `gitlabConnections: { id; name; baseUrl; scopeKind; scopePath }[]` on `BaseOptions` (pool options) and `RunnerPoolDetail`, optional `gitlabConnectionIds` and `runUntagged` on the detail, and `scope` gains `"gitlab"`.
- **Vitest:** a GitLab create and a GitLab update survive `parse*RunnerPoolInput` with `scope`, `gitlabConnectionIds` and `runUntagged` intact; the refinement rejects a GitLab pool with repositories, a non-GitLab scope with GitLab connections, duplicates and more than `maxCount`.

## Descriptor-contract gaps to close generically

1. **Row actions.** `ConnectionBindings` has Edit and Remove only. Add an optional generic per-row action list (e.g. `actions?: (item) => Array<{ label; run(item): Promise<SecretReveal | null | void>; confirm?; confirmOptions?: { confirmLabel? }; success? }>`), rendered by `connections-section.tsx` beside Edit/Remove in the same cluster. Run each through `useAction`, with the field names matching its options so the row passes them straight through. A returned reveal reaches the **one** `SecretRevealDialog` owned by `ConnectionsSection` through an `onReveal` callback passed down `FetchedConnections` → `ConnectionRows` → `ConnectionRow` (no dialog per row). Rename `PostCreateReveal` to the neutral `SecretReveal` (it now serves rotation too). GitLab uses actions for **Verify** (toast success / show the server error) and **Rotate webhook secret** (confirm, then reveal the new URL and secret once).
2. **Platform option values (shape settled, A finding 5).** `platformOptions.Component` receives no props, so `run_untagged` has no way into the payload. Shape: `platformOptions.initialValues?: (pool?: RunnerPoolDetail) => unknown`; `Component: ComponentType<{ values: unknown; onChange: (values: unknown) => void }>`; the forms hold `Readonly<Record<platformId, unknown>>` initialised by a new `initialPlatformOptionValues(descriptors, pool?)` in `targets.ts` (beside `initialSelection`), so the two routes stay platform-free. The values travel inside the existing `TargetUpdateValues` object as `platformOptions` for updates, and in a matching values argument (same `platformOptions` key) to `toCreatePayload` / `buildCreatePayload` — no new positional channel. Existing descriptors ignore it. Extend the BRANCHING patterns in `platform-neutral-source.test.ts` with `gitlab` ids and `/runUntagged|gitlabConnectionIds/`.
3. **Post-create reveal.** `PostCreateReveal` has one secret value; the GitLab reveal shows the webhook URL and the secret (both with copy buttons) and says to enable **Job events** under the project/group's Settings → Webhooks. Use `details` for the URL and setting if `SecretRevealDialog` already supports copying inside it; otherwise extend the reveal type generically.

4. **GitHub descriptor assumes two scopes (A round 1, block 1).** `github.tsx`'s `initialSelection` (:244-255), `initialPriorityKeys` (:256), `readOnlySummary` (:257) and `toUpdatePayload` (:225-243) treat every non-`repository` pool as an organization pool, so a `scope: "gitlab"` pool's settings would get a locked GitHub runner-group chip that disables every GitLab option. Fix: branch on `"repository"` and `"organization"` explicitly and return the empty value (`{}`, `[]`, `null`, `{}`) for any other scope. No `gitlab` string in `github.tsx`; output for repository and organization pools unchanged (existing tests stay green).
5. **Connection field shapes (A finding 3).** The GitLab form needs, on `ConnectionField`: (a) `type: "select"` with `options: readonly { value: string; label: string }[]` (scope kind Project/Group); (b) `defaultValue?: string`, seeding the create values (base URL `https://gitlab.com`, scope kind `project`); (c) `validate?: (value: string) => string | null`, like `labelRules.validate`, rendered by `FieldInput` with `aria-invalid` and a `text-danger` line like the labels row (`runner-pools.new.tsx:249-250`), blocking submit. Use a static "Project or group path" label; no value-dependent labels.
6. **Registry order (A finding 4).** `buildCreatePayload` merges with `Object.assign` in registry order and GitHub's create partial always sends `scope`, `installationId`, `repositoryIds`, `runnerGroupId`. Register GitLab **last** in `registry.tsx` with a comment that order matters, and pin it with a Vitest on `buildCreatePayload(platforms, …)` using the real registry: a GitLab-only selection gives `scope: "gitlab"`, a positive `installationId`, `repositoryIds: []`, `runnerGroupId: 1`. Comment in `gitlab.tsx` that `context.options.installations[0]` is read because #138 still requires an installation.
7. **Delivery platform (A finding 7).** Put the id-prefix lookup in `registry.tsx` beside `getPlatform` (e.g. `platformForDeliveryId(id)`), and make the fallback destination text a descriptor value (GitHub "GitHub App"; others none, rendered "—"), so `webhooks.tsx` holds no platform comparison.

Record each gap and its fix in `docs/handoffs/141/decisions.md`.

## Out of scope (A finding 9)

The pool list destination (`runner-pools.index.tsx:81`) and the pool detail Account row (`runner-pools.$poolId.index.tsx:129`) will still show a GitLab pool as its installation's GitHub account. Tracked as #159 in epic #136; #141 does not widen to it.

## Validation (Vitest)

- Base URL: http(s) URL, no query/fragment, trailing slash removed; default `https://gitlab.com`.
- Scope path: non-empty, `segment(/segment)*`, no leading/trailing slash, no `..`, no whitespace.
- Tags: any character but a comma, non-empty after trim; the shared 64-character cap applies.
- Real registry (gap 4): with a `scope: "gitlab"` pool fixture, `initialSelection(platforms, pool)` returns only `gitlab.*` keys, `readOnlySummaries` returns only GitLab, and `buildUpdatePayload` carries no GitHub repository fields; existing GitHub repository/organization outputs are unchanged.
- Real registry order (gap 6): `buildCreatePayload(platforms, …)` with a GitLab-only selection.
- Schema round-trip: see "Pool fields in the shared schema and types".
- Payload partials: create/update for project-only, group-only, mixed project+group, none selected (empty partial that does not override GitHub's), `runUntagged` true/false.
- API functions: each calls the spec path and method with the spec body (mock `api`), including PATCH, 204 verify and rotate.
- Registry: GitLab appears in Add connection's chooser, the Targets picker and Platform options only when targeted (extend the forward-compat tests).

## Walkthrough (headless, against mocked API responses that match the spec)

Add a GitLab connection (reveal shows URL + secret once, copy buttons, Job events note); error states: bad token (400 "rejected the access token"), missing scope (400 naming `create_runner`), duplicate (409); verify success and failure; edit name and replace token; rotate secret (confirm, reveal); delete (and 409 while a pool uses it); pick a GitLab project and a group as pool targets on the new-pool form, see GitHub/Bitbucket kinds excluded; toggle `run_untagged` in Platform options and confirm the POST body carries `scope: "gitlab"`, `gitlabConnectionIds`, `runUntagged`; open a GitLab pool's settings, add a second GitLab connection and save (PATCH/PUT body carries both ids), and the read-only view lists only GitLab; tag validation error on a comma; runner row link for a GitLab runner; a GitLab delivery row shows the GitLab badge.

## Rules

- Keep every GitLab string in `src/features/platform-connections/gitlab.tsx`, the GitLab functions in `platform-connections.functions.ts`, the wire-field files named under Hard limits (`schemas.ts`, `runner-pools.functions.ts`, `operations.functions.ts`), and test fixtures.
- Do not deploy anything. Builds and tests use at most 2 jobs (`CARGO_BUILD_JOBS=2`, `RUST_TEST_THREADS=2`); the Mac is shared.
