# Handoff-T-red: Phase 4 - GitLab in the shared connections and targets UI (#141)

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5 (head 99a80f8; nothing under `src/` changed)
**Brief / wireframe reviewed:** docs/handoffs/141-brief.md (as amended in 9b9f6ff), docs/handoffs/141/wireframe.html (revision 3), docs/handoffs/141/handoff-D.md, docs/handoffs/141/handoff-A.md (round 3)

## A precondition
Confirmed: A returned **PASS** on the plan in round 3 (`docs/handoffs/141/handoff-A.md`, commit 99a80f8), with three warns. Two of them shape the tests:
- **Warn 1:** the backend test lives in `crates/gridops-api/src/resources/gitlab_tests.rs` and uses its existing harness. No new harness was added.
- **Warn 3:** the registry is allowed to name GitLab in its import and its array entry. The neutral-source test allows exactly those lines, plus comments.

## Baseline
Before any change: `npm ci`, then `npx vitest run` gave **46 files, 461 tests, all passed**. `cargo test -p gridops-api --no-run` built cleanly.

## Tests authored

All Vitest tests use the production registry (`platforms` / `getPlatform("gitlab")`). None of them imports `gitlab.tsx` directly, so no test fails because of a missing module. Where a test needs a new export (`initialPlatformOptionValues`, `gitlabLabelError`), it reads it through a namespace import and asserts it is a function first. A missing export is therefore an assertion failure, not a crash.

### Backend: `crates/gridops-api/src/resources/gitlab_tests.rs` (integration, real routers and a SQLite database)
| Test | Pins | Why this tier |
|---|---|---|
| `the_runners_list_carries_a_gitlab_runners_link_fields_and_nulls_for_a_github_runner` | Brief "Runner link, backend edit". `GET /api/v1/runners` returns `gitlabRunnerId`, `gitlabBaseUrl` and `gitlabScopePath` for a GitLab runner. For a GitHub runner the three keys are **present** and null. | The fields come from the SQL join, so only a request through the real handler proves them. It reuses `seed_connection`, `seed_pool` and `seed_runner`. |

### `tests/gitlab-descriptor.test.ts` (unit, node environment, `fetch` stubbed; 52 tests)
| Group | Pins |
|---|---|
| Connection form (22) | <ul><li>Field order is name, address, Project/Group, path, token.</li><li>The address defaults to `https://gitlab.com` and the kind (a select with `project`/`group`) defaults to `project`.</li><li>The token is a secret field. Its help names `create_runner`, `read_api` and Owner.</li><li>Only the name and the token can be edited.</li><li>Address `validate` (9 cases): http(s) only, no query or fragment, trailing slash allowed.</li><li>Path `validate` (10 cases): `segment(/segment)*`, with no leading or trailing slash, no empty part, no `..` and no whitespace.</li></ul> |
| API calls through the bindings (6) | <ul><li>`list` is `fetch`: GET `/api/v1/platform-connections/gitlab`.</li><li>`create` sends a POST with the exact spec body and removes a trailing slash from the address. A blank address sends the default.</li><li>`postCreate(created)` reveals the 201 `webhookSecret`.</li><li>`update` sends a PATCH with `{name}`, plus `accessToken` only when a token was typed.</li><li>`remove` sends a DELETE.</li></ul> |
| Target kinds (9) | <ul><li>`gitlab.project` and `gitlab.group` are standalone, `many`, with no provider requirement, and have the group headings `GitLab · Project` / `GitLab · Group`.</li><li>On create, options come from `options.gitlabConnections`, filtered by `scopeKind`, with description `<baseUrl>/<scopePath>` and no fetch.</li><li>On create, a kind is available only when its scope has a connection. It is unavailable when the options lack the field (an older server).</li><li>`offered` / `available` hold for create, a GitLab pool, a repository pool and an organization pool.</li><li>On a GitLab pool's settings, `load` fetches every connection, including the unused "Docs site". The keys equal those from `initialSelection`.</li><li>A GitLab target cannot be combined with any GitHub or Bitbucket kind, in either direction (`unavailableReason`). A project and a group can be combined.</li></ul> |
| Pool payloads (10) | <ul><li>`buildCreatePayload(platforms, …, { platformOptions })` for project-only, group-only and mixed selections gives `scope: "gitlab"`, `installationId: 1` (the first installation), `repositoryIds: []`, `runnerGroupId: 1`, the ids in click order, and `runUntagged: false`.</li><li>`runUntagged` comes from the option values.</li><li>With no GitLab target, the GitLab partial adds no defined key.</li><li>A GitLab pool's update sends the full id set and a changed `runUntagged`, and no `repositoryIds`, `priority*`, `runnerGroupId` or `bitbucketConnectionIds` (gap 4).</li><li>Repository and organization pools never send GitLab fields.</li><li>`initialSelection` gives only `gitlab.*` keys, `readOnlySummaries` is `[["gitlab", "Mobile app, Platform group"]]`, and there are no priority keys.</li><li>A pool detail without the GitLab fields still opens.</li></ul> |
| Tags (1) | The descriptor's `labelRules.validate` and `schemas.gitlabLabelError` accept any character but a comma and reject `"a,b"`, `""` and `"   "`. |
| Runner link (4) | <ul><li>The link is `<baseUrl>/<scopePath>/-/runners/<id>`.</li><li>It is null when any of the three fields is null or undefined.</li></ul> |

