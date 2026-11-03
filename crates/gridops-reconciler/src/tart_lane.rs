//! The reconciler's Tart starts, run beside the pass (#95 phase 2, #104).
//!
//! A Tart runner start takes 30 to 65 seconds, and up to about five minutes
//! when the Tart agent waits for a macOS VM slot. Inside the pass it held up
//! every pool after it, Docker pools included. So a Tart start, once
//! admitted, runs in a task of its own and the pass carries on.
//!
//! Only one Tart start runs at a time. The manager's Tart lock serialises
//! Tart admissions and starts anyway: an admission sent while a start runs
//! would wait for the whole start, holding up the pass again. So while a
//! start runs, the pass asks for no other Tart runner (it *holds the pool
//! back*) and the start's own pool counts its runner as starting.
//!
//! The Mac's VM slot line (`gridops-manager`'s `vm_slots`) drops a pool that
//! did not ask in a whole round, a round being one pass identifier. A pool
//! held back did not stop asking, so a round stays open, and passes keep
//! sending the same identifier, until a pass has asked for every pool that
//! wanted a Tart runner.
//!
//! The pass is woken early when a Tart start ends (a pool held back can ask
//! now, and a failed start has freed its slot) and when a VM slot frees while
//! a pool waits in the line (`main.rs`'s `watch_vm_slots`).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use serde_json::Value;
use tokio::sync::Notify;

/// How often the reconciler looks at the Mac's VM slots while a pool waits
/// in the line for one.
pub(crate) const SLOT_WATCH_INTERVAL: Duration = Duration::from_secs(3);

/// How a Tart start that ran in the background ended, for the pass that
/// next visits its pool to record as it would have recorded it in the pass.
#[derive(Debug)]
pub(crate) enum Finished {
    Provisioned,
    /// The start was deferred, not failed (see `deferred_manager_error`).
    Deferred {
        reason: Option<String>,
        full_provider: Option<String>,
    },
    /// The start failed; the runner and its event are already recorded.
    /// `counts` when the failure counts towards the pool's provisioning
    /// circuit, as a failed start in the pass did; the error is the pool's
    /// pass error either way.
    Failed {
        error: anyhow::Error,
        counts: bool,
    },
}

#[derive(Debug, Default)]
struct LaneState {
    /// The pool and runner of the Tart start under way, once admitted.
    in_flight: Option<(String, String)>,
    /// A start is claimed: admission is being asked, or the start runs.
    claimed: bool,
    /// A pool was held back since the round began.
    held_back: bool,
    /// A pool waited in the Mac's line in this pass, and in the last one.
    line_waiting_now: bool,
    line_waiting: bool,
    /// Ended starts whose pool the pass has not visited since.
    finished: HashMap<String, Finished>,
}

/// The reconciler's lane for Tart starts.
#[derive(Debug)]
pub(crate) struct TartLane {
    state: Mutex<LaneState>,
    /// Wakes the reconciler for a pass before its interval is up.
    wake: Arc<Notify>,
    watch_interval: Duration,
}

impl TartLane {
    pub(crate) fn new(wake: Arc<Notify>, watch_interval: Duration) -> Self {
        Self {
            state: Mutex::default(),
            wake,
            watch_interval,
        }
    }

