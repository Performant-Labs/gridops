//! `scripts/release-preflight.sh` against the real API, with a real agent:
//! the token checks it makes, and the machine read it parses, are the ones
//! the server actually answers. Its ssh, sudo and disk checks are covered by
//! `scripts/test-release-preflight.sh` with stand-ins.

use std::sync::Arc;

use gridops_core::api_tokens::ApiTokenScope;

use super::{
    api_tokens::mint,
    placement::{FakeDocker, active_machine, seed_pool},
    *,
};

struct Preflight {
    ok: bool,
    output: String,
}

impl Preflight {
    fn line(&self, prefix: &str) -> bool {
        self.output.lines().any(|line| line.starts_with(prefix))
    }
}

/// Runs the script with the release token and none of the optional settings.
fn preflight(f: &Fixture, token: &str) -> Result<Preflight> {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/release-preflight.sh");
    let mut command = std::process::Command::new("sh");
    command
        .arg(script.as_os_str())
        .env("GRIDOPS_URL", &f.origin)
        .env("GRIDOPS_TOKEN", token)
        .env("GRIDOPS_ALLOW_INSECURE", "1")
        .env_remove("GRIDOPS_API_TOKEN");
    for (name, _) in std::env::vars() {
        if name.starts_with("PREFLIGHT_") {
            command.env_remove(name);
        }
    }
    let output = command.output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // The token never reaches the output.
    assert!(!text.contains(token), "the token was printed:\n{text}");
    Ok(Preflight {
        ok: output.status.success(),
        output: text,
    })
}

async fn host_state(f: &Fixture, host: &str) -> Result<(String, i64)> {
    Ok(
        sqlx::query_as("SELECT lifecycle_state,revision FROM fleet_hosts WHERE id=?")
            .bind(host)
            .fetch_one(&f.database)
            .await?,
    )
}

#[tokio::test]
async fn the_release_preflight_passes_with_the_documented_token_and_a_healthy_fleet() -> Result<()>
{
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let machine = active_machine(&f, &f.origin, Arc::new(FakeDocker::default())).await?;
    let host = machine.host.to_string();
    let before = host_state(&f, &host).await?;
    let token = mint(
        &f,
        "release",
        &[ApiTokenScope::FleetRead, ApiTokenScope::FleetOperate],
    )
    .await?;

    let run = preflight(&f, &token)?;
    let out = &run.output;
    assert!(run.ok, "{out}");
    assert!(run.line("PASS  control-plane"), "{out}");
    assert!(run.line("PASS  token-read"), "{out}");
    assert!(run.line("PASS  token-operate"), "{out}");
    assert!(run.line("PASS  heartbeats: 1 machine, all fresh"), "{out}");
    assert!(run.line("PASS  start-gate"), "{out}");
    assert!(run.line("PASS  placement-failures"), "{out}");
    // What the API cannot tell a token is said, not guessed.
    assert!(run.line("SKIP  token-expiry"), "{out}");
    // Proving fleet:operate changed nothing.
    assert_eq!(host_state(&f, &host).await?, before);
    Ok(())
}

#[tokio::test]
async fn the_release_preflight_fails_a_token_that_cannot_operate_without_operating() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let machine = active_machine(&f, &f.origin, Arc::new(FakeDocker::default())).await?;
    let host = machine.host.to_string();
    let before = host_state(&f, &host).await?;

    let reader = mint(&f, "read-only", &[ApiTokenScope::FleetRead]).await?;
    let run = preflight(&f, &reader)?;
    let out = &run.output;
    assert!(!run.ok, "{out}");
    assert!(run.line("PASS  token-read"), "{out}");
    assert!(
        out.lines()
            .any(|line| line.starts_with("FAIL  token-operate") && line.contains("fleet:operate")),
        "{out}"
    );

    let operator = mint(&f, "operate-only", &[ApiTokenScope::FleetOperate]).await?;
    let run = preflight(&f, &operator)?;
    let out = &run.output;
    assert!(!run.ok, "{out}");
    assert!(
        out.lines()
            .any(|line| line.starts_with("FAIL  token-read") && line.contains("fleet:read")),
        "{out}"
    );
    assert!(run.line("PASS  token-operate"), "{out}");
    assert!(run.line("FAIL  heartbeats: not checked"), "{out}");

    let run = preflight(&f, "gat_NotARealTokenNotARealTokenNotARealToken00")?;
    assert!(!run.ok);
    assert!(
        run.line("FAIL  token-read: HTTP 401 unauthenticated"),
        "{}",
        run.output
    );
    assert!(run.line("FAIL  token-operate"), "{}", run.output);

    assert_eq!(host_state(&f, &host).await?, before);
    Ok(())
}

#[tokio::test]
async fn the_release_preflight_names_a_machine_that_keeps_failing_starts() -> Result<()> {
    let f = Fixture::new().await?;
    seed_pool(&f).await?;
    let machine = active_machine(&f, &f.origin, Arc::new(FakeDocker::default())).await?;
    let host = machine.host.to_string();
    let name: String = sqlx::query_scalar("SELECT name FROM fleet_hosts WHERE id=?")
        .bind(&host)
        .fetch_one(&f.database)
        .await?;
    // Three start failures just now: the machine is set aside.
    for _ in 0..3 {
        sqlx::query(
            "INSERT INTO fleet_events (id,host_id,placement_id,target_id,kind,detail_json,created_at)
             VALUES (?,?,NULL,NULL,'placement_failed',?,?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&host)
        .bind(json!({"reason": "start_deadline_expired", "message": "Timed out."}).to_string())
        .bind(gridops_core::now_millis())
        .execute(&f.database)
        .await?;
    }
    let token = mint(
        &f,
        "release",
        &[ApiTokenScope::FleetRead, ApiTokenScope::FleetOperate],
    )
    .await?;

    let run = preflight(&f, &token)?;
    let out = &run.output;
    assert!(!run.ok, "{out}");
    assert!(
        out.lines().any(|line| line.starts_with("FAIL  start-gate")
            && line.contains(&name)
            && line.contains("3 start failures")),
        "{out}"
    );
    assert!(
        out.lines()
            .any(|line| line.starts_with("FAIL  placement-failures")
                && line.contains(&format!("{name}: 3 in the last hour"))),
        "{out}"
    );
    assert!(run.line("PASS  heartbeats"), "{out}");
    Ok(())
}