Why unit: these are pure descriptor and `targets.ts` functions. The only I/O is `api()`, which the stubbed `fetch` covers.

### `tests/gitlab-pool-schema.test.ts` (unit; 13 tests)
- **Fields survive the parse.** On create, `scope`, ids and `runUntagged` are kept. `runUntagged` defaults to false, and a GitHub pool's `gitlabConnectionIds` defaults to `[]`. On update, both fields are kept when sent and left out when not.
- **Tags.** A GitLab pool's tags follow the shared 64-character cap.
- **Create-only scope rule.** These are rejected:
  - a GitLab pool with repositories;
  - a GitLab pool with Bitbucket workspaces;
  - a `gitlab` scope with no connection;
  - a repository pool with GitLab ids;
  - an organization pool with GitLab ids.
- **Bounds, for create and update.** Unique UUIDs up to `maxCount` are accepted and kept. Duplicates and more than `maxCount` are rejected on path `["gitlabConnectionIds"]`, and non-UUIDs are rejected.

Why unit: `createRunnerPoolAction` sends the zod-parsed input, so the schema decides what reaches the server.

### `tests/gitlab-connections.test.ts` (jsdom, rendered `ConnectionsSection` with the real registry; 8 tests)
- **Rows.**
  - Each row shows the GitLab badge, the Project or Group badge, and the detail `gitlab.com/acme/platform/mobile-app` or `git.example.com/platform`.
  - For a manager, the row cluster reads **Edit, Verify, Rotate webhook secret, Remove** in that order (gap 1).
- **Member.** A member sees none of the four buttons.
- **Verify.** It POSTs to `…/{id}/verify` and raises a success toast. On a 400 it shows the server's message verbatim.
- **Rotate.**
  - It asks first (`window.confirm`, with a message naming the connection). If declined, it sends nothing.
  - If confirmed, it POSTs to `…/webhook-secret/rotate` and the single reveal dialog shows the new secret and the URL.
  - Closing the dialog removes the secret from the DOM.
- **Chooser.** GitLab is listed after Bitbucket. The form opens with the address `https://gitlab.com`, a `password` token field and the `SearchableSelect` trigger showing "Project".
- **Create.**
  - The POST carries the exact spec body, and the existing create toast still fires.
  - The reveal holds the secret, the webhook URL and the words "Job events", with at least two copy buttons.
  - On close, the secret and the URL leave the DOM.
