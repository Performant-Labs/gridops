//! The reconciler's default log filter admits sqlx's slow-statement warnings
//! (#179): its retention deletes and the fleet pass write to the database
//! every agent request waits on, and a statement that holds the write lock
//! for 100 ms or more must show in the log. sqlx's per-statement lines stay
//! out.

use std::sync::{Arc, Mutex, PoisonError};

use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt as _},
};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<(String, Level)>>>);

impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push((
            event.metadata().target().to_owned(),
            *event.metadata().level(),
        ));
    }
}

#[test]
fn the_default_filter_admits_slow_statement_warnings_and_not_every_statement() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry()
        .with(super::default_log_filter())
        .with(capture.clone());
    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!(target: "sqlx::query", "slow statement: execution time exceeded alert threshold");
        tracing::info!(target: "sqlx::query", "a statement");
        tracing::debug!(target: "sqlx::query", "a statement");
        tracing::info!(target: "gridops_reconciler", "a pass");
    });
    let seen = capture
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        seen,
        [
            ("sqlx::query".to_owned(), Level::WARN),
            ("gridops_reconciler".to_owned(), Level::INFO),
        ]
    );
}
