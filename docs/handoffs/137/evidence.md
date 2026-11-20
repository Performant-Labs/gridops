# Evidence: #137 GitLab shared contract

Source facts in unchanged code that the change, or T's tests, rely on.

## F (Phase 5, implementation)

- **Fact:** every pooled database connection has foreign keys on, so a plain `DROP TABLE runners` inside a normal migration would enforce the inbound FK actions. This is why 0046 runs outside sqlx's transaction and switches them off.
  **Source:** `crates/gridops-core/src/db.rs:29`
  **Verbatim excerpt:**
  > .foreign_keys(true)

- **Fact:** `runner_events.runner_id` cascades on delete from `runners`, so dropping `runners` with foreign keys on would delete the runner event history.
  **Source:** `migrations/0001_initial.sql:92`
  **Verbatim excerpt:**
  > FOREIGN KEY (`runner_id`) REFERENCES `runners`(`id`) ON UPDATE no action ON DELETE cascade,

- **Fact:** `log_streams.runner_id` is set to NULL when its runner is deleted.
  **Source:** `migrations/0001_initial.sql:47` (inside `CREATE TABLE log_streams`, which starts at line 34)
  **Verbatim excerpt:**
  > FOREIGN KEY (`runner_id`) REFERENCES `runners`(`id`) ON UPDATE no action ON DELETE set null

- **Fact:** `workload_placements.runner_id` references `runners(id)` with no action, so a dangling reference after the rebuild would be a `pragma_foreign_key_check` violation, which the 0046 guard catches.
  **Source:** `migrations/0022_fleet_foundation.sql:252`
  **Verbatim excerpt:**
  > runner_id TEXT REFERENCES runners(id),

- **Fact:** sqlx runs a migration outside its own transaction exactly when the file's text starts with `-- no-transaction`. 0046's first line is that directive, which lets its `PRAGMA foreign_keys` statements take effect.
  **Source:** `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/sqlx-core-0.9.0/src/migrate/source.rs:234`
  **Verbatim excerpt:**
  > let no_tx = sql.starts_with("-- no-transaction");

- **Fact:** the `PrepareEnvironment` validation applies the persistent-only and no-Windows-ARM64 rules through `CiTarget::is_bitbucket()`. That is why `is_bitbucket` now names the Bitbucket variants explicitly instead of `!is_github()`. These call sites are unchanged.
  **Source:** `crates/gridops-core/src/fleet/protocol/agent.rs:283-289`
  **Verbatim excerpt:**
  > || (prepare.target.is_bitbucket() && prepare.mode != RunnerMode::Persistent)
  > || (prepare.target.is_github()
  > && prepare.runtime_kind == RuntimeKind::TartVm
  > && prepare.mode != RunnerMode::Ephemeral)
  > || (prepare.target.is_bitbucket()
  > && prepare.execution_os == ExecutionOs::Windows
  > && prepare.architecture == Architecture::Arm64)

- **Fact:** in the unchanged capability check, a non-Bitbucket target on Tart must be ephemeral, and the Bitbucket rules apply only to `TargetKind::is_bitbucket()`. A GitLab target kind therefore gets the ephemeral rule, which matches the GitLab plan (always ephemeral), and none of the Bitbucket rules.
  **Source:** `crates/gridops-core/src/fleet/capabilities.rs:378-384`
  **Verbatim excerpt:**
  > if !requirement.runner.target_kind.is_bitbucket()
  > && requirement.runner.mode != RunnerMode::Ephemeral
  > {
  > return Err(CapabilityRejection::UnsupportedMode);
  > }
  > }
  > if requirement.runner.target_kind.is_bitbucket() {

- **Fact:** the manager forwards a Tart start to the Tart agent by re-serializing its `ProvisionRunner`. So the new `gitlab`/`gitlabRunnerToken` fields reach the Tart agent intact, and their `skip_serializing_if` keeps them out of forwarded GitHub and Bitbucket bodies.
  **Source:** `crates/gridops-manager/src/main.rs:1711`
  **Verbatim excerpt:**
  > let body = serde_json::to_value(&*input)
