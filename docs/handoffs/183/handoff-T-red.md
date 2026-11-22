# Handoff-T-red: Phase 4 - Retire a GitLab connection's fleet targets on removal (#183)

**Date:** 2026-11-22
**Branch:** claude/gitlab-183-retire-target (head 37fdd0b; the only changes are test files)
**Brief / wireframe reviewed:** docs/handoffs/183-brief.md (amended aef7a18), docs/handoffs/183/handoff-A.md (R2), docs/handoffs/183/decisions.md. Wireframe: N/A (copy change only).

## A precondition
Confirmed: A returned **PASS** on the plan (Phase 3, R2, 0 block / 4 warn). Where build §1/§2 and "Architecture decisions" differ, the tests follow the binding decisions (A warn 1). So the FK stays NO ACTION, and the retire is one UPDATE that sets `retired_at` and nulls `gitlab_connection_id`.

## Tests authored

All tests call only APIs that already exist, so every test binary compiles and each RED is a runtime assertion failure. None is a build error.

### HTTP tier: `crates/gridops-api/tests/control_plane_contracts.rs`, next to the DELETE test at :2345 (decision §7)
These run the real API binary, and each DELETE response is checked against the OpenAPI schema. HTTP is the cheapest tier that covers the handler's refusal order, its transaction, the audit row and the fleet routes together.

| Test | Pins |
|---|---|
| `removing_a_gitlab_connection_retires_its_fleet_target_and_revokes_its_grant` | **(a), the issue's case.** The GitLab pool is saved on fleet machines. The grant is set through `POST /api/v1/fleet/hosts/{id}/grants/{target}` with every flag off, which is prod's shape. The pool is then deleted (`delete_pool`). After that, DELETE answers 204 and: the target row stays, retired, with a NULL connection; the grant is revoked with revision 1→2; there is exactly one `fleet.target.retired` event (target set, host and placement NULL, actor `user-1`, detail mentions the connection id); audit `gitlab.connection_deleted` has `retiredTargetIds=[T]`, `hostGrantsRevoked=1`, `sessionGrantsRevoked=0`, `profileLinksRevoked=0` (decision §5); the connection and its secrets are gone; `GET /api/v1/fleet/targets` no longer lists T; a grant POST on T answers **404 `not_found`** and changes nothing (decision §4). |
| `removing_a_gitlab_connection_keeps_its_fleet_history_and_revokes_its_platform_link` | **(b), the history case.** The pool ran a finished workload: an `admitted` intent, a `cleaned` placement, and a `fleet_events` row on the placement. The pool then switches to another connection, the way the pool update replaces memberships (`resources.rs:1883-1895`), so the profile link stays enabled. DELETE answers 204. The intent, placement and event still reference T, and T is retired. The profile link is `enabled=0` with `revoked_at` set. The grant is revoked with revision 2. The other connection's target is untouched. Audit `hostGrantsRevoked=1`, `profileLinksRevoked=1`. |
| `a_gitlab_connection_is_not_removed_while_anything_still_runs_through_it` | **The refusals and their order (item 2).** Each step expects a 409 that matches the schema, and the DB snapshot (connections, target→connection, grants, links, retired events, removal audits, secrets) must be identical to before. The steps run in this order: (1) a pool uses the connection, and the message names `gl-member` (unchanged); (2) a live runner is registered through it, and the message contains `1 runner registered through Acme app` (unchanged); (3) a **queued** intent of another pool on T; (4) a **running** placement; (5) a **cleaned** placement with a **committed** allocation. Steps 3-5 must each name the workload's pool `gl-live`. (6) Once the allocation is released, the leftover `admitted` intent counts as history and DELETE answers 204. |
| `gitlab_connection_list_shows_seeded_connections_and_delete_answers_404_and_409` (existing, extended) | **The never-used case.** A `GET /api/v1/fleet/targets` runs first, which creates the connection's target, and then the final DELETE must still answer 204. The test asserts that the list created the target. |

### Core tier: `crates/gridops-core/tests/gitlab_pool_fleet.rs` (integration, public `fleet::targets` API)
| Test | Pins |
|---|---|
| `a_retired_gitlab_target_is_never_revived_offered_or_relinked` | **Key-path, list and save guards (decision §6).** T is retired, its link is revoked, and the connection row is still present under the same id, as after a restore. Then: `gitlab_connection_target` returns `None` and inserts no row; `list_targets` leaves T out and still lists the group target; `pool_targets` returns only the group target; saving `save_pool_fleet` again leaves T's link at `(0, revoked)` and links only the group; T stays retired. |

