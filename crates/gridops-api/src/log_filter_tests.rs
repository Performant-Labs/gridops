//! The API logs `SQLite` statements that run for 100 ms or more (#179). On
//! 2026-11-21 scans of 176 ms and 375 ms held the write lock until every
//! other writer timed out, and nothing was logged: sqlx's warning goes to the
//! `sqlx::query` target, which the API's default filter dropped, and its
//! default threshold is 1 s.
//!
//! sqlx emits the warning on its connection worker thread, so only a
//! process-wide subscriber sees it. This module installs one, built from the
//! API's own default filter.

use std::{
    collections::BTreeMap,
    fmt::Debug,
    sync::{Mutex, Once, PoisonError},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt as _},
};

#[derive(Clone, Debug)]
struct Captured {
    target: String,
    level: Level,
    fields: BTreeMap<String, String>,
}

static CAPTURED: Mutex<Vec<Captured>> = Mutex::new(Vec::new());

struct Capture;

impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        CAPTURED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Captured {
                target: event.metadata().target().to_owned(),
                level: *event.metadata().level(),
                fields: fields.0,
            });
    }
}

#[derive(Default)]
struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

/// The API's default filter, as `main` installs it, with a layer that keeps
/// every event it lets through.
fn install() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::registry()
                .with(super::default_log_filter())
                .with(Capture),
        );
    });
}

fn captured() -> Vec<Captured> {
    CAPTURED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn is_slow_warning(event: &Captured) -> bool {
    event
        .fields
        .get("message")
        .is_some_and(|message| message.contains("slow statement"))
}

#[tokio::test]
async fn a_statement_running_past_100_ms_is_logged_as_a_slow_statement_warning() -> Result<()> {
    install();
    let directory =
        std::env::temp_dir().join(format!("gridops-slow-statement-{}", uuid::Uuid::new_v4()));
    let pool = gridops_core::connect_database_path(&directory.join("db.sqlite")).await?;
    // Counts higher until the statement runs for at least 200 ms: past the
    // 100 ms threshold, well short of sqlx's default 1 s on any machine.
    let mut limit = 250_000_i64;
    loop {
        let started = Instant::now();
        sqlx::query(
            "WITH RECURSIVE slow_statement_probe(n) AS
               (SELECT 1 UNION ALL SELECT n + 1 FROM slow_statement_probe WHERE n < ?)
             SELECT count(*) FROM slow_statement_probe",
        )
        .bind(limit)
        .fetch_one(&pool)
        .await?;
        if started.elapsed() >= Duration::from_millis(200) {
            break;
        }
        if limit > 1 << 28 {
            bail!("no statement ran for 200 ms");
        }
        limit *= 2;
    }
    let probe = |event: &Captured| {
        event
            .fields
            .values()
            .any(|value| value.contains("slow_statement_probe"))
    };
    // The worker thread logs once the statement has finished.
    let mut warnings = Vec::new();
    for _ in 0..40 {
        warnings = captured().into_iter().filter(probe).collect();
        if !warnings.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    pool.close().await;
    let _ = std::fs::remove_dir_all(directory);

    assert!(
        !warnings.is_empty(),
        "a statement that ran for 200 ms logged nothing on sqlx::query"
    );
    for warning in &warnings {
        assert_eq!(warning.target, "sqlx::query", "{warning:?}");
        assert_eq!(warning.level, Level::WARN, "{warning:?}");
        assert!(is_slow_warning(warning), "{warning:?}");
        assert_eq!(
            warning.fields.get("slow_threshold").map(String::as_str),
            Some("100ms"),
            "{warning:?}"
        );
    }
    // Statements that aren't slow stay out of the log.
    let statements: Vec<Captured> = captured()
        .into_iter()
        .filter(|event| event.target == "sqlx::query" && !is_slow_warning(event))
        .collect();
    assert!(statements.is_empty(), "{statements:?}");
    Ok(())
}