- **Inline validation.** An invalid address or path sets `aria-invalid="true"`, and clicking submit sends nothing. Once both are fixed, the POST goes out without the trailing slash (gap 5c).
- **Server error.** "GitLab rejected the access token." is shown verbatim in the dialog, and the typed values are kept.

Why jsdom: the row-action rendering, the `onReveal` plumbing to the one dialog, `defaultValue` seeding and `aria-invalid` on `FieldInput` are all component behaviour. Nothing cheaper reaches them.

### `tests/gitlab-pool-forms.test.ts` (jsdom, rendered routes; 4 tests)
- **New pool: picker.** The picker shows `GitLab · Project` and `GitLab · Group` with the `<baseUrl>/<scopePath>` descriptions, and makes no GitLab GET.
- **New pool: switch and POST.**
  - `Run untagged jobs` (a `role=switch` with that accessible name) does not appear while only GitHub is targeted.
  - It appears in Platform options once GitLab is picked: collapsed by default and off.
  - Turning it on and submitting POSTs `scope: "gitlab"`, `installationId: 1`, `repositoryIds: []`, `runnerGroupId: 1`, `gitlabConnectionIds: [Mobile app, Platform group]` and `runUntagged: true`, after the schema parse.
- **GitLab pool settings.**
  - The GitLab list is fetched.
  - "Docs site" is offered, and no GitHub or Bitbucket option appears.
  - There is no "Runner group 1" chip (gap 4).
  - The switch starts from the pool (on). After adding Docs site and turning the switch off, the PUT carries all three ids, `runUntagged: false` and no `repositoryIds`.
- **GitHub organization pool settings.** There is no GitLab group and no GitLab GET (E6).

Why jsdom: this is the only tier that proves the two routes carry `platformOptions` values to the payload (gap 2) and that the schema keeps the fields.

### `tests/webhook-delivery-platform.test.ts` (jsdom, rendered `/_app/webhooks` page; 4 tests)
- **GitHub rows.** Each carries the GitHub badge. The installation delivery that has no repository still shows "GitHub App": "GitHub" appears twice, once in the badge and once in the destination.
- **GitLab rows.** A `gitlab:<connection>:<key>` delivery and a `gitlab:unmatched:<key>` delivery each show the GitLab badge and no "GitHub" text.
- **Detail sheet.** A GitLab delivery's sheet has the GitLab badge and no "GitHub App" destination (gap 7).

### Changes to existing tests (needed once GitLab is registered)
- **`tests/platform-registry.test.ts`.**
  - The registry is now `["github", "bitbucket", "gitlab"]`, with GitLab last (gap 6).
  - The lookup test checks `getPlatform("gitlab").name === "GitLab"` and `getPlatform("gitlab") === platforms[2]`.
  - The unknown-platform fallback test now uses `azure-devops`.
  - The type test checks `SecretReveal` (the rename in gap 1).
- **`tests/runner-row-platform.test.ts`.** A GitLab runner's badge reads "GitLab", and an unknown id (`azure-devops`) shows its raw id.
- **`tests/connections-page.test.ts`.** `PostCreateReveal` is now `SecretReveal`. The stub serves an empty GitLab list, so registering GitLab does not add a 404 alert to the existing per-platform-loading and empty-state tests.
- **`tests/fixtures/pool-forms.ts`.** `poolFormFetch` serves an empty GitLab list, for the same reason.
- **`tests/platform-neutral-source.test.ts`.**
  - The BRANCHING patterns for the two pool routes gain GitLab ids and `/runUntagged|gitlabConnectionIds/`. The shared-component and runner-row patterns gain `gitlab`. These additions are invariant guards and are **green before and after** the feature.
  - New and RED: the deliveries list imports the registry and has no `"GitHub App"` literal, platform-id comparison or `.startsWith("…")`.
  - New and RED: GitLab code has one home. `gitlab.tsx` must exist. Every file that mentions GitLab must be on the brief's allowlist or be `registry.tsx`, and `registry.tsx` may name GitLab only in its import, its array entry and comments (A warn 3).