### Core unit tier: `crates/gridops-core/src/fleet/admission_tests.rs` (in-crate, because `ensure_target_identity_for_pool` is `pub(crate)`)
| Test | Pins |
|---|---|
| `a_retired_gitlab_target_is_not_valid_for_its_pool_and_is_never_linked` | **Admission and link guards (decision §6, A warn 3).** `ensure_target_identity_for_pool` returns **`Err(NotFound)`** for a retired target. Submit, the admission re-checks and operation access all go through this check, so it covers "never admitted or placed". Without a guard, the GitLab arm would answer `Forbidden` (a NULL-connection join), so `NotFound` proves the choke-point predicate exists. The pool's other target stays valid. `link_pool_target` does not link the retired target. |

### Schema tier: `crates/gridops-core/tests/fleet_schema.rs`
| Test | Pins |
|---|---|
| `FLEET_ADDED_COLUMNS` gains `("retired_at","INTEGER",None)`, so the existing `gitlab_upgrade_from_0046_keeps_the_fleet_graph_and_every_reference` now requires it | The 0046→latest upgrade appends `retired_at` last, nullable, with no default, and NULL on every existing row (decision §1). |
| `the_0048_rebuild_keeps_every_target_and_reference` (new) | 0047→latest on a database that already holds a GitLab target with a grant, a profile link and an event. The test checks that: every record is identical; every table's FKs are unchanged, including `fleet_ci_targets`' four NO ACTION FKs; `retired_at` is appended and NULL; every other schema object's definition is unchanged; the trigger is back under its name; the table is STRICT; `foreign_key_check` is empty; integrity is ok; and `foreign_keys` is ON on every pooled connection. |
| `retiring_a_gitlab_target_frees_its_connection_and_is_final` (new) | **The retire transition (decision §1).** These are refused: nulling the connection without retiring, and a retire combined with a change to the project id, the key or the kind. The retire UPDATE itself succeeds, and the retired row keeps its identity. The connection then deletes while the grant, link and event references remain. An unretired target still blocks its own connection's delete with `FOREIGN KEY constraint failed`. After the retire, these are refused and leave the row unchanged: un-retiring, re-retiring, attaching a connection, changing the key, changing the project id. |
| `the_0048_foreign_key_guard_fails_on_a_dangling_reference` (new; the existing 0047 test now shares the helper `assert_foreign_key_guard`) | 0048 is exactly one `migrations/0048_*.sql`, and it carries a CREATE/INSERT/DROP `*_fk_guard` that refuses a dangling reference and accepts a clean DB. This follows 0047's style and A's `_fleet_0048_fk_guard`. |

**Not covered by tests, by design:** the OpenAPI DELETE description, the confirmation copy in `src/features/platform-connections/gitlab.tsx:378`, `docs/gitlab-runners.md` and the CHANGELOG. These are copy and documentation only, there is no existing test of them, and a wording test would be brittle. S verifies them. The reconciler's fleet pass is covered through the choke points (A decision §6), so it gets no separate test.

## RED confirmation

Run: `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p gridops-core` and `... -p gridops-api` (the shared target dir comes from the untracked `.cargo/config.toml`).

- **gridops-core:** `fleet_schema` 13 pass, **4 fail**; `gitlab_pool_fleet` 7 pass, **1 fail**; lib 310 pass, **1 fail**. Every other test binary passes.
- **gridops-api:** `control_plane_contracts` 35 pass, **4 fail**; `fleet_http` 149 pass, 1 fail (pre-existing and environmental; see the note below).

