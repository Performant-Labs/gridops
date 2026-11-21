# [GitLab: pool list and pool detail show a GitLab pool as a GitHub account](https://github.com/Performant-Labs/gridops/issues/159)

Part of epic #136. Found by #141's architecture review (finding 9). Everything it depends on is on master: #145 (platform registry), #138 (GitLab pool API), #141 (GitLab descriptor).

## Need

Two views assume every pool belongs to a GitHub account:

- the pool list's destination column, `PoolRow` in `src/routes/_app/runner-pools.index.tsx` (~line 81 and the `Avatar`/`githubAvatar` at ~line 98);
- the pool detail's **Destination** group in `src/routes/_app/runner-pools.$poolId.index.tsx` (~lines 127-133): the Scope row, the Account row with a GitHub avatar, and the `pool.scope === "repository"` branch between `RepositoryList` and the Runner group row.

A `scope: "gitlab"` pool still stores an installation id (#138 requires one on every pool), so today both views show it as that installation's GitHub account, and the detail also shows Scope "Gitlab" and a meaningless "Runner group #1".

## Scope

### Frontend: the destination comes from the registry

- Both views work out the pool's destination from the platform registry in `src/features/platform-connections/`. **The routes compare no platform**: no platform or kind id string, no `pool.scope`, no `githubAvatar`, no platform-named field (`gitlabConnections`, `bitbucketConnections`, `repositoryCount`, …).
- `readOnlySummary` does not fit as is: it takes a `RunnerPoolDetail`, and the list row has the lighter `RunnerPool` (`src/features/operations/operations.functions.ts`). Add descriptor hooks instead (names are the implementer's/architect's call; this is the intended shape):
  - a way for the registry to find **the platform that owns a pool's scope** (e.g. an `ownsPool`/`poolScopes` declaration per descriptor, resolved by a registry function such as `platformForPool(pool)`, in the style of `platformForDeliveryId`). GitHub owns `repository` and `organization`; GitLab owns `gitlab`; Bitbucket owns no scope (its pools are GitHub-scoped). An unknown scope resolves to a safe fallback that never throws and shows no GitHub avatar;
  - a **list destination** hook on the owning descriptor: what the list row's destination cell shows for a `RunnerPool` (text, its title/tooltip, and an optional leading avatar or icon);
  - a **detail destination** hook (e.g. a component taking the `RunnerPoolDetail`) that renders the rows of the Destination group. GitHub's keeps today's rows exactly (Scope, Account with the GitHub avatar, and either the repository list with its "Show N more" toggle or the Runner group row); `RepositoryList` moves with it.
- **GitHub and Bitbucket pools look exactly as they do now**, in both views: same text, same avatar, same rows, same order. A Bitbucket pool is GitHub-scoped, so it keeps its GitHub account destination.
- **A GitLab pool shows its GitLab connections**:
  - list: the connection names (e.g. `Acme / web` or `2 connections` when more than one, with all names in the title), and the GitLab platform icon instead of a GitHub avatar;
  - detail: Scope "GitLab", then the pool's connections (name, and `baseUrl/scopePath` as detail, from `RunnerPoolDetail.gitlabConnections`); no Account row with a GitHub avatar, no Runner group row.
  - A GitLab pool whose connection list is empty or absent (an older server) shows a neutral fallback (e.g. "GitLab" or a dash), never the installation's GitHub account.
- GitLab-specific code stays in `gitlab.tsx` and GitHub-specific code in `github.tsx`; the "GitLab code has one home (#141)" test must keep passing (the routes must not name GitLab).

### API: the pool list carries GitLab connections

- The list endpoint `runner_pools` in `crates/gridops-api/src/resources.rs` (~line 1292) returns no GitLab fields, so the list row cannot show a GitLab pool's connections. Add `gitlabConnections` to each list item: an array of `{ id, name, baseUrl, scopeKind, scopePath }` (the same shape `pool_gitlab_connections` / `pool_connection_json` in `crates/gridops-api/src/resources/gitlab.rs` already return for the detail), empty for a pool with none.
- Load them for the whole page with **one** query over the page's pool ids (not one query per pool), ordered as the detail orders them (`membership.created_at, connection.id`). Keep the helper in `resources/gitlab.rs`.
- Add `gitlabConnections?: GitlabPoolConnection[]` to the `RunnerPool` type (optional: absent from an older server).
- No migration, no change to any other endpoint.

## Constraints

- No deploy. Nothing GitLab ships to Mars until epic #136 is done.
- Rust builds share this Mac with other sessions: keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` (set in `.claude/settings.local.json`).
- Do not change the pool forms, the Targets picker, or `readOnlySummary`'s existing behaviour (callers elsewhere may rely on it).

## Done when

- `tests/platform-neutral-source.test.ts` covers both routes: neither `runner-pools.index.tsx` nor `runner-pools.$poolId.index.tsx` contains a platform or kind id string, `pool.scope`, a scope comparison, `githubAvatar`, or a platform-named pool field, and both import from `~/features/platform-connections/registry`.
- Unit tests (vitest) show, for a GitHub repository pool (one and several repositories), a GitHub organization pool, a Bitbucket (GitHub-scoped, with workspaces) pool, a GitLab pool with one and with two connections, a GitLab pool with no connections, and an unknown scope:
  - the registry resolves the owning platform as described;
  - the list destination is exactly today's for GitHub and Bitbucket pools (`N repositories` / the repository full name / the account login, with the GitHub avatar), and the connection names with no GitHub avatar for GitLab;
  - the detail destination renders today's rows for GitHub and Bitbucket pools, and Scope "GitLab" plus the connection rows (and no Account or Runner group row) for GitLab.
- A Rust API test shows the pool list returns a GitLab pool's `gitlabConnections` (names and order as the detail) and `[]` for a GitHub pool, from a single batched query.
- The headless UI walkthrough opens `/runner-pools` and one pool's detail page for a GitHub pool and for a GitLab pool and confirms the above.
- `npm run lint`, `npm run typecheck` (or the repo's equivalents), `npm test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test -p gridops-api` pass.