    fn state(&self) -> MutexGuard<'_, LaneState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn wake(&self) -> &Arc<Notify> {
        &self.wake
    }

    pub(crate) fn watch_interval(&self) -> Duration {
        self.watch_interval
    }

    /// Starts a pass. Answers whether it begins a new round of the Mac's
    /// line: not while a pool held back in the last pass has yet to ask.
    pub(crate) fn begin_pass(&self) -> bool {
        let mut state = self.state();
        let new_round = !state.held_back;
        state.held_back = false;
        state.line_waiting_now = false;
        new_round
    }

    /// Ends a pass: whether a pool now waits in the Mac's line is what this
    /// pass found.
    pub(crate) fn end_pass(&self) {
        let mut state = self.state();
        state.line_waiting = state.line_waiting_now;
    }

    /// Claims the lane for a Tart start, or answers `None` while another
    /// is claimed. The claim is given back when it is dropped.
    pub(crate) fn claim(self: &Arc<Self>) -> Option<TartClaim> {
        let mut state = self.state();
        if state.claimed {
            return None;
        }
        state.claimed = true;
        Some(TartClaim {
            lane: Arc::clone(self),
            started: false,
        })
    }

    /// A pool that wanted a Tart runner was not asked for, because another
    /// Tart start runs: the round stays open.
    pub(crate) fn hold_back(&self) {
        self.state().held_back = true;
    }

    /// A pool waits in the Mac's line (for a slot, or for room in
    /// `GridOps`'s own budget there) after this pass's admission.
    pub(crate) fn note_line_wait(&self) {
        self.state().line_waiting_now = true;
    }

    /// Whether to watch the slots: a pool waited in the line after the last
    /// pass and no Tart start runs (when one ends, the pass is woken anyway).
    pub(crate) fn watching_slots(&self) -> bool {
        let state = self.state();
        state.line_waiting && !state.claimed
    }

    /// Whether `runner_id`'s start is under way; nothing may remove it then.
    pub(crate) fn starting(&self, runner_id: &str) -> bool {
        self.state()
            .in_flight
            .as_ref()
            .is_some_and(|(_, id)| id == runner_id)
    }

    /// The ended start of `pool_id` not yet recorded by a pass, if any.
    pub(crate) fn take_finished(&self, pool_id: &str) -> Option<Finished> {
        self.state().finished.remove(pool_id)
    }

    /// Forgets ended starts of pools that are gone.
    pub(crate) fn retain_finished(&self, pool_ids: &[&str]) {
        self.state()
            .finished
            .retain(|pool_id, _| pool_ids.contains(&pool_id.as_str()));
    }

    #[cfg(test)]
    pub(crate) fn claimed(&self) -> bool {
        self.state().claimed
    }
}

/// The lane, held for one Tart start.
#[derive(Debug)]
pub(crate) struct TartClaim {
    lane: Arc<TartLane>,
    /// The start was admitted and runs.
    started: bool,
}

impl TartClaim {
    /// The start of `runner_id` for `pool_id` was admitted and now runs.
    pub(crate) fn start(&mut self, pool_id: &str, runner_id: &str) {
        self.started = true;
        self.lane.state().in_flight = Some((pool_id.to_owned(), runner_id.to_owned()));
    }

    /// The start ended: its pool's next pass records `finished`.
    pub(crate) fn finish(&mut self, finished: Finished) {
        let mut state = self.lane.state();
        if let Some((pool_id, _)) = state.in_flight.take() {
            state.finished.insert(pool_id, finished);
        }
    }
}

impl Drop for TartClaim {
    fn drop(&mut self) {
        {
            let mut state = self.lane.state();
            state.claimed = false;
            state.in_flight = None;
        }
        // An admission that was refused or failed started nothing, and the
        // pass that asked carries on; a start that ran wakes the pass to
        // record how it ended and to ask for the pools held back meanwhile.
        if self.started {
            self.lane.wake.notify_one();
        }
    }
}

/// What the reconciler watches in the Mac's VM slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SlotView {
    in_use: u64,
    free: u64,
}

impl SlotView {
    /// From the manager's `vmSlots` report; `None` without one.
    pub(crate) fn from_report(report: &Value) -> Option<Self> {
        Some(Self {
            in_use: report.get("inUse")?.as_u64()?,
            free: report.get("free")?.as_u64()?,
        })
    }
}

/// Whether a slot freed between two looks: fewer in use (a VM stopped,
/// `GridOps`'s or other software's, or a start gave its lease back), or more
/// free (the limit was raised). A view that only stays free does not count:
/// the head of the line may be waiting for room in `GridOps`'s own budget, and
/// asking again would change nothing.
pub(crate) fn slot_freed(before: SlotView, now: SlotView) -> bool {
    now.in_use < before.in_use || now.free > before.free
}

