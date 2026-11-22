# Handoff-F: Phase 6 - Retire a GitLab connection's fleet targets on removal

**Date:** 2026-11-22
**Branch:** claude/gitlab-183-retire-target (on top of bf262aa, T's RED)
**Issue:** #183

## What was done
- `migrations/0048_fleet_target_retire.sql` (new): rebuilds `fleet_ci_targets` in 0047's style (`-- no-transaction`, foreign keys off and on around its own `BEGIN`/`COMMIT`, `_fleet_0048_fk_guard`). It appends `retired_at INTEGER` as the last column, still STRICT. The two GitLab CHECK arms accept `(gitlab_connection_id IS NOT NULL OR retired_at IS NOT NULL)`. The FK stays NO ACTION. `fleet_ci_targets_immutable_identity` keeps its name and message and now aborts every UPDATE except the retire transition: `OLD.retired_at IS NULL AND NEW.retired_at IS NOT NULL`, every identity column `IS OLD`, and the connection stays the same or becomes NULL.
- `crates/gridops-core/src/fleet/targets.rs`:
  - **Key path:** `target_id_for_key` reads only unretired rows (`AND retired_at IS NULL`, `fetch_optional`). A new private `ensure_live_target` returns `Option`. `ensure_target` keeps its public signature and answers `NotFound` for a retired key. `gitlab_connection_target` returns `Ok(None)` for a retired key. The insert still does `ON CONFLICT DO NOTHING`, so nothing is revived or re-created.
  - `list_targets`: `WHERE t.retired_at IS NULL`.
  - `save_pool_fleet` profile-link upsert: `INSERT … SELECT ?,id,1 FROM fleet_ci_targets WHERE id=? AND retired_at IS NULL ON CONFLICT …` (a backstop).
  - `link_pool_target`: `AND t.retired_at IS NULL` (a backstop).
  - The `live_placement!()` `macro_rules!` fragment is now the one list of terminal placement states. `LIVE_WORKLOADS` (pool-keyed, behaviour unchanged) and the new `LIVE_CONNECTION_WORK` (keyed by `p.target_id`, plus `queued` intents, returning pool names) are both built from it with `concat!`.
  - New core function `retire_gitlab_connection_targets(&mut SqliteConnection, connection_id, actor_user_id, now) -> ConnectionTargetsRetirement`. It returns `LiveWork(pool names)` and changes nothing, or `Retired(RetiredTargets)`. For each target it revokes `host_target_grants` and `host_session_target_grants` (`revoked_at`, `revision+1`), revokes `profile_ci_targets` (`enabled=0`, `revoked_at`), runs the one retire UPDATE, and inserts a `fleet.target.retired` event (`TARGET_RETIRED_EVENT`). The event has `target_id` set, no host or placement, the actor user, and camelCase detail: `gitlabConnectionId` plus the per-target counts.
- `crates/gridops-core/src/fleet/service.rs`: the `ensure_target_identity_for_pool` kind lookup adds `AND retired_at IS NULL`, so a retired target is `NotFound`.
- `crates/gridops-core/src/fleet/registry/mod.rs`: the `set_host_grant` existence check adds `AND retired_at IS NULL`, so the grant answers 404.
- `crates/gridops-api/src/resources/gitlab.rs`: `delete_gitlab_connection` now opens `begin_with("BEGIN IMMEDIATE")`. It keeps the pool refusal and then the runner refusal, drops the `fleet_ci_targets` COUNT refusal, then calls the core function. `LiveWork` becomes a 409 that names the pools: "Fleet work of pool gl-live still runs or waits on Acme app. Stop it or let it finish first." The audit event `gitlab.connection_deleted` gains `retiredTargetIds`, `hostGrantsRevoked`, `sessionGrantsRevoked` and `profileLinksRevoked`. The handler has no new fleet SQL.
- `docs/openapi/gridops-v1.json`: the GitLab DELETE description and its 409 description now say what blocks removal and what removal does (it retires targets, revokes grants and links, and keeps history).
- `src/features/platform-connections/gitlab.tsx`: one added sentence in `removeConfirm`: "Its fleet targets are retired and their machine grants revoked; their fleet history is kept."
- `docs/gitlab-runners.md`: a new "Removing a connection" section covering what blocks removal, the order to follow, and what removal does.
- `CHANGELOG.md`: an Upgrading entry for migration 0048 and a Fixed entry for #183.

## Design decisions
- **The retire guard on the key path sits in `target_id_for_key`.** The other option was a GitLab-only check in `ensure_gitlab_target`. One inline predicate on the shared lookup covers both GitLab arms. GitHub targets never retire, so the predicate is a no-op for them. `ensure_target` keeps returning `String`, mapping a retired key to `NotFound`, so its four external callers are untouched. The `Option` form is private, used only by `gitlab_connection_target`, and there is no `NotFound`→`None` error mapping.
- **One trigger, not two.** A allowed either folding "retired is final" into the identity trigger or adding a second trigger in 0040's style. I folded it into one `WHEN NOT (allowed transition)`, because the identity trigger already covers every UPDATE. This keeps the name and message the tests and A pinned, and puts the whole rule in one place. A retire of a non-GitLab row is also allowed by the trigger. That is harmless because nothing does it (GitHub and Bitbucket retirement is out of scope), and narrowing it would add a kind clause the brief did not ask for.
- **Which profile links get revoked:** every link on the target with `revoked_at IS NULL`, including ones `save_pool_fleet` had only disabled (`enabled=0`). Revoking a disabled-but-unrevoked link records the retirement on it too. Test (b) counts 1, and test (a) counts 0 because `delete_pool` removed the links.
- **The live-work 409 names pools, not placements.** An operator acts on pools (scale to zero, or wait), and T's test pins the pool name. The wording "still runs or waits" covers both running placements and queued intents.
- **Event detail keys are camelCase,** matching the `fleet.host.deleted` detail (`registry/delete.rs:196-204`).

## Reuse / extend-vs-new
EXTEND, per the brief and A. The retire is a new function in the existing `fleet::targets` module (A decision §5), beside `delete_pool`. It shares `LIVE_WORKLOADS`'s liveness clause through a `macro_rules!`/`concat!` fragment (A warn 4), which is `delete_pool`'s own composition pattern. The revoke follows `registry/delete.rs:165-171`. The migration follows 0047's rebuild. Every retired-target guard is one inline `retired_at IS NULL` predicate inside the existing SQL (A warn 3). There is no new helper, module or route.

## Architecture notes for A
- **Layers:** a migration, core `fleet::targets` (a new public function and two new public types: `RetiredTargets`, `ConnectionTargetsRetirement`, and the const `TARGET_RETIRED_EVENT`), the inline predicates in `service.rs` and `registry/mod.rs`, and the API handler.
- **Public interface:** `ensure_target`'s signature is unchanged. Behaviour change: a retired key is now `NotFound` instead of returning the retired id. `gitlab_connection_target`'s signature is unchanged; it now also returns `None` for a retired key.
- **Dependency direction:** unchanged (api → core). No new crates.
- **Schema:** `fleet_ci_targets.retired_at`, the widened GitLab CHECK arms, and the trigger's `WHEN` clause.

### Sweep: every production `fleet_ci_targets` / `target_id` read (brief item 3)
| Site | Verdict |
|---|---|
| `targets.rs` `target_id_for_key` (key path for `ensure_target`, `gitlab_connection_target`, `sync_gitlab_targets`, `pool_targets`) | **Filters retired** |
| `targets.rs` `list_targets` (`GET /api/v1/fleet/targets`) | **Filters retired** |
| `targets.rs` `save_pool_fleet` profile-link upsert | **Filters retired** (backstop; its targets come from `pool_targets`) |
| `targets.rs` `link_pool_target` | **Filters retired** (backstop; the GitLab arm also needs a non-NULL connection) |
| `targets.rs` the two GitHub/GitLab `INSERT … ON CONFLICT DO NOTHING` | Keep: a taken key is never re-inserted, and the read after it filters retired |
| `targets.rs` `LIVE_WORKLOADS` / `delete_pool` | Keep: history and cleanup, keyed by pool |
| `targets.rs` `LIVE_CONNECTION_WORK`, `retire_gitlab_connection_targets` | New; only unretired rows still carry the connection |
| `service.rs:743` `ensure_target_identity_for_pool` (submit, admission re-check via `recheck_intent_authorization`, operation access) | **Filters retired** → `NotFound` |
| `service.rs:561`, `:594`, `:635` (fix-sandbox policy) | N/A: `github_repository` only |
| `service.rs:753-810` (per-kind identity COUNTs) | Covered: reached only after the `:743` lookup |
| `service.rs:926` `ensure_target_access` | Covered: submit and operation access also run the identity check; GitLab targets are admin-only |
| `registry/mod.rs:1173` `set_host_grant` existence check | **Filters retired** → 404 |
| `registry/mod.rs:1134` `host_grants` | Covered by revocation (reads `revoked_at IS NULL` only) |
| `registry/mod.rs:2075` `ensure_target_admin` | History read, keep |
| `registry/queries.rs:826` `readable_targets` and the host/backend/placement/event readers (`:885-890`, `:945-954`, `:1029-1092`, `:1257-1259`, `:1638-1641`) | History read, keep. Grant- and link-based arms already require `revoked_at IS NULL`, so they are covered by revocation |
| `admission.rs:131-145` (priority CTE), `:204`, `:333-351` (grant counts), `:622-643` (admit CTE: grant + `profile_ci_targets` join), `:670`, `:707`, `:825` (session grants) | Covered by revocation + `ensure_target_identity_for_pool` (A §6). No queued intent can exist on a target at retire time (the retire refuses), and no new one can be submitted |
| `admission.rs:1376` `target_from_db` (`:451`, `:1239`) | Covered: runs only for an intent that passed the re-check |
| `admission.rs:1098-1111`, `:1212-1218` (placement/event inserts) | Covered: downstream of admission |
| `placement.rs:48`, `failure.rs:187`, `retry.rs:129`, `store.rs:51` | History read, keep |
| `api_tokens.rs:617`, `resources.rs:2348` | N/A: `audit_events.target_id`, not a fleet target |
| `gridops-reconciler/src/main.rs:4158`, `:5517` | N/A: GitHub test-fixture inserts |
| Reconciler fleet pass (`fleet.rs:176` `ensure_target` GitHub; `gitlab_lifecycle.rs:548-552` `gitlab_connection_target`) | Covered by the key-path guard |
| Enrollment, handshake, `authorization.rs` | N/A: no GitLab path (A §6) |

## Deviations from spec / wireframe
None. Where build §1 and §2 offered options, I followed the binding decisions (A warn 1): the FK stays NO ACTION, and the retire is one UPDATE.

## Tier 1 self-check (incl. tests now GREEN)
Constraints used: `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, `-p <crate>` runs, and the shared target dir (`~/.cache/cargo-target/gridops`, from the untracked config).

- `cargo build -p gridops-core -p gridops-api`: Finished.
- `cargo test -p gridops-core`: every binary passes. lib `311 passed; 0 failed` (includes `a_retired_gitlab_target_is_not_valid_for_its_pool_and_is_never_linked`); `fleet_schema` `17 passed; 0 failed` (the four RED schema tests are now GREEN); `gitlab_pool_fleet` `8 passed; 0 failed`.
- `cargo test -p gridops-api --no-fail-fast`:
  - `control_plane_contracts`: `39 passed; 0 failed` (all four of T's RED tests are now GREEN).
  - `fleet_http`: `149 passed; 1 failed`. The failure is `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, the pre-existing macOS-host-only failure T recorded.
  - unit: `130 passed; 1 failed`. The failure is `resources::gitlab_tests::a_connection_a_fleet_target_references_is_refused_with_409_not_500`. It is a stale test; see "Tests that look wrong".
- `cargo test -p gridops-reconciler --no-fail-fast`: `125 passed; 0 failed`.
- `cargo fmt -p gridops-core -p gridops-api -- --check`: clean.
- `cargo +1.98.1 clippy -p gridops-core -p gridops-api -p gridops-reconciler --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl`: Finished, no warnings.
- Frontend (after `npm ci --ignore-scripts`): `npm run build` (vite build + `tsc --noEmit`) passes. `eslint src/features/platform-connections --max-warnings=0` is clean. `vitest run`: `54 files, 624 tests passed`.
- `npm run validate:openapi`: valid, with 67 warnings, the same count as `origin/master`'s file.

## Evidence appendix
`docs/handoffs/183/evidence.md`

## Tests that look wrong (for T)
- **`crates/gridops-api/src/resources/gitlab_tests.rs:1222` `a_connection_a_fleet_target_references_is_refused_with_409_not_500`** pins the behaviour #183 removes. It creates the connection's target with `ensure_target` (nothing uses it) and expects DELETE to be refused with 409. This contradicts:
  - brief item 1 ("A retired GitLab target must no longer block deleting its connection");
  - the brief's test list ("A connection whose target was created only by `list_targets` (never used) can be deleted");
  - T's own extension of `gitlab_connection_list_shows_seeded_connections_and_delete_answers_404_and_409`, which now expects 204 in the same situation.

  T's RED run did not catch it because it passed before the change. It now fails at `gitlab_tests.rs:1186`, inside `assert_delete_refused`. This is a test-only fix with nothing to change in production code. T should rewrite it to the new contract (a 204 and a retired target), or delete it as superseded by the HTTP tests. I did not edit it.

## Known issues
None beyond the stale test above and the pre-existing macOS-only `release_preflight` failure.

## Files changed
- `migrations/0048_fleet_target_retire.sql` (new)
- `crates/gridops-core/src/fleet/targets.rs`
- `crates/gridops-core/src/fleet/service.rs`
- `crates/gridops-core/src/fleet/registry/mod.rs`
- `crates/gridops-api/src/resources/gitlab.rs`
- `docs/openapi/gridops-v1.json`
- `src/features/platform-connections/gitlab.tsx`
- `docs/gitlab-runners.md`
- `CHANGELOG.md`