- **New fixture `tests/fixtures/gitlab.ts`.** It holds three connections shaped as `GitlabConnection` ("Mobile app", project, on gitlab.com; "Platform group", group, on git.example.com; "Docs site", project, not used by the pool), plus the pool projection, the create options, a GitLab pool and a fetch stub.

## RED confirmation

**Vitest.** Ran `npx vitest run` (full suite):
```
 Test Files  8 failed | 43 passed (51)
      Tests  87 failed | 459 passed (546)
```
Every failure is in a new or updated test, and every existing test passes:

| Failing tests | Count | Failure |
|---|---|---|
| gitlab-descriptor | 52 | |
| gitlab-pool-schema | 13 | |
| gitlab-connections | 8 | |
| webhook-delivery-platform | 4 | |
| gitlab-pool-forms | 4 | |
| platform-neutral-source | 3 | (new) |
| platform-registry | 2 | (updated) |
| runner-row-platform | 1 | (updated) |

Representative failures, each the feature's own assertion:
- `expected [ 'github', 'bitbucket' ] to deeply equal [ 'github', 'bitbucket', 'gitlab' ]`: the registry (gap 6).
- `expected 'gitlab' to be 'GitLab'`: getPlatform falls back.
- `The GitLab connection form has no field labelled "GitLab address"` / `expected [] to deeply equal [ 'Connection name', …(4) ]`: no fields yet.
- `expected 'page' to be 'fetch'`: the fallback list binding.
- `expected undefined to be type of 'function'`: no `create` / `update` / `runnerLink`.
- `No registered target kind "gitlab.project"`: no kinds.
- `expected [ 'github.runner-group' ] to deeply equal [ 'gitlab.group', 'gitlab.project' ]`: a GitLab pool's settings get GitHub's locked runner-group chip today (gap 4).
- `GitLab is registered: expected [ 'github', 'bitbucket' ] to include 'gitlab'`: the precondition of the five "adds nothing" tests. Without it they would pass against the unknown-platform fallback.
- `GitLab declares a label rule: expected undefined to be type of 'function'`.
- `Invalid option: expected one of "repository"|"organization"`: the schema `scope` enum. `expected undefined to deeply equal [ …(3) ]` and `expected true to be false`: the update schema strips the GitLab fields today.
- `expected 'ConnectionsEvery CI platform GridOps …' to contain 'Mobile app'` and `Add connection does not offer GitLab`: no GitLab list, rows or chooser entry.
- `expected 'GitHub · Repositoryocto-org/appocto-o…' to contain 'GitLab · Project'` and `No option with Mobile app in "Targets"`: the pool forms.
- `expected 'ProcessedJob HookGitHub App7h' to contain 'GitLab'`: today a GitLab delivery is labelled "GitHub App" (gap 7).
- `expected 'import { Link, createFileRoute, useNa…' not to match /["']GitHub App["']/` and `gitlab.tsx exists: expected false to be true`: the neutral-source checks.

No test fails on an import, a typo or setup. Fixes made while getting a valid RED:
- Five tests passed before the feature because they would also hold for the unknown-platform fallback or the stripping schema. I gave them the "GitLab is registered" precondition, or an assertion that the parsed value is kept.
- One test failed with a `TypeError` on the missing `initialPlatformOptionValues`. It now asserts that the export exists first.
- Two delivery tests passed before the feature. I merged them into one that counts the GitHub badge beside the "GitHub App" fallback.

**Backend.** Ran `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api the_runners_list_carries`:
```
thread '…the_runners_list_carries_a_gitlab_runners_link_fields_and_nulls_for_a_github_runner' panicked at crates/gridops-api/src/resources/gitlab_tests.rs:2062:5:
assertion `left == right` failed: {"accountLogin":"fixture",…,"id":"runner-gitlab",…,"platform":"gitlab",…}
  left: None
 right: Some(Number(77))
test result: FAILED. 0 passed; 1 failed
```
The harness, the seeding and the request all worked: both runners are listed and `platform` is `gitlab`. The failure is the missing `gitlabRunnerId` key.

