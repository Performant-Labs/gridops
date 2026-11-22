# Handoff-S: Phase 8 - Container jobs on the linux pool: runner externals and tool cache at host-valid paths (#126)

**Date:** 2026-11-22
**Branch:** claude/fix-126-externals (head d4e38c8; base 9e357e0)
**Issue:** #126
**Brief:** docs/handoffs/126-brief.md
**Handoff-T reviewed:** docs/handoffs/126/handoff-T-green.md (round 2), with handoff-T-red.md
**Handoff-A reviewed:** docs/handoffs/126/handoff-A.md (round 3, PASS), docs/handoffs/126/handoff-A-dup.md (PASS)
**Handoff-F reviewed:** docs/handoffs/126/handoff-F.md (round 2, with round 1 below it)
**Also read:** docs/handoffs/126/decisions.md, docs/handoffs/126/evidence.md, docs/handoffs/126-diff-result-r1.md, docs/handoffs/126-diff-result-r2.md
**Operator-facing report:** N/A (no visual surface)

## A precondition
Confirmed. `handoff-A.md` (round 3) is PASS with 2 warns, both done in F's round 2. `handoff-A-dup.md` (anti-duplication gate) is PASS.

## T precondition
Confirmed. `handoff-T-green.md` reports "Blocking issues: None". The code tree is identical to the one T ran: `git diff af5f052..HEAD` (T-green's commit to HEAD) touches only `docs/handoffs/`. I did not re-run the Rust suites; T's results (host-agent lib 29, dispatch 8, docker_gitlab 10, docker_runtime 1+1+2, version 1; core lib 313; fmt and clippy clean) stand for this tree.

## Visual-diff-tool precondition
N/A. This is a Rust host-agent and core change with no rendered output. No screenshots, no HTML report.

## Preview / spec sanity check
One defect in the source of truth, not worth a hold. Brief decision 3 names `docker image inspect -f '{{.Id}} {{.Config.WorkingDir}}'`, which is a template error on Docker 29 for an image with no `WorkingDir`. F used `{{.Id}} {{json .Config}}` and documented it as a deviation; it gives the behaviour decision 3 asks for (no working directory gives `Configuration, false`). F's implementation faithfully matches the intent, so this is not REWORK material and not an ADVISORY-HOLD.

## Spec compliance
Checked by reading the full diff (`master...HEAD`): `docker.rs`, `docker/externals_tests.rs`, `work_folder.rs`, `fleet/mod.rs`, `tests/docker_gitlab.rs`, `tests/docker_runtime.rs`, `docs/fleet-agent.md`, `CHANGELOG.md`; plus `dispatch.rs`, `provisioning.rs` (forwarder) and `deploy/systemd/gridops-host-agent.service` for the wiring the change depends on.

| Brief requirement | Compliant | Evidence |
|---|---|---|
| 1. Externals at the same absolute path on host and runner; runner runs from a root whose `externals` is that path; JIT and persistent both | YES | `SHARED_RUNNER_ROOT_SETUP` (docker.rs:981) runs in the main shell right after the secret read, copies every entry of `$PWD` but `externals` (dot-entries included, `bin` copied not linked), links `externals` to the cache, `cd`s into the root. Spliced by `github_runner_script` (:1000) into both modes before `config.sh` / `run.sh`. Run under bash by `a_shared_jit_runner_...`, `a_shared_persistent_runner_...`, `the_runner_root_gets_every_entry_...` |
| 1. Per-image cache under `/var/lib/gridops-agent/externals/<id>`, filled once in Prepare, mounted read-only at the same path | YES | `fill_externals_cache` (:1124) called from `prepare` (:118) after the pull; `shared_socket_args` (:941) uses `--mount type=bind,...,readonly`. Tests: `the_cache_is_filled_once_per_image_...`, `a_shared_runner_gets_its_folders_...` |
| 1. Runner root at `/var/lib/gridops-agent/runners/<environment id>`, outside the work folder, from the typed id only | YES | `shared_runner_root(PreparedEnvironmentId)` in `work_folder.rs`; tests `a_runner_root_is_a_direct_child_...` and `the_runner_root_and_the_externals_cache_are_never_under_a_work_folder` |
| 1. Copy cost stated, measured or estimated, saying which | YES | handoff-F: per start 272 files / 84,797,525 bytes; one-time fill 9,122 files / 599,417,734 bytes; estimated from the runner 2.338.2 tarball because the pool image could not be pulled ("unauthorized"). `docs/fleet-agent.md` repeats the figures and labels them estimates |
| 1. Cache name from `{{.Id}}` by a pure, unit-tested function; one non-empty path component; doc says how formed; bad id is `Runtime, true` | YES | `externals_cache_name` (hex without `sha256:`, lowercase hex only); `the_cache_name_is_...`; bad id covered in `the_start_check_...` and the fill's docker-error loop |
| 1. Every Start resolves `{{.Id}}` itself, checks that cache, creates the container from that id | YES in code, PARTLY pinned by tests (see finding S-1) | `shared_runner` (:1063) resolves, checks, then creates folders; `start` (:203) sets `image = runner.image_id` and creates from it. Verified by reading |
| 1. Symlinked `externals` followed or refused, doc comment, test pins it, no dangling link ever cached | YES | Followed (`docker cp -L`); doc comment on `fill_externals_cache`; `link_out_of` refuses links that leave the tree with `Configuration, false`; `a_symlinked_externals_is_followed_...` |
| 1. Cleanup removes the runner root with the runner | YES | `parse_shared_folders_leftover` returns work folder and runner root, never the cache; `finds_the_work_folder_and_the_runner_root_but_never_the_externals_cache`, `cleanup_removes_...` |
| 1 / decision 8. Stale caches swept after a new fill via the existing `missing()` | YES | `sweep_externals_caches` (:1307) under the fill lock; `a_fill_sweeps_caches_whose_image_is_gone_...` has both cases |
| 2. Tool cache at `<work>/_tool`, image `.env` cannot override | YES | `RUNNER_TOOL_CACHE` and `AGENT_TOOLSDIRECTORY` set with `-e`; `.env` lines rewritten; `the_runner_resolves_its_tool_cache_to_the_work_folder_whatever_the_image_sets` |
| 3. Fail loudly: Prepare cannot fill, Start cache missing, each logged | YES | One private `Refusal` carries code, retryability and reason; `answer()` logs it with `tracing::warn!`. Start's check runs in the socket branch, before either script is used |
| 4. GitLab, Tart, native, no-socket runners unchanged | YES | `uses_externals_cache` is the single predicate (`docker_socket && !is_gitlab`), used by both Prepare and Start; Prepare's `request.docker_socket` is the profile's `requires_docker_socket` (dispatch.rs:336, :365), so Prepare and Start agree. Guard assertions added to `tests/docker_gitlab.rs` and `tests/docker_runtime.rs`; script is byte-identical when `shared == false` |
| 5. Tests, no daemon needed; every failure path has an exact `(FailureCode, retryable)` pair and reason text | YES (one gap, S-1) | All five listed failure paths are tested with exact pairs; the reason text is asserted at the failing function. The Start-level test asserts the pair but not the text (the text is asserted on `externals_cache`) |
| 6. Docs: fleet-agent.md, `work_folder.rs` module doc, CHANGELOG with Upgrading item | YES | All present. CHANGELOG has the `### Upgrading` item and the `### Fixed` entry; fleet-agent.md carries its own "Upgrading" sentence; anchors resolve |

**Constraints.** `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` used by F and T. No fleet host, Mars or deployed agent touched. What was and was not exercised is stated plainly in handoff-F and handoff-T-green; I agree with that list (below).

**Production code read for defects, none found.** Points I checked specifically:
- Secret-read ordering: the setup runs after `IFS= read -r S`, so the 85 MB copy cannot race `feed_secret`.
- `set -e` safety of `[ ... = externals ] && continue` and of the three glob arms (unmatched globs are skipped by the `-e`/`-L` guard).
- A retried Start over a populated runner root (`cp -a` merges, `ln -sfn` replaces the link).
- `StateDirectoryMode=0700` with `ProtectSystem=full`: the daemon is root and the job container mounts the cache directory itself, so the 0700 parent does not block it; same precedent as the work folder.
- A failed `docker create` or `feed_secret` removes both folders; a failed runner-root creation removes the work folder.
- Cleanup reading `{{.Config.Image}}` now sees the image id, which `docker run` accepts while the container still exists.

## Quality audit
| Area | Result | Notes |
|------|--------|-------|
| API consistency | N/A | No API, wire, schema or protocol change; no new `FailureCode` or `DiagnosticCode` |
| Error handling | PASS | Every refusal is a typed `Refusal` with the brief's pair; Docker/IO errors are retryable, image content and permission are not |
| UI/UX match to spec | N/A | No UI |
| Accessibility | N/A | No UI |
| Architecture gate | PASS | A round 3 PASS; A-dup PASS. F's round 2 closed A's two warns |
| Code organization | PASS | One pure function per concern (`externals_cache_name`, `externals_cache`, `shared_socket_args`, `github_runner_script`, `shared_runner`); core holds the shared paths |
| Security | PASS | `docker cp` output is checked for links that leave the tree; cache name is lowercase hex only; cleanup parse accepts only direct children of the two roots and never `.` or `..`; no secret is added to args or env |
| Performance | PASS | Cache fill is once per image in Prepare (20 min budget); Start adds one `image inspect`, two `mkdir`s and one `docker create` inside its 25 s budget; tree removals run under `spawn_blocking` |
| Visual regression | N/A | No UI |
| Naming consistency | PASS | The shared-folder family now has one vocabulary after F's round 2 |
| Test quality (`testing/test-quality.md` section 7) | FAIL (one finding, test-only) | See below |

### Test-quality audit (rubric section 7)
Per test, the 20 tests in `externals_tests.rs`, the 6 in `shared_folder_tests`, the 2 in `work_folder.rs` and the added guard assertions each name one behaviour, run at a sensible tier (pure function, script under bash, stand-in `docker` without a daemon) and are behavioural. T's mutation spot-checks (readonly dropped, `.env` rewrite removed, `..?*` arm removed) show the tests fail for the right reason. The suite is proportionate: this is a feature story with about 14 new failure and success branches, and the loops in `a_docker_error_during_the_fill_...` fan across distinct branches (`create`, `cp`, `rm`, inspect, bad id), not one branch.

**S-1 (blocking, TEST-ONLY): Start's "resolve the id itself and use it" behaviour is pinned by no test, although T said it would be and the seam exists.**
- The brief makes it binding twice: item 1 ("Every Start resolves the image's `{{.Id}}` itself ... and creates the runner container from that id ... A re-pull that moves the tag between Prepare and Start therefore either finds that id's cache or fails with the cache-missing code; it never runs one image's runner against another image's cache") and decision 7.
- `handoff-T-red.md` ("Not covered by tests, and why") lists it as read-only and says: "If F routes the socket branch through a helper that takes the roots as `&Path`, I will add a stand-in test of it". F did: `shared_runner(socket, image, &SharedPaths)` (docker.rs:1063), whose `SharedPaths` fields are plain `PathBuf`s, and F's handoff flagged it "Seam for T". T-green round 1 and round 2 left it as "PASS by reading".
- `grep` finds no test that calls `shared_runner`, `SharedPaths` or reads `image_id`. Two consequences:
  - Breaking decision 7 (resolving the id once and reusing it, or returning the tag instead of the id) turns no test red.
  - `a_shared_start_without_a_cache_fails_retryably_and_creates_nothing` (externals_tests.rs:980) cannot tell "no cache for the resolved id" from "the image inspect failed": both answer `Runtime, true` with no `docker create`. It also does not assert that `image inspect` ran for the tag, nor the reason text.
- I verified the wiring by reading, as T asked of S: `start` passes `&image` (the tag) to `shared_runner`, which resolves `{{.Id}}` on that call, checks `<cache_root>/<hex>` and only then creates the folders; `start` then sets `image = runner.image_id` and creates the container from it. The code is correct. The gap is the missing tripwire.

**Required change (T, test-only; no `src/` change):** add one stand-in test of `shared_runner` in `externals_tests.rs`, using temp roots for `SharedPaths` and the existing `StandIn` (`docker.set("id", ...)` moves what the tag resolves to). Two cases in one test:
1. Cache present for the id the tag resolves to. Assert `Ok`, `image_id == "sha256:<HEX>"` (not `TAG`), `args` contains the read-only `--mount` of `<cache_root>/<HEX>`, `folders` lists the work folder and the runner root, and both directories exist.
2. Tag moved: a cache exists for `HEX`, the stand-in now resolves the tag to a different id. Assert `Err` with `Runtime, true`, a reason naming the new `sha256:<id>` and its cache path, an `image inspect` call for the tag in the stand-in's `calls`, and that neither the work folder nor the runner root was created.

This also gives a deterministic check of "cache before folders" that no longer depends on `/var/lib/gridops-agent` being unwritable on the test machine. The one remaining unpinned line, `image = runner.image_id` in `start` (docker.rs:203), needs a real writable `/var/lib/gridops-agent` to run, so it stays verified by reading; do not add a production seam for it in this cycle.

## Scope check
F delivered the brief's scope. Nothing outside `docker.rs`, `work_folder.rs`, `fleet/mod.rs`, the two existing test files (assertions only), docs and CHANGELOG changed; the server, reconciler, manager and `instrumented_runner_script` are untouched. Small additions beyond the letter of the brief are each justified in F's handoff and A's review: the chmod-and-retry in `remove_tree_blocking`, the link-escape refusal, `.` rejected by the cleanup parse, `remove_dir`-first in `remove_shared_folder`. No under-delivery.

## Verdict

REWORK — one finding, test-only (no production change required):

1. **S-1.** Add the `shared_runner` stand-in test described above (T repairs; `reworkKind: test-only`). Everything else passes: spec compliance, production code, docs and CHANGELOG need no change. After this test is green the audit is PASS.

## Advisory notes
Non-blocking; none needs action for the verdict.
- **Not exercised by any test, needing a real Docker daemon or fleet host** (agreed with T and F): Start's success path, `docker cp -L` and `--mount ...,readonly` on a real engine, and the premise of the fix, a host bind source resolved through the runner root's `externals` link. The diff gate's NV-1 to NV-5 (Docker and actions/runner source claims) stay open for the first live run. The first fleet host to take this build should run one `container:` job before the change is called fixed.
- **Upgrade window.** An environment prepared by the old agent has no cache; after the host agent is upgraded its Start fails `Runtime, true` until the placement is re-prepared. This matches decision 5 and the docs say a start fails when the cache is missing; whether the server re-prepares on a retryable Start failure is outside this change.
- **Silent skips.** The Start, Prepare-wiring and 0555 tests return early on root or on a host with `/var/lib/gridops-agent`. They run on a normal non-root CI runner; a root-only CI would silently skip them.
- **Sweep test, second case** (`a_fill_sweeps_caches_whose_image_is_gone_...`): the `.partial` filter on `left` is unnecessary (staging is removed by name, and the first case pins it). Dropping the filter is harmless; not a gate item.
- **`the_stand_in_answers_like_docker_29`** checks the stand-in against recorded Docker 29.8 messages. It is a fidelity pin on a test double and can only fail when the double is edited. Keeping it is defensible; deleting it would lose no production signal.
- **`only_a_github_runner_that_shares_the_socket_uses_the_externals_cache`** has 7 rows for one predicate; the org and group rows add no branch beyond `is_gitlab()`. Cheap, left as is.
- The runner scripts' forwarder state is the fixed `/tmp/gridops-forwarded-job-state` (core's script). The new script tests serialise within one process with `SCRIPT_LOCK`, but two worktrees running these tests at once would share that path, as the manager's existing forwarder test already does.
