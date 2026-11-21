# Handoff-A: Phase 4 - GitLab in the shared connections and targets UI (#141)  (up-front plan review, round 3)

**Date:** 2026-11-20
**Branch:** claude/gitlab-141-ui-3ae7f5 (head 3571a74; `git diff 0086aa9 HEAD --stat -- src tests crates` is empty, so no code exists yet)
**Brief reviewed:** docs/handoffs/141-brief.md (amended in 9b9f6ff after round 2)   **Reuse map:** there is no `docs/handoffs/141/survey.md`. The analogue is the Bitbucket descriptor and the #145 descriptor contract (`docs/handoffs/145/survey.md`, plus the brief's Target kinds section). Recommendation: EXTEND.   **Wireframe:** docs/handoffs/141/wireframe.html, revision 3, with docs/handoffs/141/handoff-D.md (approval line not filled in)
**Verdict:** PASS

## Summary
PASS. The amendment in 9b9f6ff fixes the round-2 block:
- On create, the GitLab target options come from the pool options.
- On a GitLab pool's settings, `load` fetches them with `getGitlabConnections(signal)`. This uses the existing lazy-load seam (`github.tsx:93-105`).
- `offered` and `available` are scoped to create or to a GitLab pool.
- All of this stays in `gitlab.tsx`, with no `TargetContext` or route change. Wireframe E5, E5b and E6 match.

All five round-2 warns are taken up.

The one new plan element is the narrow backend edit to the runners list. It follows the existing pattern: the list already joins display-only fields, and the GitHub descriptor already builds its runner link from raw fields. No parallel branch touches that function.