**Static checks on the test files:**
- `rustfmt --edition 2024 --check` is clean, and `cargo fmt --check -p gridops-api` exits 0.
- `npx eslint tests --max-warnings=0` is clean.
- `npx tsc --noEmit`, restricted to `tests/`: every remaining error names a type the feature adds and is expected to clear once F's types land:
  - `SecretReveal`;
  - `ConnectionField` `defaultValue` / `options` / `validate` / `"select"`;
  - `initialPlatformOptionValues`;
  - the fourth `values` argument of `buildCreatePayload` / `toCreatePayload`;
  - `TargetUpdateValues.platformOptions`;
  - `gitlabLabelError`;
  - the parsed `gitlabConnectionIds` / `runUntagged`.

  The `src/routes` errors come from the missing generated `routeTree.gen.ts` and predate this change.
- `cargo clippy -p gridops-api --all-targets -- -D warnings` (with `-A clippy::unused_async_trait_impl`): no finding in `gitlab_tests.rs`. The local clippy 0.1.99 reports lints in files this change does not touch (`unused_async_trait_impl` in host-agent and api, and `assert!`-on-empty in `overview_queue.rs` and `tests/fleet_http/*`). Those are outside #141.

## Contract notes for F
These are the DOM and API contracts the tests rely on, all taken from the brief or the wireframe.

**Labels and copy:**
- Field labels: `Connection name`, `GitLab address`, `Project or group path`, `Access token`.
- The Project/Group control is the only `button[aria-haspopup="listbox"]` in the form, with the label of the selected option.
- Button labels: `Verify`, `Rotate webhook secret`.
- The switch is a `role=switch` whose accessible name contains `Run untagged jobs`.
- The reveal closes with the default `I’ve stored it, close` button and contains "Job events".

**Behaviour:**
- Rotation asks through `confirmAction`. The tests stub `window.confirm`, which `confirmAction` falls back to when no `ConfirmHost` is mounted. The confirm message names the connection.
- Inline errors are `aria-invalid="true"` on the input. Submit must send nothing while an inline error is shown: either the submit button is disabled, or the submit handler checks.
- The GitLab create partial must add **no defined key** when nothing GitLab is selected. The frozen #145 payload constants in `tests/pool-target-payloads.test.ts` compare with `toEqual`, so a stray `gitlabConnectionIds: []` or `runUntagged: false` would break them.
- Option keys and `initialSelection` keys must match, and an option's `connectionId` or `nativeId` is the connection's UUID.

**Things the tests leave open:**
- The tests do not fix the shape of the GitLab platform-option value, the row-action type (`actions`) or the name of `platformForDeliveryId`.
- For an unchanged `runUntagged` on settings, the tests accept either sending it or leaving it out. The brief is silent and the wireframe says "only when different". Only a changed value is pinned.

**Possible follow-up for T:**
- `tests/targets-row-forward-compat.test.ts` renders `PlatformOptionsSection` with only `descriptors` and `selection`. If F makes new `values` / `onChange` props on the section required, that test needs a harness update. That is mine to make in Phase 6, as a test-only rework.

## Ready for F
**RED is valid.** F may implement against these tests.

Run the suites with:
- `npx vitest run`
- `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-api the_runners_list_carries`

The test files are staged by explicit path.

Not done in this phase: no satisfiability sketch of the implementation was built. The DOM contracts above were checked against the existing components instead:
- `TargetsRow` loads offered and available kinds on mount;
- the Radix `Switch` exposes `role=switch` and `aria-checked`;
- the `SearchableSelect` trigger has `aria-haspopup="listbox"`;
- `useAction` reports through `toast.promise`.

If F finds a test that cannot be met under the brief, it comes back to T as a test-only rework.
