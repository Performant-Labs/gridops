# Decision journal: #139 GitLab reconciler runner lifecycle

## A (Phase 3, up-front plan review): 2026-11-20T15:19:09-08:00
- **Decided:** BLOCK, with 4 block findings and 5 warn findings (see `handoff-A.md`).
  - (1) The orphan sweep has no seam. `GitLabApi`, which the brief puts off limits, has no way to list runners, and #146 doesn't add one. Without the sweep, there is also no recovery for a failed GitLab delete (no cleanup table, no migration allowed).
  - (2) Pool delete, and the runner delete and rebuild actions, run in the API (`delete_runner_resources`, which sends GitLab runners to `cleanup_github_runner`). The `runners` rows then cascade away, so the reconciler can't clean them up.
  - (3) "Same insert" contradicts D1 on the fleet path. The fleet path registers in `register_and_start`, after the row and placement exist. On the manager path it departs from the analogue, which inserts, registers in `start_admitted_runner`, updates the ID, and rolls back through `failure_cleanup`.
  - (4) "main.rs gets only the call sites" leaves out `fleet.rs`, core `placement.rs` (the D5 diagnostic), the `Runner` and `ManagedRunner` structs, `delete_runner`'s platform dispatch, `Cargo.toml` and the test constructors. Read literally, it leads to a parallel pipeline.
  - These parts of the plan were accepted as consistent: the child-module placement (`super::` imports, as in `fleet.rs`), the `gitlab` field, per-call credentials through `runtime_secret`, connection selection modelled on `next_bitbucket_connection`, the GitLab arms in `targets.rs` and `service.rs`, and D9's `CiPlatform::Gitlab`.
- **Assumed:** #138's "GitLab sections of `resources.rs`" doesn't cover runner deletion; its issue text lists only the connection and pool-target handlers. A pool can't mix GitLab targets with others (#138), so a pool with `runner_pool_gitlab_connections` rows is a GitLab pool.
- **Hedged:**
  - Finding 1 offers two options, (a) a contract extension or (b) descoping plus a database-only retry. A gives no exact retry design, because none is established for a platform without a cleanup table.
  - Finding 3 accepts "create before insert" on the manager path, if the brief limits it to that path in writing and keeps rollback in the existing failure branch.
  - No Reuse map (`survey.md`) was provided, so the brief's named analogues were the baseline.
- **Evidence:**
  - Read: the brief, `139-brief-result-r1.md`, issues #136, #138, #146 and #148 (via `gh issue view`), and #137's `decisions.md` and `handoff-A.md`.
  - Source: `crates/gridops-reconciler/src/{main.rs,fleet.rs,start_tests.rs (head),Cargo.toml}`, `crates/gridops-core/src/gitlab/{mod.rs,fake.rs}`, `crates/gridops-core/src/fleet/{placement.rs,targets.rs,service.rs,protocol/agent.rs,protocol/outcomes.rs}`, `crates/gridops-core/src/provisioning.rs` (`failure_cleanup`), `crates/gridops-api/src/resources.rs` (`delete_runner_pool`, `delete_runner_resources`), `migrations/0047_gitlab.sql`, and `git show 08c32b3`.
