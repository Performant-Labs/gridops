# Handoff-T-green: Phase 7 (round 2, after diff gate r1 rework) - Container jobs on the linux pool: runner externals and tool cache (#126)

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals
**Issue:** #126
**Handoff-F reviewed:** docs/handoffs/126/handoff-F.md (round 2)
**Handoff-T-red:** docs/handoffs/126/handoff-T-red.md

## GREEN confirmation
All runs with `CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, non-root, on a Mac with no `/var/lib/gridops-agent`, so the 0555 and "agent host" stand-in tests ran in full.

    cargo test -p gridops-host-agent  -> lib 29, dispatch 8, docker_gitlab 10, docker_runtime 1+1+2, version 1: all passed
    cargo test -p gridops-core --lib  -> 313 passed; 0 failed

F reported lib 28; the 29th is the one test I added this round (below). No other discrepancy with F's handoff.

**Test added (mine):** `the_runner_root_gets_every_entry_of_the_image_dot_entries_included`, in `crates/gridops-host-agent/src/docker/externals_tests.rs`. It pins diff gate W-1: a plain file, a `.dot-file` and a `..double-dot-file` in the image's runner directory all reach the runner root, and `externals` stays the link to the cache. F fixed W-1 with a third glob arm but no test pinned it. No production code touched.

**Spot-check (production edit temporarily made, then reverted):** removing the `"$gridops_image_dir"/..?*` arm made the new test FAIL; with F's code it passes. After the revert, `git status` shows only `externals_tests.rs` modified. (Note: my revert first used a broad `git checkout` that briefly reset the tree; I restored it with `git checkout HEAD -- .`, re-added my test, and re-ran everything above on the final tree.)

## Tier 1 results
| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo +1.98.1 clippy --workspace --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` (workspace on F's code; `-p gridops-host-agent` re-run after my test) | PASS, no warnings |
| `cargo test -p gridops-host-agent` / `-p gridops-core --lib` | PASS (above) |
| Server start / API smoke | N/A: host-agent only; no server, wire or schema change |
| `cargo test --workspace` | Not re-run this round: round 2 touched only `docker.rs` private items and docs. Last round's one failure, `gridops-api` `release_preflight::the_release_preflight_fails_early_without_a_macos_arm64_machine`, is environmental (this host is macOS arm64) and unrelated to #126. |

## Tier 2 results
- **Round-2 changes read against the diff (`5535839..HEAD` in `docker.rs`):** PASS.
  - `remove_tree` is async and runs `remove_tree_blocking` under `spawn_blocking`; `JoinError` becomes `io::Error` and the three callers already handle IO errors. The failed-rename and Prepare-wiring tests exercise the changed lines and pass.
  - `create_shared_folder` rename and `shared_folder_tests` rename are mechanical; no assertion changed.
  - The W-1 glob arm `..?*` sits behind the existing `-e`/`-L` guard, so an unmatched glob is skipped. Now pinned by a test.
- **Coverage per criterion, test quality, error handling, data integrity, migration safety:** unchanged from the previous T-green and still PASS. The suite is proportionate (the new test is one behavior at the cheapest tier, the script under bash with stubs, no overlap with the JIT/persistent tests, which assert the cwd and link).
- **Type safety:** no new casts or untyped values. PASS.
- **Security:** unchanged. PASS.
- **Playwright:** N/A (no UI).
- **Evidence appendix:** `evidence.md` has F's new `spawn_blocking(...).await?` entry; the test relies on nothing else outside the diff.
- **Docs (NV-6):** `docs/fleet-agent.md` now marks the sizes as estimates from the release tarball. PASS.

## Acceptance criteria status
Items 1 to 6 PASS, backed by the tests named in the previous T-green plus the new dot-entries test for the runner-root copy. Item 6's copy-cost figures are estimates from the runner 2.338.2 tarball, and the docs say so.

## Blocking issues
None.

## Advisory notes
- Not exercised by any test, needing a real Docker daemon or fleet host: Start's success path (needs a writable `/var/lib/gridops-agent`), `docker cp -L` and `--mount ...,readonly` on a real engine, and a host bind source resolved through the runner root's `externals` link (the premise of the fix). The brief forbids live fleet verification from the Mac.
- Diff gate W-2 (sweep only after a new fill) is left as brief decision 8 states it; the leak is bounded to one crashed fill per image.

---

# Handoff-T-green: Round 3 (test-only rework, S-1) - Container jobs on the linux pool (#126)

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals
**Issue:** #126
**Finding addressed:** handoff-S.md S-1 (TEST-ONLY). No production (`src/` non-test) change.

## What changed
One test added to `crates/gridops-host-agent/src/docker/externals_tests.rs`:
`a_shared_start_uses_the_id_the_tag_resolves_to_now_and_its_cache`. It calls `shared_runner` (docker.rs:1063) directly against the existing stand-in `docker`, with `SharedPaths` built from temp roots under the stand-in's scratch directory, so it never touches `/var/lib/gridops-agent` and does not depend on that path being unwritable.

1. **Cache present for the id the tag resolves to:** `Ok`; `image_id == "sha256:<HEX>"` (not `TAG`); `args` holds a read-only bind `--mount` of `<cache_root>/<HEX>` at its own path; `folders` is exactly `[work folder, runner root]` and both directories exist; an `image inspect ... TAG` call was made.
2. **Tag moved** (`docker.set("id", "sha256:<HEX_A>")`, the `HEX` cache still present): `Err` with `Runtime, true`; the reason names `sha256:<HEX_A>` and `<cache_root>/<HEX_A>`; an `image inspect ... TAG` call was made; neither the work folder nor the runner root exists.

The F-suggested narrowing of the symlinked-externals test was already done in round 1 (renamed to `a_symlinked_externals_is_followed_and_the_cache_never_holds_a_dangling_link`, requires the `Ok`/follow branch; see decisions.md). Nothing further to change there.

## Mutation checks (production edited temporarily, then restored from a copy; `git diff -- docker.rs` empty afterwards)
- `shared_runner` returns `image.to_owned()` (the tag) as `image_id`: the new test FAILS with `the runner must be created from the resolved id, not "harbor.example/performantlabs/pl-runner:1.70.4"`.
- The cache check moved after the folder creation: the new test FAILS with `a refused Start created its folders` (the existing `a_shared_start_without_a_cache_...` also fails, answering `Configuration, false` on this Mac).

## GREEN / Tier 1 (`CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2`, non-root, no `/var/lib/gridops-agent`)
| Check | Result |
|---|---|
| `cargo test -p gridops-host-agent` | PASS: lib 30 (29 + the new test), dispatch 8, docker_gitlab 10, docker_runtime 1+1+2, version 1 |
| `cargo test -p gridops-core --lib fleet` | PASS: 196 passed, 0 failed |
| `cargo fmt --all -- --check` | PASS |
| `cargo +1.98.1 clippy -p gridops-host-agent --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` | PASS, no warnings |

## Tier 2
- **Coverage:** item 1 / decision 7 ("Start resolves `{{.Id}}` itself and uses that id and its cache") is now pinned by a test, closing S-1. The one line still verified by reading is `image = runner.image_id` in `start` (docker.rs:203), which needs a writable `/var/lib/gridops-agent`; S said not to add a production seam for it this cycle.
- **Test quality:** one behavior (Start's id resolution and cache-before-folders ordering) at the cheapest tier that reaches it (the helper against the stand-in, not the full `call`). It does not duplicate `a_shared_start_without_a_cache_...`, which stays as the end-to-end `call` check of both runner modes; the new test adds what that one cannot tell apart (no cache for the resolved id vs a failed inspect), plus the reason text and the inspect call.
- **Evidence appendix:** the test relies on nothing outside the diff (`shared_runner`, `SharedPaths`, `externals_cache` are all part of #126's diff).

## Blocking issues
None. S-1 is addressed.
