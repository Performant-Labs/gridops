# Decision journal: #146 GitLab HTTP client implementing GitLabApi

## A (Phase 3, up-front plan review): 2026-11-20T15:14:37-08:00
- **Decided:** PASS, with 0 block findings and 5 warn findings (see `handoff-A.md`).
  - **Accepted as consistent with the code:**
    - placement in `gitlab/http.rs`, next to `fake.rs`, implementing the #137 trait;
    - no new dependency, and only the one allowed line in `mod.rs`;
    - `user_agent("GridOps/0.1")` with `redirect(Policy::none())`, as in `BitbucketClient` and `GitHubClient`;
    - a hand-written mock server on tokio `TcpListener` (the precedent is `stand_in_docker.rs`; axum isn't a `gridops-core` dependency).
  - **Accepted as deliberate departures the brief justifies in writing:**
    - typed `GitLabError`, from the contract;
    - no response body in any error (the `glrt-` token);
    - the base URL passed on each call (D2).
  - **The five warnings:**
    - (1) Pin `#[derive(Clone)] GitLabClient { http }` with `new() -> anyhow::Result<Self>`, re-exported from `gitlab/mod.rs` and not from `lib.rs`. No wave-1 issue owns wiring `Arc::new(GitLabClient::new()?)` into the API's and the reconciler's `main.rs`.
    - (2) Use one `send` and `error_for` seam, with 404 checked before mapping in `delete_runner`. Don't copy Bitbucket's `send`, which makes its own `delete_runner` 404 branch unreachable.
    - (3) Pin `CONNECT_TIMEOUT`, `REQUEST_TIMEOUT` and `MAX_PAGES` as constants.
    - (4) Put the tests in `gitlab/http_tests.rs`, mounted as `fake.rs` mounts its tests. Bound the mock server. Give the live test the reasoned `#[ignore = …]` form and document its run command, as `sandbox_live.rs` does.
    - (5) No `base_url` scheme policy is stated anywhere. #138's normalisation is the seam for it, and the host agent's `InsecureUrl` (https, or loopback http) is the precedent.
- **Assumed:**
  - The mock server can serve `MAX_PAGES` responses quickly enough that no test-only page-cap hook is needed. This was not measured.
  - #138, if it lands before #146, has some production stand-in for `AppState.gitlab`. Its issue text says only "inject `Arc<dyn GitLabApi>` … so tests swap in the fake".
- **Hedged:**
  - Finding 1's wiring gap and finding 5's scheme policy are epic-level: #146 owns neither `main.rs` nor connection input. I reported them for O to record on #136 and did not block #146 on them.
  - The auth header (`PRIVATE-TOKEN` versus the neighbours' `bearer_auth`) I left as acceptable either way, folded into finding 2.
  - No Reuse map (`survey.md`) was provided, so the brief's named analogues were the baseline.
- **Evidence:**
  - Read: the brief, the brief-gate result r1, `docs/handoffs/137-brief.md` (D2 to D4, B-6, R2-W3, the `base_url` normalisation at :453) and `docs/handoffs/137/handoff-A.md`.
  - Issues: epic #136 (body, and the deviations comment D1 to D16), #138 and #139 (`gh issue view`).
  - Contract and fake: `crates/gridops-core/src/gitlab/{mod.rs,fake.rs,fake_tests.rs}`.
  - Neighbouring clients: `crates/gridops-core/src/{bitbucket.rs,github.rs,lib.rs}`, `crates/gridops-core/Cargo.toml` and the workspace `Cargo.toml` (deps and lints).
  - Client construction: `crates/gridops-api/src/{main.rs:82-83,state.rs}` and `crates/gridops-reconciler/src/main.rs:367-368`.
  - Precedents:
    - `crates/gridops-core/src/fleet/domain.rs` (`ExternalId` serde);
    - `crates/gridops-host-agent/src/client.rs:65-78` and `crates/gridops-agent/src/provider_auth.rs:334-335` (timeouts and `InsecureUrl`);
    - `crates/gridops-manager/src/{stand_in_docker.rs,route_timeout_tests.rs}`, `crates/gridops-reconciler/src/start_tests.rs` and `crates/gridops-api/src/oauth.rs` (mock servers);
    - `crates/gridops-agent/tests/sandbox_live.rs` (the ignored live test);
    - a `#[path = …]` grep across `crates/*/src` (the sibling test-module pattern).

## T (Phase 4, RED): 2026-11-20T15:33:50-08:00
- **Decided:** RED is valid. I wrote 18 unit tests in `crates/gridops-core/src/gitlab/http_tests.rs`, all run against a hand-written tokio `TcpListener` mock, and 1 ignored live test in `crates/gridops-core/tests/gitlab_live.rs`. Both files are staged by path.
  - On the real tree the RED is at compile time: E0432 unresolved `gridops_core::gitlab::GitLabClient`.
  - With a throwaway stub it is at runtime: 18 of 18 fail on their assertions.
  - The suite pins behaviour over HTTP, not private names. No test references `MAX_PAGES`, the timeouts or a Retry-After helper.
  - A's warnings W1 and W4 are adopted. The `pub use` re-export is required, because the live test imports it. The tests are in `http_tests.rs`, and the live test uses the reasoned `#[ignore = …]` form.
- **Assumed:**
  - GitLab's non-admin `GET /user` omits `is_admin`, so the mock omits it, which means F must default it to false.
  - Query parameter order and `[]` percent-encoding are F's choice, so the tests compare decoded, sorted pairs.
  - Paths are compared raw, with uppercase `%2F`.
  - `tag_list` may be sent as an array or as a comma-joined string.
- **Hedged:**
  - Three items are left to a Phase 6 code read: the request timeout, "never log a body" (there is no log-capture dependency, and no new one is allowed), and `set_sensitive`.
  - The `" 30 "` trim isn't separately observable, because hyper strips surrounding whitespace.
  - I did not run the live test, because I have no `GITLAB_TEST_TOKEN`.
  - The clippy `-D warnings` gate already fails on `autoscaling.rs:1615/1701`, a file outside #146. I flagged it for O.
- **Evidence:**
  - I checked that the suite can be satisfied with a throwaway draft `http.rs`: 18 of 18 GREEN, and no clippy or fmt findings in either test file. The draft was deleted and mod.rs restored.
  - I ran 17 single mutations of that draft (listed in `handoff-T-red.md`). Each one turns at least one test red, and none hangs.
  - I fixed two invalid tests along the way. The later-page test passed against the stub; it now asserts page 2 was reached. The page-cap test hung when the cap was removed; its mock now stops at page 1001.
  - A 1000-page cap runs in 0.36s.

## F (Phase 6, implementation): 2026-11-20T15:42:47-08:00
- **Decided:** I implemented `GitLabClient` in `crates/gridops-core/src/gitlab/http.rs` and added `pub mod http;` and `pub use http::GitLabClient;` to `gitlab/mod.rs`. All 18 of T's unit tests are GREEN, and the live test compiles and is listed as ignored.
  - I took A's pins: `#[derive(Debug, Clone)] GitLabClient { http }` with `new() -> anyhow::Result<Self>`, the `send`/`error_for`/`get_json`/`get_all_pages` seam with 404 checked before mapping in `delete_runner`, and the constants 10s, 30s and `MAX_PAGES` = 100.
  - **`MissingScope` is sanitised.** A 403 body's `scope` is repeated only if it matches `[a-z_ ]{1,64}`; otherwise it is reported as `api`.
  - **`resolve_scope(Group)` sends `with_projects=false`.**
  - **No logging** in `http.rs`.
- **Assumed:**
  - GitLab's `insufficient_scope` `scope` field is a short space-separated list of scope names. A real value outside `[a-z_ ]` would be reported as `api`, which is still a correct "missing scope" signal.
  - `with_projects=false` is still honoured by GitLab v4. It is deprecated, not removed.
- **Hedged:**
  - The live test was not run, because there is no `GITLAB_TEST_TOKEN`.
  - `cargo clippy --all-targets -D warnings` still fails on `autoscaling.rs:1615/1701`. Those failures were already there and are outside #146; I left them for O.
  - Production wiring of `GitLabClient` is still unowned (A-W1, epic-level).
- **Evidence:**
  - `cargo test -p gridops-core`: the lib has 279 passed, and every integration binary is ok.
  - `cargo fmt --all --check` is clean.
  - `cargo clippy -p gridops-core --lib -D warnings` is clean.
  - `cargo check --workspace --all-targets` is clean.
  - The facts this relies on in unchanged code are in `docs/handoffs/146/evidence.md`.

## T (Phase 7, GREEN): 2026-11-20
- **Decided:** The suite is GREEN with no blocking Tier 2 issue. I changed no test and no production code (F flagged no wrong tests).
- **Assumed:** The two clippy `--all-targets` failures in `autoscaling.rs` are pre-existing and not a #146 blocker; they matched T-red's report from before F's change.
- **Hedged:**
  - The live test was not run (no `GITLAB_TEST_TOKEN`).
  - The `Retry-After` trim branch and the 30s timeout are verified by reading, not by test.
- **Evidence:**
  - `cargo test -p gridops-core --lib gitlab::http`: 18 passed. Full crate: lib 279 passed, all integration binaries ok.
  - Two mutations applied to the real `http.rs` (same-origin check off, 404 not forgiven on delete) each failed one test; the file was restored and `git status` is clean.
  - fmt and lib clippy are clean.

## S (Phase 10, spec audit): 2026-11-20T15:58:00-08:00
- **Decided:** PASS. Read `http.rs`, `http_tests.rs`, `gitlab_live.rs` and the `mod.rs` diff against every clause of the brief; all are met (see `handoff-S.md`).
  - No browser or visual-diff precondition applies (no visual surface), so there is no HTML report.
  - The clippy `--all-targets` failure T and F left for O is a toolchain artifact, not a repo defect: CI pins 1.98.1 (`.github/workflows/ci.yml:88-101`), and `cargo +1.98.1 clippy -p gridops-core --all-targets --locked -- -D warnings -A clippy::unused_async_trait_impl` and `cargo +1.98.1 fmt --all -- --check` are both clean. The two failures at `autoscaling.rs:1615/1701` come from the local default toolchain (1.99.1).
  - Test quality passes the rubric. Two small items (the `" 30 "` row that hyper's whitespace stripping makes unobservable, and 4 statuses sharing one `error_for` arm in the leak test) are advisory, not rework.
  - The diff-gate warnings are dispositioned: W-1 (`MAX_PAGES` 100) is A-W3's pin; W-2 (`tests/gitlab_live.rs`) is A-W4's named location; W-3 (clippy) is resolved as above; W-4 (membership 403 → `MissingRole`) is acceptable because step 1 already proved the scopes.
- **Assumed:**
  - T's and F's independent test runs (18/18, full crate green) still describe the tree at `8b0cad1`; the only commits after T-green are the a-dup result file. I did not re-run the tests, because that is T's job.
- **Hedged:**
  - The live test has never run (no `GITLAB_TEST_TOKEN`), so wire behaviour against a real GitLab rests on GitLab's source and docs plus the mock server.
  - GitLab's own docs disagree on the project-runner role: `runners_scope.md` says Maintainer and `users.md` says Owner. The brief follows the first. If the second is right, `verify_access` passes a Maintainer and `create_runner` then returns `MissingRole`, so it fails safe. Not a hold; the by-hand live run with a Maintainer-not-Owner token settles it.
  - Granular-scope tokens (GitLab 19.2+) may omit `scopes`, which would give a generic `Upstream` instead of `MissingScope`. It fails closed.
  - `handoff-A-dup.md` is not in this directory; the a-dup result is `docs/handoffs/146-diff-result-r1.md`.
- **Evidence:**
  - Read: the brief, `handoff-A.md`, `handoff-F.md`, `handoff-T-red.md`, `handoff-T-green.md`, `146-diff-result-r1.md`, `decisions.md`, `evidence.md`, `http.rs`, `http_tests.rs`, `gitlab_live.rs`, `mod.rs`, `.github/workflows/ci.yml`, and the release's `testing/test-quality.md`.
  - Ran: `cargo +1.98.1 clippy ...` and `cargo +1.98.1 fmt ... --check` (both clean); `git diff cdda02e HEAD --stat` (4 code files, no `Cargo.toml`, no contract change); `git fetch` and a diff of `origin/master` (no change under `gitlab/` or `tests/`).
  - GitLab master docs and source fetched 2026-11-20: `doc/api/{users,jobs,groups,runners,personal_access_tokens}.md`, `doc/ci/runners/runners_scope.md`, `lib/api/user_runners.rb` (`tag_list` is `Array[String]` with `CommaSeparatedToArray`) and `lib/api/validations/types/comma_separated_to_array.rb`.