There are three new warns:
- the test location for the backend edit;
- a runner-row sibling of #159 that is not recorded anywhere;
- an allowlist wording conflict for `registry.tsx`.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | Runner link, backend edit: "Test: an API test (beside #138's GitLab tests in `crates/gridops-api/src/resources/gitlab_tests.rs`, **or the existing runners-list test**)" | pattern consistency (test placement) / scope | **There is no existing runners-list test.** The only test that touches `GET /api/v1/runners` is `tests/fleet_http/token_access.rs:1072`, which only checks that the route refuses bearer tokens. The `mod tests` in `resources.rs` (`:6227`) holds pure-function unit tests and no router harness. If F reads "or" as a choice, F may build a second router/database harness. That would be a parallel test path.<br>**What is consistent:** the edit itself.<br>- The query already LEFT JOINs display-only fields: `repo.full_name AS repository`, `account_login`, and `machine_json`.<br>- The alias `gc` is free.<br>- `runners.gitlab_connection_id`, `runners.gitlab_runner_id`, `gitlab_connections.base_url` and `gitlab_connections.scope_path` all exist (`migrations/0047_gitlab.sql`).<br>- `/api/v1/runners` is not in `docs/openapi/gridops-v1.json`, so no OpenAPI change is needed, which fits the hard limit.<br>- `github.tsx:205-207` builds its link from raw fields too, so raw `gitlabBaseUrl`/`gitlabScopePath` (rather than a server-built URL) matches.<br>- No parallel branch or worktree (#139, #140, #142, #147, #148, the #138 follow-up) changes the `runners` function or `gitlab_tests.rs`. | Put the test in `crates/gridops-api/src/resources/gitlab_tests.rs` and drop the "or" alternative. That file already drives the real routers and has `seed_connection`, `seed_pool` and `seed_runner` with `gitlab_connection_id` and `gitlab_runner_id` (`:241-306`). Seed a GitHub runner beside the GitLab one for the all-null case. Do not add a new harness. |
| 2 | warn | Wireframe G note: "Pool and Repository rows are unchanged; the repository column shows the scope path when the server provides one, else is blank as today" | scope (unrecorded sibling of #159) / dependency direction | **For a GitLab runner, the Repository row and column will not be blank.** `runner-row.tsx:46, :109, :127` render `runner.repository ?? runner.accountLogin`. For a GitLab runner, `repository` is null (the LEFT JOIN on `target_repository_id`). `accountLogin` is the pool installation's GitHub login, which #138 still requires. So a GitLab runner is shown under a GitHub account. This is the same defect class as #159, but #159 names only the pool list and the pool detail, not the runner row. The obvious local fixes would be drift:<br>- A `runner.platform` branch in `runner-row.tsx` (the neutral-source test forbids it).<br>- Emitting the scope path as `repository` in the backend (the brief says "No other filter or column changes"). | Do not fix it in #141. O should add the runner row (`runner-row.tsx:46, :109, :127`) to #159, so that one descriptor hook covers the pool list, the pool detail and the runner row. If D revises the wireframe, correct the G note. If O wants it fixed in #141 instead, that is a brief change: add a generic descriptor hook (for example `runnerDestination?(runner)`) and add `runner-row.tsx` to the allowlist for it. |
| 3 | warn | Hard limits: shared files "may change only … (no `gitlab` string or GitLab branch in them): … `registry.tsx` …". Gap 6: "Register GitLab **last** in `registry.tsx`". Deliveries: "an id prefix matching a registered descriptor id, else GitHub". | naming / test shape | **Read literally, the allowlist line forbids the registration that gap 6 requires.** Registering GitLab means `import { gitlabPlatform } from "./gitlab"` plus one array entry. The registry's own header (`registry.tsx:1-4`) and the contract header (`types.ts:1-6`) say a new platform is "one more descriptor module plus one line in `registry.tsx`", so that is the designed seam. A T who pins "no `gitlab` in shared files" over `registry.tsx` would block it. The "else GitHub" fallback in `platformForDeliveryId` invites a quoted `"github"` id in a file that otherwise compares only ids it is given. | Read the rule for `registry.tsx` as: the GitLab import and array entry are allowed, and nothing else in the file names a platform. In `platformForDeliveryId`, fall back by reference to the already-imported `githubPlatform` (or `getPlatform` with a commented reason), not a new string comparison. Any neutral-source assertion over `registry.tsx` must allow the registration lines. |

**Round-2 findings, now resolved:**
- **Block 1 (settings option source).** The brief's Target kinds section now spells out `offered` (create, or `pool.scope === "gitlab"`), `available` (create: the kind's scope has a connection; update: a GitLab pool) and `load` (create: pool options; update: `getGitlabConnections(signal)`). Both paths filter by `scopeKind` and use keys equal to `initialSelection`'s. There is no `TargetContext` or route change. The Vitest list gains the matching cases.
  - Wireframe E5 (an unused "Docs site" offered), E5b (loading and per-kind error/Retry) and E6 (no GitLab group on a GitHub or Bitbucket pool) match.
  - Checked against `github.tsx:87-105, :118`.
  - `GET /api/v1/platform-connections/gitlab` (`resources/gitlab.rs:84-101`) takes any `AuthUser` and lists every connection. This confirms the round-2 assumption that the settings fetch works for any signed-in manager.
- **Warn 2 (split refinement):** taken up. The brief now has a create-only scope rule and a bounds rule shared by both schemas, mirroring `validate_gitlab_targets` / `validate_gitlab_connections`.
- **Warn 3 (select control):** `SearchableSelect` with `searchable={false}` is named in gap 5a and in wireframe B note 1.
- **Warn 4 (badge and neutral-source test):** gap 7 now names the exported `PlatformBadge` and the runner-row-style assertion. The existing "GitHub App" copy (`webhooks.tsx:82-83`) stays.
- **Warn 5 (wireframe extras):** the create toast stays (brief and B note 4), and the H2 Platform row is dropped.
- **Warn 6 (baseline):** the pool wire now points at master (`0086aa9`), and the branch has master merged (7962134).

**Checked and consistent (no finding):**
- **Optional runner fields.** `gitlabRunnerId`, `gitlabBaseUrl` and `gitlabScopePath` are optional on `Runner`, while `bitbucketRunnerUuid` is required-nullable (`operations.functions.ts:50`). Now that the backend always emits them, either shape works. Optional matches the brief's "optional pool fields" choice and avoids churn in every `Runner` fixture. No change asked.
- **Delivery ids.** The id prefix agrees with #140's settled brief: `gitlab:<connection_id>:<key>`, and `gitlab:unmatched:<key>` for rejected rows. A `${descriptor.id}:` prefix match covers both. GitHub delivery GUIDs never carry that prefix.
- **Coexistence.** These worktrees change none of #141's shared frontend files, the `runners` function in `resources.rs`, or `gitlab_tests.rs`:
  - #139 (core and reconciler only, uncommitted);
  - #140 (brief only);
  - #148 (a merge of master in progress; nothing in `resources.rs` beyond master);
  - the #138 follow-up (`main.rs`, `resources/gitlab.rs`, `state.rs`).

## Notes for O
Not required (PASS). Recommended while the brief is open:
1. **(Finding 1)** In "Runner link", replace "or the existing runners-list test" with "in `crates/gridops-api/src/resources/gitlab_tests.rs`, using its `seed_connection` / `seed_pool` / `seed_runner` helpers".
2. **(Finding 2)** Comment on #159 (or edit its body) to add the runner row's Repository row and column (`src/features/runners/runner-row.tsx:46, :109, :127`, `runner.repository ?? runner.accountLogin`) as a third view that shows a GitLab runner under its installation's GitHub login. Add one line to the brief's "Out of scope" saying so. If D revises the wireframe, correct the G note.
3. **(Finding 3)** In Hard limits, add after `registry.tsx`: "(the GitLab import and its array entry are the allowed registration; nothing else in the file names a platform)".

## Patterns referenced
- `crates/gridops-api/src/resources.rs:1916-1998` (the runners list query and JSON, with its existing joined display fields) and `crates/gridops-api/src/resources/gitlab_tests.rs:241-306` (the router harness and GitLab runner seeding).
- `src/features/platform-connections/github.tsx:87-105, :205-207` (the lazy `load` seam, the scope-scoped `offered`, and a runner link built from raw runner fields).
- `src/features/platform-connections/registry.tsx` and `types.ts` headers (a new platform is one descriptor module plus one registry entry).
- `src/features/runners/runner-row.tsx:46, :83-94, :109, :127` (the descriptor `runnerLink`, and the `repository ?? accountLogin` fallback).
- `tests/platform-neutral-source.test.ts` (the shared-file assertions the new generic code must extend).