Exact failures:
```
gitlab_upgrade_from_0046_keeps_the_fleet_graph_and_every_reference  fleet_schema.rs:363
  assertion failed: fleet_ci_targets: added columns
  left:  [gitlab_connection_id, gitlab_project_id, gitlab_group_id]
  right: [..., ("retired_at", "INTEGER")]
the_0048_rebuild_keeps_every_target_and_reference  fleet_schema.rs:363
  fleet_ci_targets: added columns  left: []  right: [("retired_at", "INTEGER")]
retiring_a_gitlab_target_frees_its_connection_and_is_final
  Error: 0048 allows the retire: retired_at set and the connection dropped in one UPDATE
    0: error returned from database: (code: 1) no such column: retired_at
the_0048_foreign_key_guard_fails_on_a_dangling_reference
  Error: expected exactly one migration 0048 in .../migrations, found []
a_retired_gitlab_target_is_never_revived_offered_or_relinked
  Error: 0048 lets a GitLab target retire / no such column: retired_at
fleet::admission::admission_tests::a_retired_gitlab_target_is_not_valid_for_its_pool_and_is_never_linked
  0048 lets a GitLab target retire: ... no such column: retired_at
removing_a_gitlab_connection_retires_its_fleet_target_and_revokes_its_grant  control_plane_contracts.rs:2702
  left: 409 right: 204  {"error":"gridops-e2e (prod check) is a CI target of the fleet (1 target). Remove it from the fleet first."}
removing_a_gitlab_connection_keeps_its_fleet_history_and_revokes_its_platform_link  :2822
  left: 409 right: 204  {"error":"Acme app is a CI target of the fleet (1 target). Remove it from the fleet first."}
gitlab_connection_list_shows_seeded_connections_and_delete_answers_404_and_409  :2453
  left: 409 right: 204  {"error":"Acme app is a CI target of the fleet (1 target). Remove it from the fleet first."}
a_gitlab_connection_is_not_removed_while_anything_still_runs_through_it  :2952
  then the live runner: Acme app is a CI target of the fleet (1 target). Remove it from the fleet first.
```

**Why each RED is for the right reason:**
- Every HTTP failure is the refusal #183 removes ("is a CI target of the fleet").
- Every schema and core failure is the missing 0048 retire schema: no `retired_at`, no retire transition, no 0048 file.
- None of them is a typo, a missing import, or a broken seed. Step 1 of the refusal test (the unchanged pool refusal) passes before failing at step 2, and every setup statement before each failure ran.

**Plumbing checked beyond the first failure.** The HTTP tests stop at their first DELETE before F's code exists, so I checked their later setup with a temporary test-only probe. I deleted the probe afterwards, and it is not in the diff. The probe confirmed all of the following:
- The workload, allocation and event seeds insert under every trigger and FK.
- A target-keyed liveness query reads live for both a running placement and an unreleased allocation, and reads drained once the allocation is released with proof.
- The grant route answers 200 on a live target and 404 `{"code":"not_found"}` on an unknown one.
- `GET /api/v1/fleet/targets` lists a GitLab target.
- Calling `delete_pool` from the test process works against the API's database file.

I also replayed every accept and refuse statement of `retiring_a_gitlab_target_frees_its_connection_and_is_final` in `sqlite3`, against A's binding CHECK and trigger design (decision §1). It matches: 4 refusals before the retire, the retire succeeds, the connection delete succeeds, an unretired target still fails the FK, and 6 refusals after the retire.

**Lint and format:** `cargo fmt -p gridops-core -p gridops-api -- --check` is clean. `cargo +1.98.1 clippy -p gridops-core -p gridops-api --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (the CI toolchain and flags) is clean.

**Pre-existing, unrelated failure (advisory):** `fleet_http::release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine` fails on this Mac. The fixture's "Linux" machine reports the OS and architecture the test agent was compiled for (`release_preflight.rs:63-65`), so on macOS ARM64 it is a Mac and the check passes. It passes on Linux CI. My diff touches neither `fleet_http` nor any production code.

## Ready for F
**RED is valid. F may implement against these tests.** Notes for F:
1. Removing the connection must produce the exact schema and audit contract above: `retired_at` set in the same UPDATE that nulls the connection; the audit keys `retiredTargetIds`, `hostGrantsRevoked`, `sessionGrantsRevoked` and `profileLinksRevoked`; the event kind `fleet.target.retired` with no host; and `revision+1` on revoked grants.
2. The live-work 409 must contain the **name of the pool** whose workload is still running (`gl-live` in the test). The test asserts no other wording.
3. `ensure_target_identity_for_pool` must answer `NotFound` for a retired target (A warn 3), not fall through to `Forbidden`.
4. The 0048 migration must be one `migrations/0048_*.sql` file that contains a `*_fk_guard` temp-table guard, as 0047 does.
5. The refusal order is pinned as **pool → live runner → live work**. Today's two refusals keep their order, and the new one comes after them (the brief: "Keep today's refusals and their order … Add a refusal …"). Step 2 of the refusal test runs with a queued intent already present and expects the runner message. So the handler's runner check must run before the core function's liveness refusal, which is the natural place since that function also performs the retire.