/// Whether to wake the pass for what the watcher sees `now`, after `last`
/// (`None` on its first look since a pool began to wait, or since a Tart
/// start ended). On a first look a free slot counts as freed: it may have
/// freed between the admission that put the pool in the line and this look,
/// and the pass that recorded the wait cannot have asked for it. That wakes
/// at most one pass more each time watching begins, also when the head of
/// the line waits for `GridOps`'s own budget rather than a slot.
pub(crate) fn wake_for_slots(last: Option<SlotView>, now: SlotView) -> bool {
    match last {
        Some(before) => slot_freed(before, now),
        None => now.free > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context as _, Result};
    use serde_json::json;

    fn lane() -> Arc<TartLane> {
        Arc::new(TartLane::new(Arc::default(), SLOT_WATCH_INTERVAL))
    }

    /// Whether the pass would be woken now.
    fn woken(lane: &TartLane) -> bool {
        let notified = std::pin::pin!(lane.wake().notified());
        futures_util::FutureExt::now_or_never(notified).is_some()
    }

    #[test]
    fn one_tart_start_at_a_time() -> Result<()> {
        let lane = lane();
        let mut first = lane.claim().context("the lane is free")?;
        assert!(lane.claim().is_none(), "a second start must wait");
        first.start("pool-a", "runner-a");
        assert!(lane.claimed());
        assert!(lane.starting("runner-a"));
        assert!(!lane.starting("runner-b"));
        first.finish(Finished::Provisioned);
        drop(first);
        assert!(!lane.claimed());
        assert!(!lane.starting("runner-a"));
        assert!(matches!(
            lane.take_finished("pool-a"),
            Some(Finished::Provisioned)
        ));
        assert!(lane.take_finished("pool-a").is_none(), "recorded once");
        assert!(lane.claim().is_some(), "free again");
        Ok(())
    }

    #[test]
    fn a_start_that_ran_wakes_the_pass_and_a_refused_admission_does_not() -> Result<()> {
        let lane = lane();
        drop(lane.claim().context("the lane is free")?);
        assert!(!woken(&lane), "a refused admission started nothing");
        let mut started = lane.claim().context("the lane is free again")?;
        started.start("pool-a", "runner-a");
        drop(started);
        assert!(woken(&lane), "the start ended: the pass records it at once");
        Ok(())
    }

    #[test]
    fn the_ended_start_of_a_pool_that_is_gone_is_forgotten() -> Result<()> {
        let lane = lane();
        for pool in ["pool-a", "pool-b"] {
            let mut claim = lane.claim().context("the lane is free")?;
            claim.start(pool, "runner");
            claim.finish(Finished::Provisioned);
        }
        lane.retain_finished(&["pool-b"]);
        assert!(lane.take_finished("pool-a").is_none());
        assert!(lane.take_finished("pool-b").is_some());
        Ok(())
    }

    #[test]
    fn a_round_stays_open_while_a_pool_is_held_back() {
        let lane = lane();
        assert!(lane.begin_pass(), "the first pass begins a round");
        assert!(lane.begin_pass(), "nobody was held back");
        lane.hold_back();
        assert!(
            !lane.begin_pass(),
            "a pool held back in the last pass has yet to ask in this round"
        );
        assert!(lane.begin_pass(), "every pool asked in the last pass");
    }

    #[test]
    fn the_slots_are_watched_only_while_a_pool_waits_and_no_start_runs() {
        let lane = lane();
        lane.begin_pass();
        lane.note_line_wait();
        assert!(!lane.watching_slots(), "published when the pass ends");
        lane.end_pass();
        assert!(lane.watching_slots());
        let claim = lane.claim();
        assert!(!lane.watching_slots(), "the start's end wakes the pass");
        drop(claim);
        lane.begin_pass();
        lane.end_pass();
        assert!(!lane.watching_slots(), "nobody waits any more");
    }

    #[test]
    fn a_slot_frees_when_fewer_are_in_use_or_more_are_free() -> Result<()> {
        let view = |in_use: u64, free: u64| {
            SlotView::from_report(&json!({"inUse": in_use, "free": free})).context("a report")
        };
        let full = view(2, 0)?;
        assert!(slot_freed(full, view(1, 1)?), "a VM stopped");
        assert!(slot_freed(view(1, 0)?, view(1, 1)?), "the limit was raised");
        assert!(!slot_freed(full, full));
        assert!(
            !slot_freed(view(1, 1)?, view(1, 1)?),
            "free all along: the head waits for budget, not a slot"
        );
        assert!(!slot_freed(view(1, 1)?, full));
        assert_eq!(SlotView::from_report(&Value::Null), None);
        Ok(())
    }

    #[test]
    fn the_first_look_wakes_the_pass_for_a_slot_already_free() -> Result<()> {
        let view = |in_use: u64, free: u64| {
            SlotView::from_report(&json!({"inUse": in_use, "free": free})).context("a report")
        };
        assert!(
            wake_for_slots(None, view(1, 1)?),
            "freed before the first look"
        );
        assert!(!wake_for_slots(None, view(2, 0)?));
        assert!(
            !wake_for_slots(Some(view(1, 1)?), view(1, 1)?),
            "after the first look, only a change wakes it"
        );
        assert!(wake_for_slots(Some(view(2, 0)?), view(1, 1)?));
        Ok(())
    }
}
