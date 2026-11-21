# Evidence: #139 GitLab reconciler runner lifecycle

Facts the change (F, Phase 5) relies on that live in unchanged code. Line numbers are in this branch's tree after F's change.

- **Fact:** `retry_failed_start` itself takes the failed runner row out of service, so `finish` treats a retry made in that call as the moment the GitLab row left service.
  **Source:** `crates/gridops-core/src/fleet/retry.rs:269-270`
  **Verbatim excerpt:**
  > // The runner that failed is done; the pool counts its replacement instead.
  > sqlx::query("UPDATE runners SET status='failed',updated_at=? WHERE id=? AND status='starting'")

- **Fact:** a placement is retried only while its runner row is still `starting`. A GitLab row that F's fleet rollback already failed is never retried, so a failed `create_runner` cannot start a retry storm.
  **Source:** `crates/gridops-core/src/fleet/retry.rs:113`
  **Verbatim excerpt:**
  > if runner_status.as_deref() != Some("starting") {

- **Fact:** `abandon_unstarted` changes nothing once a `start_runner` command exists for the placement. F's rollback after `create_runner` or a failed bootstrap store therefore frees capacity only when no start command was issued.
  **Source:** `crates/gridops-core/src/fleet/placement.rs:387-394`
  **Verbatim excerpt:**
  > "SELECT EXISTS(SELECT 1 FROM host_commands WHERE placement_id=? AND kind='start_runner')",
  > )
  > .bind(&id)
  > .fetch_one(&mut *tx)
  > .await?;
  > if started {
  > return Ok(false);
  > }

- **Fact:** recording a placement failure writes the runner's `failure_reason` only when it is empty. The readable GitLab reason F writes first is kept, and a later agent message does not overwrite it.
  **Source:** `crates/gridops-core/src/fleet/failure.rs:218-220`
  **Verbatim excerpt:**
  > sqlx::query(
  > "UPDATE runners SET failure_reason=?,updated_at=?
  > WHERE id=? AND status!='deleted' AND COALESCE(failure_reason,'')=''",

- **Fact:** on the manager path, an exited, dead or missing container maps a runner to `failed`, and the same pass then deletes every `failed` runner through `delete_runner`. Those are the two points F's GitLab exit hook and the GitLab delete arm sit in.
  **Source:** `crates/gridops-reconciler/src/main.rs:903` and `crates/gridops-reconciler/src/main.rs:937-944`
  **Verbatim excerpt:**
  > "exited" | "dead" | "missing" => "failed",
  >
  > let failed = runners(app, &pool.id)
  > .await?
  > .into_iter()
  > .filter(|runner| runner.status == "failed")
  > .collect::<Vec<_>>();
  > for runner in &failed {
  > delete_runner(app, view, pool, runner).await?;
  > }

- **Fact:** every fleet count and reset runs in `fleet::tick`, which `reconcile()` calls before it loads the pools. The fleet path therefore needs no mid-pass pool reload; the manager path reloads after its status loop.
  **Source:** `crates/gridops-reconciler/src/main.rs:595` and `crates/gridops-reconciler/src/main.rs:635`
  **Verbatim excerpt:**
  > if let Err(error) = fleet::tick(app).await {
  >
  > let pools = load_pools(&app.database).await?;

- **Fact:** `record_provision_failure` counts from the `Pool` it is given and sets `backoff` (or `blocked` at three). Every count F makes outside the start loop is therefore given a pool loaded by `load_pool` at that moment.
  **Source:** `crates/gridops-reconciler/src/main.rs:3795-3805`
  **Verbatim excerpt:**
  > async fn record_provision_failure(database: &SqlitePool, pool: &Pool) -> Result<()> {
  > let attempts = pool.provision_failure_count.saturating_add(1);
  > let (circuit_open, delay_ms) = provision_failure_policy(attempts);
  > let retry_at = now_millis().saturating_add(delay_ms);
  > sqlx::query(
  > "UPDATE runner_pools SET provision_failure_count=?,provision_retry_at=?,provision_circuit_open=?,state=?,updated_at=? WHERE id=?",
  > )
  > .bind(attempts)
  > .bind(retry_at)
  > .bind(circuit_open)
  > .bind(if circuit_open { "blocked" } else { "backoff" })

- **Fact:** the threshold is three attempts, so three distinct refusals open the circuit.
  **Source:** `crates/gridops-reconciler/src/main.rs:3831-3836`
  **Verbatim excerpt:**
  > fn provision_failure_policy(attempts: i64) -> (bool, i64) {
  > if attempts >= 3 {
  > (true, PROVISION_CIRCUIT_COOLDOWN_MS)
  > } else {
  > (false, provision_backoff_ms(attempts))
  > }

- **Fact:** `FakeGitLab::delete_runner` answers `NotFound` for a runner it does not have. That is what F's "NotFound is done" handling (round-4 W3) is exercised against.
  **Source:** `crates/gridops-core/src/gitlab/fake.rs:226-229`
  **Verbatim excerpt:**
  > if instance.runners.len() == before {
  > Err(GitLabError::NotFound)
  > } else {
  > Ok(())

- **Fact:** choosing `CiPlatform::Gitlab` gives an ephemeral runner in a VM runtime. That is where the manager request's `mode:"ephemeral"` and `runtime:"vm"` come from.
  **Source:** `crates/gridops-core/src/provisioning.rs:64`
  **Verbatim excerpt:**
  > CiPlatform::Gitlab => ("ephemeral".to_owned(), true, "vm".to_owned()),

- **Fact:** the idle timeout sent on both paths is the published default.
  **Source:** `crates/gridops-core/src/provisioning.rs:9`
  **Verbatim excerpt:**
  > pub const GITLAB_DEFAULT_WAIT_TIMEOUT_SECS: u32 = 600;
