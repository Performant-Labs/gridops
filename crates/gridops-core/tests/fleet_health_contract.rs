//! #197: the fleet health contract that is implemented (not stubbed) here,
//! through `gridops_core::fleet_health`'s public API only:
//! - the wire spellings of the shared types (`docs/design/fleet-health.md`);
//! - the job classifier (`category.rs`): position order, first match wins,
//!   `other` as the fallback, case-insensitive, an invalid rule skipped by
//!   `compile` and named by `validate`;
//! - `merge_concerns` and `assemble`;
//! - the fixture: `src/features/fleet-health/fixture.json` deserializes into
//!   the Rust types and is exactly what the fake source serves and what the
//!   `OpenAPI` example shows, so the doc, the types, `OpenAPI` and the UI
//!   fixture can't drift apart;
//! - `FakeFleetHealthSource`'s seams for #194's cache tests.

use std::{sync::Arc, time::Duration};

use anyhow::{Context as _, Result};
use gridops_core::fleet_health::{
    Concern, ConcernSubject, FleetHealth, FleetHealthSource, HISTOGRAM_BUCKETS, HealthWindow,
    JobCategory, ScoreComponent, Severity, assemble,
    category::{self, CategoryRule, RuleField},
    fake::{self, FakeFleetHealthSource},
    histogram_bucket, merge_concerns,
};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("../../../src/features/fleet-health/fixture.json");
const OPENAPI: &str = include_str!("../../../docs/openapi/gridops-v1.json");

fn concern(key: &str, severity: Severity, since: Option<i64>) -> Concern {
    Concern {
        key: key.to_owned(),
        severity,
        title: format!("{key} title"),
        detail: format!("{key} detail."),
        subject: ConcernSubject::Fleet,
        link: Some("/fleet/health".to_owned()),
        since,
    }
}

fn rule(position: i64, category: JobCategory, field: RuleField, pattern: &str) -> CategoryRule {
    CategoryRule {
        position,
        category,
        field,
        pattern: pattern.to_owned(),
    }
}

