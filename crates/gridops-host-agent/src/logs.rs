//! Ships the agent's own log to `GridOps`.
//!
//! A tracing layer copies every event at info level or above into a bounded
//! in-memory buffer; the supervisor uploads it in batches while the session is
//! up. When `GridOps` is unreachable the buffer keeps the newest lines and
//! counts what it dropped, so a long outage can't grow memory without bound.
//! Credentials and registration material are never logged, so nothing secret
//! can be shipped.

use std::{
    collections::VecDeque,
    fmt::Write as _,
    sync::{Arc, Mutex},
};

use serde::Serialize;
use tracing::{Event, Level, Subscriber, field::Field};
use tracing_subscriber::{Layer, layer::Context};

/// Lines kept while `GridOps` can't be reached.
const BUFFER_LINES: usize = 5_000;
/// Lines sent per upload.
pub const BATCH_LINES: usize = 500;
const BATCH_BYTES: usize = 512 * 1024;
const MAX_MESSAGE: usize = 8_192;

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub logged_at: i64,
    pub level: &'static str,
    pub target: String,
    pub message: String,
}

#[derive(Default)]
struct State {
    lines: VecDeque<LogLine>,
    dropped: u64,
}

/// Shared between the capture layer and the uploader.
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<State>>);

impl LogBuffer {
    /// Takes up to a batch of the oldest lines, plus how many were dropped
    /// since the last batch.
    pub fn take(&self) -> (Vec<LogLine>, u64) {
        let Ok(mut state) = self.0.lock() else {
            return (Vec::new(), 0);
        };
        // Stay well under the API's 1 MiB request limit.
        let mut bytes = 0;
        let count = state
            .lines
            .iter()
            .take(BATCH_LINES)
            .take_while(|line| {
                bytes += line.message.len() + line.target.len() + 64;
                bytes <= BATCH_BYTES
            })
            .count()
            .max(usize::from(!state.lines.is_empty()));
        let lines = state.lines.drain(..count).collect();
        (lines, std::mem::take(&mut state.dropped))
    }

    /// Puts an unsent batch back in front of newer lines.
    pub fn restore(&self, lines: Vec<LogLine>, dropped: u64) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        state.dropped += dropped;
        for line in lines.into_iter().rev() {
            if state.lines.len() >= BUFFER_LINES {
                state.dropped += 1;
                continue;
            }
            state.lines.push_front(line);
        }
    }

    fn push(&self, line: LogLine) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.lines.len() >= BUFFER_LINES {
            state.lines.pop_front();
            state.dropped += 1;
        }
        state.lines.push_back(line);
    }

    /// The layer that fills this buffer.
    pub fn layer(&self) -> CaptureLayer {
        CaptureLayer(self.clone())
    }
}

pub struct CaptureLayer(LogBuffer);

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let level = *metadata.level();
        // sqlx warns whenever the journal waits on a slow disk; that stays in
        // the local log rather than filling GridOps.
        if level > Level::INFO || metadata.target().starts_with("sqlx") {
            return;
        }
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        let mut message = visitor.message;
        if !visitor.fields.is_empty() {
            if !message.is_empty() {
                message.push(' ');
            }
            message.push_str(&visitor.fields);
        }
        if message.len() > MAX_MESSAGE {
            let mut end = MAX_MESSAGE;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        let mut target = metadata.target().to_owned();
        target.truncate(128);
        self.0.push(LogLine {
            logged_at: gridops_core::now_millis(),
            level: match level {
                Level::ERROR => "error",
                Level::WARN => "warn",
                _ => "info",
            },
            target,
            message,
        });
    }
}

#[derive(Default)]
struct Visitor {
    message: String,
    fields: String,
}

impl tracing::field::Visit for Visitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value}", field.name());
        }
    }
}