fn labels(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn the_shared_types_have_the_pinned_wire_spellings() -> Result<()> {
    let categories: Vec<Value> = JobCategory::ALL
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()?;
    assert_eq!(
        categories,
        vec![
            json!("test"),
            json!("build"),
            json!("check"),
            json!("deploy"),
            json!("fix-agent"),
            json!("other")
        ]
    );
    for category in JobCategory::ALL {
        assert_eq!(
            category.as_str().parse::<JobCategory>().ok(),
            Some(category)
        );
        assert_eq!(serde_json::to_value(category)?, json!(category.as_str()));
    }
    assert!("fix_agent".parse::<JobCategory>().is_err());
    assert!("Tests".parse::<JobCategory>().is_err());

    let windows: Vec<(String, i64)> = HealthWindow::ALL
        .iter()
        .map(|window| (window.as_str().to_owned(), window.millis()))
        .collect();
    assert_eq!(
        windows,
        vec![
            ("1h".to_owned(), 3_600_000),
            ("24h".to_owned(), 86_400_000),
            ("7d".to_owned(), 604_800_000)
        ]
    );
    for window in HealthWindow::ALL {
        assert_eq!(serde_json::to_value(window)?, json!(window.as_str()));
        assert_eq!(window.as_str().parse::<HealthWindow>().ok(), Some(window));
    }
    for bad in ["30d", "1d", "24H", ""] {
        assert!(bad.parse::<HealthWindow>().is_err(), "{bad} parsed");
    }

    assert_eq!(
        serde_json::to_value([Severity::Critical, Severity::Warning, Severity::Info])?,
        json!(["critical", "warning", "info"])
    );
    assert!(Severity::Critical < Severity::Warning && Severity::Warning < Severity::Info);

    assert_eq!(
        serde_json::to_value(ConcernSubject::Fleet)?,
        json!({"kind": "fleet"})
    );
    assert_eq!(
        serde_json::to_value(ConcernSubject::Host("h-1".to_owned()))?,
        json!({"kind": "host", "id": "h-1"})
    );
    assert_eq!(
        serde_json::to_value(ConcernSubject::Pool("p-1".to_owned()))?,
        json!({"kind": "pool", "id": "p-1"})
    );

    let mut offline = concern(
        "host-offline:h-1",
        Severity::Critical,
        Some(1_700_000_000_000),
    );
    offline.subject = ConcernSubject::Host("h-1".to_owned());
    let wire = serde_json::to_value(&offline)?;
    assert_eq!(
        wire,
        json!({
            "key": "host-offline:h-1", "severity": "critical", "title": "host-offline:h-1 title",
            "detail": "host-offline:h-1 detail.", "subject": {"kind": "host", "id": "h-1"},
            "link": "/fleet/health", "since": 1_700_000_000_000_i64
        })
    );
    assert_eq!(serde_json::from_value::<Concern>(wire)?, offline);

    let component = ScoreComponent {
        key: "queue-wait".to_owned(),
        label: "Queue wait".to_owned(),
        weight: 40.0,
        score: None,
        deduction: Some("Not enough data: fewer than 20 jobs.".to_owned()),
    };
    assert_eq!(
        serde_json::to_value(&component)?,
        json!({"key": "queue-wait", "label": "Queue wait", "weight": 40.0, "score": null,
               "deduction": "Not enough data: fewer than 20 jobs."})
    );
    Ok(())
}

#[test]
fn histogram_buckets_are_the_nine_fixed_ranges_lower_bound_inclusive() {
    const SECOND: i64 = 1_000;
    const MINUTE: i64 = 60 * SECOND;
    assert_eq!(HISTOGRAM_BUCKETS, 9);
    for (millis, bucket) in [
        (0, 0),
        (10 * SECOND - 1, 0),
        (10 * SECOND, 1),
        (30 * SECOND - 1, 1),
        (30 * SECOND, 2),
        (MINUTE, 3),
        (2 * MINUTE, 4),
        (5 * MINUTE, 5),
        (10 * MINUTE, 6),
        (20 * MINUTE, 7),
        (40 * MINUTE - 1, 7),
        (40 * MINUTE, 8),
        (10 * 60 * MINUTE, 8),
        // A clock skew gives a negative wait; it is counted as no wait.
        (-5, 0),
    ] {
        assert_eq!(histogram_bucket(millis), bucket, "{millis} ms");
    }
}

#[test]
fn the_default_rules_classify_by_job_name_with_other_as_the_fallback() {
    let classifier = category::compile(&category::default_rules());
    for (name, expected) in [
        ("unit tests", JobCategory::Test),
        ("E2E (3/4)", JobCategory::Test),
        ("Playwright", JobCategory::Test),
        ("cargo test --workspace", JobCategory::Test),
        ("build-image", JobCategory::Build),
        ("Docker", JobCategory::Build),
        ("clippy", JobCategory::Check),
        ("TypeCheck", JobCategory::Check),
        ("deploy to Mars", JobCategory::Deploy),
        ("GitHub Pages", JobCategory::Deploy),
        ("notify slack", JobCategory::Other),
        // First match wins: `test` is position 0, before `build`.
        ("build and test", JobCategory::Test),
    ] {
        assert_eq!(
            classifier.classify(
                name,
                Some("CI"),
                Some("acme/app"),
                &labels(&["self-hosted"])
            ),
            expected,
            "{name}"
        );
    }
    // The defaults only look at the job name.
    assert_eq!(
        classifier.classify(
            "notify",
            Some("test suite"),
            Some("acme/test"),
            &labels(&["test"])
        ),
        JobCategory::Other
    );
    assert_eq!(
        category::compile(&[]).classify("unit tests", None, None, &[]),
        JobCategory::Other
    );
}

#[test]
fn rules_apply_in_position_order_over_every_field() {
    // Given out of order: compile orders them by position.
    let classifier = category::compile(&[
        rule(3, JobCategory::Deploy, RuleField::JobName, "."),
        rule(0, JobCategory::Check, RuleField::Labels, "^lint-runner$"),
        rule(
            2,
            JobCategory::Build,
            RuleField::Repository,
            "^acme/images$",
        ),
        rule(1, JobCategory::Test, RuleField::WorkflowName, "nightly"),
    ]);
    assert_eq!(
        classifier.classify(
            "x",
            Some("Nightly"),
            Some("acme/images"),
            &labels(&["LINT-RUNNER"])
        ),
        JobCategory::Check,
        "position 0 wins; a label rule matches any one label, case-insensitively"
    );
    assert_eq!(
        classifier.classify(
            "x",
            Some("Nightly"),
            Some("acme/images"),
            &labels(&["docker"])
        ),
        JobCategory::Test
    );
    assert_eq!(
        classifier.classify("x", Some("CI"), Some("acme/images"), &[]),
        JobCategory::Build
    );
    assert_eq!(
        classifier.classify("x", None, None, &[]),
        JobCategory::Deploy
    );
    // A missing workflow name or repository matches nothing.
    let workflow_only =
        category::compile(&[rule(0, JobCategory::Test, RuleField::WorkflowName, ".*")]);
    assert_eq!(
        workflow_only.classify("x", None, None, &[]),
        JobCategory::Other
    );

    let names: Vec<&str> = [
        RuleField::JobName,
        RuleField::WorkflowName,
        RuleField::Repository,
        RuleField::Labels,
    ]
    .iter()
    .map(|field| field.as_str())
    .collect();
    assert_eq!(names, ["job_name", "workflow_name", "repository", "labels"]);
}

#[test]
fn an_invalid_rule_is_skipped_by_compile_and_named_by_validate() {
    let rules = [
        rule(0, JobCategory::Build, RuleField::JobName, "("),
        rule(1, JobCategory::Test, RuleField::JobName, "build"),
        rule(2, JobCategory::Check, RuleField::JobName, "[z-a]"),
    ];
    assert_eq!(
        category::compile(&rules).classify("build", None, None, &[]),
        JobCategory::Test,
        "the broken position-0 rule is skipped, the next one applies"
    );
    let errors = match category::validate(&rules) {
        Ok(_) => Vec::new(),
        Err(errors) => errors,
    };
    let positions: Vec<i64> = errors.iter().map(|error| error.position).collect();
    assert_eq!(positions, [0, 2], "one error per invalid rule, in order");
    assert!(errors.iter().all(|error| !error.message.trim().is_empty()));
    assert!(category::validate(&rules[1..2]).is_ok());
    // A pattern past the regex size limit is refused rather than compiled
    // (a million repetitions is over any limit up to the crate's default).
    let huge = "(?:a{1000}){1000}";
    assert!(category::validate(&[rule(0, JobCategory::Test, RuleField::JobName, huge)]).is_err());
}

#[tokio::test]
async fn the_seeded_rules_load_in_order_and_equal_the_defaults() -> Result<()> {
    let directory = std::env::temp_dir().join(format!("gridops-rules-{}", uuid::Uuid::new_v4()));
    let pool = gridops_core::connect_database_path(&directory.join("db.sqlite")).await?;
    let mut conn = pool.acquire().await?;
    let loaded = category::load_rules(&mut conn).await?;
    assert_eq!(loaded, category::default_rules());
    sqlx::query("UPDATE job_category_rules SET position=position+10")
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO job_category_rules (position,category,field,pattern,created_at,updated_at) VALUES (5,'other','labels','gpu',1,1)")
        .execute(&mut *conn)
        .await?;
    let loaded = category::load_rules(&mut conn).await?;
    assert_eq!(loaded.first().map(|rule| rule.position), Some(5));
    assert_eq!(loaded.len(), 5);
    drop(conn);
    pool.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn merged_concerns_are_worst_first_then_oldest_first_one_per_key() {
    let merged = merge_concerns(
        vec![
            concern("fleet-component:queue-wait", Severity::Warning, None),
            concern("host-score:a", Severity::Critical, Some(300)),
            concern("category-other", Severity::Info, None),
            concern("host-slow:a:2000", Severity::Warning, Some(100)),
            concern("uncertain:a", Severity::Warning, Some(1)),
        ],
        vec![
            concern("host-offline:b", Severity::Critical, Some(200)),
            concern("oldest-queued", Severity::Warning, Some(150)),
            // The live view of a key replaces the windowed one.
            concern("uncertain:a", Severity::Critical, Some(50)),
        ],
    );
    let order: Vec<(&str, Severity)> = merged
        .iter()
        .map(|concern| (concern.key.as_str(), concern.severity))
        .collect();
    assert_eq!(
        order,
        vec![
            ("uncertain:a", Severity::Critical),
            ("host-offline:b", Severity::Critical),
            ("host-score:a", Severity::Critical),
            ("host-slow:a:2000", Severity::Warning),
            ("oldest-queued", Severity::Warning),
            // No `since` sorts after every dated concern of its severity.
            ("fleet-component:queue-wait", Severity::Warning),
            ("category-other", Severity::Info),
        ]
    );
    assert!(merge_concerns(Vec::new(), Vec::new()).is_empty());
}

#[test]
fn the_ui_fixture_is_the_rust_types_the_fake_and_the_openapi_example() -> Result<()> {
    let fixture_json: Value = serde_json::from_str(FIXTURE)?;
    let fixture: FleetHealth =
        serde_json::from_str(FIXTURE).context("fixture.json does not fit the Rust types")?;
    assert_eq!(
        serde_json::to_value(&fixture)?,
        fixture_json,
        "every fixture field round-trips through the Rust types"
    );
    assert_eq!(fixture.window, HealthWindow::Day);
    assert_eq!(fixture.generated_at, fake::FIXTURE_NOW);
    assert!(!fixture.stale);
    assert_eq!(
        serde_json::to_value(fake::fixture(HealthWindow::Day))?,
        fixture_json,
        "fixture.json is generated from the fake source"
    );

    let openapi: Value = serde_json::from_str(OPENAPI)?;
    let example = openapi
        .pointer(
            "/paths/~1api~1v1~1fleet~1health/get/responses/200/content/application~1json/example",
        )
        .context("GET /api/v1/fleet/health has no 200 example")?;
    assert_eq!(example, &fixture_json, "the OpenAPI example is the fixture");

    // An unknown field is a drift, not something to ignore.
    let mut extra = fixture_json.clone();
    extra["surprise"] = json!(true);
    assert!(serde_json::from_value::<FleetHealth>(extra).is_err());
    Ok(())
}

#[test]
fn the_fixture_has_the_five_machines_and_every_severity() -> Result<()> {
    let fixture: FleetHealth = serde_json::from_str(FIXTURE)?;
    let machines: Vec<(&str, u32)> = fixture
        .hosts
        .iter()
        .map(|host| (host.name.as_str(), host.cpu_budget_millis))
        .collect();
    let mut sorted = machines.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec![
            ("Jupiter", 19_200),
            ("MacBookPro", 0),
            ("callisto", 8_000),
            ("europa", 12_800),
            ("io", 22_800)
        ]
    );
    for severity in [Severity::Critical, Severity::Warning, Severity::Info] {
        assert!(
            fixture
                .concerns
                .iter()
                .any(|concern| concern.severity == severity),
            "the fixture has no {severity:?} concern"
        );
    }
    assert_eq!(
        fixture.concerns,
        merge_concerns(fixture.concerns.clone(), Vec::new()),
        "the fixture's concerns are already in merge order"
    );
    let ids: Vec<&str> = fixture.hosts.iter().map(|host| host.id.as_str()).collect();
    assert!(
        ids.contains(&fake::SLOW_HOST),
        "SLOW_HOST is a fixture machine"
    );
    assert!(
        ids.contains(&fake::FAILING_STARTS_HOST),
        "FAILING_STARTS_HOST is a fixture machine"
    );
    assert_ne!(fake::SLOW_HOST, fake::FAILING_STARTS_HOST);

    // The scoring inputs behind it: the same machines, one hour with a
    // queue backlog (jobs waiting 10 minutes or more).
    let inputs = fake::fixture_inputs();
    let mut input_ids: Vec<&str> = inputs.hosts.iter().map(|host| host.id.as_str()).collect();
    let mut fixture_ids = ids.clone();
    input_ids.sort_unstable();
    fixture_ids.sort_unstable();
    assert_eq!(input_ids, fixture_ids);
    assert!(
        inputs
            .hourly
            .iter()
            .any(|row| row.wait_histogram[6..].iter().sum::<u64>() > 0),
        "no hour has a queue backlog"
    );
    Ok(())
}

#[test]
fn assemble_joins_live_stats_and_merges_concerns() -> Result<()> {
    let windowed = fake::windowed_part(HealthWindow::Week);
    let live = fake::live_part();
    let health = assemble(HealthWindow::Week, 42, windowed.clone(), live.clone());
    assert_eq!(health.window, HealthWindow::Week);
    assert_eq!(health.generated_at, 42);
    assert!(!health.stale);
    assert_eq!(
        health.concerns,
        merge_concerns(windowed.concerns.clone(), live.concerns.clone())
    );
    assert_eq!(health.live, live.fleet);
    for host in &health.hosts {
        let stats = live.hosts.iter().find(|stats| stats.host_id == host.id);
        assert_eq!(host.live.as_ref(), stats, "{} live stats", host.name);
    }
    let day = fake::fixture(HealthWindow::Day);
    let week = fake::fixture(HealthWindow::Week);
    assert_eq!(week.window, HealthWindow::Week);
    assert_eq!(
        FleetHealth {
            window: HealthWindow::Day,
            ..week
        },
        day,
        "only `window` differs between the fake's windows"
    );
    Ok(())
}

#[tokio::test]
async fn the_fake_source_counts_calls_fails_on_demand_and_holds_calls() -> Result<()> {
    let fake = FakeFleetHealthSource::default();
    let source: Arc<dyn FleetHealthSource> = Arc::new(fake.clone());
    let windowed = source
        .windowed(HealthWindow::Hour, fake::FIXTURE_NOW)
        .await?;
    assert_eq!(windowed, fake::windowed_part(HealthWindow::Hour));
    assert_eq!(source.live(fake::FIXTURE_NOW).await?, fake::live_part());
    assert_eq!((fake.windowed_calls(), fake.live_calls()), (1, 1));

    fake.set_failing(true);
    assert!(source.windowed(HealthWindow::Day, 0).await.is_err());
    assert!(source.live(0).await.is_err());
    assert_eq!((fake.windowed_calls(), fake.live_calls()), (2, 2));
    fake.set_failing(false);
    assert!(source.windowed(HealthWindow::Day, 0).await.is_ok());

    fake.hold();
    let held = tokio::spawn({
        let source = Arc::clone(&source);
        async move { source.windowed(HealthWindow::Day, 0).await.is_ok() }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!held.is_finished(), "a held call waits for release()");
    assert_eq!(
        fake.windowed_calls(),
        4,
        "a held call is counted when it starts"
    );
    fake.release();
    assert!(tokio::time::timeout(Duration::from_secs(5), held).await??);
    Ok(())
}
