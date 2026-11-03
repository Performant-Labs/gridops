//! The Tart host's macOS VM slots (#95).
//!
//! An Apple-silicon Mac runs at most two macOS VMs at once, whoever starts
//! them. `GridOps` owns that limit: it is part of the Tart host's budget, set
//! in `GridOps` and sent with the manager policy, and this module enforces it
//! at admission, before the Tart agent is asked for a VM.
//!
//! Slots in use are the Tart agent's count of `GridOps` VMs that are running,
//! the VMs other software runs that the agent can see, and admissions that
//! are starting a VM now. A request that finds no slot waits in one queue for
//! the host, shared by every pool: the slot that frees next goes to the pool
//! that has waited longest, whichever pool happens to ask first.

use std::{
    fmt::Write as _,
    sync::atomic::{AtomicUsize, Ordering},
};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ManagerError;

/// Slots a Mac allows when `GridOps` has not said otherwise.
pub(crate) const DEFAULT_VM_SLOTS: usize = 2;
/// macOS allows no more than two macOS VMs at once.
pub(crate) const MAX_VM_SLOTS: usize = 2;

/// What the Tart agent's health reports under `capacity.vmSlots`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSlotUsage {
    /// Slots `GridOps`'s own VMs hold.
    pub(crate) gridops: usize,
    /// Running VMs that other software started.
    #[serde(default)]
    pub(crate) external: Vec<ExternalVm>,
    /// How the agent looked: `processes` or `tart-list`. Absent from agents
    /// that predate slot reporting.
    #[serde(default)]
    pub(crate) detection: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct ExternalVm {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) user: Option<String>,
    pub(crate) source: String,
}

/// Whether a request may have a slot now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotDecision {
    Admit,
    /// Refused for now: `position` (from 1) of `waiting` requests in line.
    Wait {
        position: usize,
        waiting: usize,
    },
}

#[derive(Clone, Debug)]
struct Waiter {
    pool_id: String,
    /// The reconciliation round in which this pool last asked.
    last_round: u64,
    since: DateTime<Utc>,
}

/// One line for the host's slots, ordered by when each pool began waiting.
///
/// The reconciler asks for every pool that needs a runner on every pass, and
/// says which pass it is. A pool that missed a whole pass has stopped asking
/// (its job ran elsewhere or was cancelled, or the pool was paused), so it
/// leaves the line rather than holding a slot nobody takes. Rounds count the
/// passes seen here; they never compare clocks.
#[derive(Debug, Default)]
pub(crate) struct SlotQueue {
    round: u64,
    last_pass: Option<i64>,
    waiters: Vec<Waiter>,
}

impl SlotQueue {
    /// Decides one request from `pool_id`, with `free` slots unclaimed now.
    /// `pass` is the reconciler pass asking; a request without one (a start
    /// from the web app) is admitted only when it would not take a slot from
    /// a pool already waiting, and does not join the line.
    pub(crate) fn decide(
        &mut self,
        pool_id: &str,
        pass: Option<i64>,
        free: usize,
        now: DateTime<Utc>,
    ) -> SlotDecision {
        let Some(pass) = pass else {
            let waiting = self.waiters.len();
            return if free > waiting {
                SlotDecision::Admit
            } else {
                SlotDecision::Wait {
                    position: waiting + 1,
                    waiting: waiting + 1,
                }
            };
        };
        if self.last_pass != Some(pass) {
            self.last_pass = Some(pass);
            self.round += 1;
        }
        let round = self.round;
        self.waiters
            .retain(|waiter| round.saturating_sub(waiter.last_round) < 2);
        let index = if let Some(index) = self
            .waiters
            .iter()
            .position(|waiter| waiter.pool_id == pool_id)
        {
            index
        } else {
            self.waiters.push(Waiter {
                pool_id: pool_id.to_owned(),
                last_round: round,
                since: now,
            });
            self.waiters.len() - 1
        };
        self.waiters[index].last_round = round;
        if index < free {
            self.waiters.remove(index);
            SlotDecision::Admit
        } else {
            SlotDecision::Wait {
                position: index + 1,
                waiting: self.waiters.len(),
            }
        }
    }

    /// The line, oldest first, as `(pool, waiting since)`.
    pub(crate) fn waiting(&self) -> Vec<(String, DateTime<Utc>)> {
        self.waiters
            .iter()
            .map(|waiter| (waiter.pool_id.clone(), waiter.since))
            .collect()
    }
}

/// How many more runners of the size asked for `GridOps`'s own budget on the
/// Tart host (its CPU, memory and runner count) has room for, and why there
/// is none when it is full.
#[derive(Clone, Debug)]
pub(crate) struct BudgetRoom {
    pub(crate) room: usize,
    pub(crate) refusal: Option<String>,
}

impl BudgetRoom {
    #[cfg(test)]
    pub(crate) fn unlimited() -> Self {
        Self {
            room: usize::MAX,
            refusal: None,
        }
    }
}

/// The host's slot limit and its line.
#[derive(Debug)]
pub(crate) struct VmSlots {
    limit: AtomicUsize,
    queue: std::sync::Mutex<SlotQueue>,
}

impl Default for VmSlots {
    fn default() -> Self {
        Self {
            limit: AtomicUsize::new(DEFAULT_VM_SLOTS),
            queue: std::sync::Mutex::default(),
        }
    }
}

/// Slots in use: `GridOps` VMs, other software's VMs, starts in flight.
pub(crate) fn slots_in_use(usage: &AgentSlotUsage, starting: usize) -> usize {
    usage
        .gridops
        .saturating_add(usage.external.len())
        .saturating_add(starting)
}

impl VmSlots {
    pub(crate) fn limit(&self) -> usize {
        self.limit.load(Ordering::Relaxed)
    }

    /// Sets the limit `GridOps` sent; refuses one the Mac cannot have.
    pub(crate) fn set_limit(&self, limit: usize) -> Result<(), ManagerError> {
        if !(1..=MAX_VM_SLOTS).contains(&limit) {
            return Err(ManagerError::BadRequest(format!(
                "The Tart host's macOS VM slots must be 1 to {MAX_VM_SLOTS}; macOS runs at most {MAX_VM_SLOTS} macOS VMs at once."
            )));
        }
        self.limit.store(limit, Ordering::Relaxed);
        Ok(())
    }

    fn queue(&self) -> std::sync::MutexGuard<'_, SlotQueue> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Admits one VM start for `pool_id`, or refuses it with its place in
    /// line. `starting` counts admissions already holding a slot for a VM
    /// being started; `budget` is the room `GridOps`'s own Tart budget has.
    /// Both wait in the one line: a start goes ahead only when the pool is
    /// among the first in line that the free slots and the budget can take.
    /// Refused for lack of a slot, the code is `vm_slot_busy` and the message
    /// says who holds the slots; for lack of budget, `host_capacity_exhausted`.
    /// The caller holds the reservation lock, so the check and the
    /// reservation that follows are one step.
    pub(crate) fn admit(
        &self,
        pool_id: &str,
        pass: Option<i64>,
        usage: &AgentSlotUsage,
        starting: usize,
        budget: &BudgetRoom,
    ) -> Result<(), ManagerError> {
        let limit = self.limit();
        let in_use = slots_in_use(usage, starting);
        let free_slots = limit.saturating_sub(in_use);
        match self
            .queue()
            .decide(pool_id, pass, free_slots.min(budget.room), Utc::now())
        {
            SlotDecision::Admit => Ok(()),
            SlotDecision::Wait { position, waiting } if free_slots >= position => {
                Err(ManagerError::Guardrail {
                    code: "host_capacity_exhausted",
                    message: budget_waiting_message(position, waiting, budget),
                })
            }
            SlotDecision::Wait { position, waiting } => Err(ManagerError::Guardrail {
                code: "vm_slot_busy",
                message: waiting_message(position, waiting, limit, in_use, usage, starting),
            }),
        }
    }

    /// The slots as the manager's health reports them. `agent_max_runners`
    /// is the Tart agent's own cap on `GridOps` runners (`maxRunners` in its
    /// health); a cap below the slot limit is reported as a warning.
    pub(crate) fn report(
        &self,
        usage: &AgentSlotUsage,
        starting: usize,
        agent_max_runners: Option<usize>,
    ) -> Value {
        let limit = self.limit();
        let in_use = slots_in_use(usage, starting);
        let queue = self
            .queue()
            .waiting()
            .into_iter()
            .enumerate()
            .map(|(index, (pool_id, since))| {
                json!({
                    "poolId": pool_id,
                    "position": index + 1,
                    "waitingSince": since.to_rfc3339_opts(SecondsFormat::Secs, true),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "limit": limit,
            "inUse": in_use,
            "free": limit.saturating_sub(in_use),
            "gridops": usage.gridops,
            "starting": starting,
            "external": usage.external,
            "detection": usage.detection,
            "queue": queue,
            "agentMaxRunners": agent_max_runners,
            "warning": agent_cap_warning(limit, agent_max_runners),
        })
    }
}

/// Says so when the Tart agent's own cap on `GridOps` runners
/// (`GRIDOPS_TART_MAX_RUNNERS`) is below the slot limit. The lower of the two
/// wins: the manager would admit a start that the agent then turns away, so
/// `GridOps` uses fewer slots than it is set to. A cap at or above the limit
/// needs no warning, since the limit governs then.
pub(crate) fn agent_cap_warning(limit: usize, agent_max_runners: Option<usize>) -> Option<String> {
    let cap = agent_max_runners?;
    (cap < limit).then(|| {
        format!(
            "The Tart agent allows {cap} runner{} (GRIDOPS_TART_MAX_RUNNERS); macOS VM slots are set to {limit}, so GridOps uses at most {cap} of them. Raise GRIDOPS_TART_MAX_RUNNERS to {limit} and run the Tart agent installer again, or set the slots to {cap}.",
            if cap == 1 { "" } else { "s" }
        )
    })
}

/// Why a start waits for room in `GridOps`'s own Tart budget.
fn budget_waiting_message(position: usize, waiting: usize, budget: &BudgetRoom) -> String {
    let why = budget.refusal.clone().unwrap_or_else(|| {
        format!(
            "GridOps's own budget there has room for {} more runner{}.",
            budget.room,
            if budget.room == 1 { "" } else { "s" }
        )
    });
    format!(
        "Waiting for room on the Tart host: position {position} of {waiting} in the queue. {why}"
    )
}

/// Why a start waits, said so an operator can see where it stands and who
/// holds the slots.
fn waiting_message(
    position: usize,
    waiting: usize,
    limit: usize,
    in_use: usize,
    usage: &AgentSlotUsage,
    starting: usize,
) -> String {
    let mut message = format!(
        "Waiting for a macOS VM slot: position {position} of {waiting} in the queue. {} of {limit} slots in use (GridOps runners: {}; other software: {}",
        in_use.min(limit),
        usage.gridops + starting,
        usage.external.len(),
    );
    if !usage.external.is_empty() {
        let names = usage
            .external
            .iter()
            .take(3)
            .map(|vm| match &vm.user {
                Some(user) => format!("{} ({user})", vm.name),
                None => vm.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        message.push_str(" — ");
        message.push_str(&names);
        if usage.external.len() > 3 {
            let _ = write!(message, " and {} more", usage.external.len() - 3);
        }
    }
    message.push_str(").");
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(gridops: usize, external: &[(&str, Option<&str>)]) -> AgentSlotUsage {
        AgentSlotUsage {
            gridops,
            external: external
                .iter()
                .map(|(name, user)| ExternalVm {
                    name: (*name).to_owned(),
                    user: user.map(ToOwned::to_owned),
                    source: "tart-process".into(),
                })
                .collect(),
            detection: Some("processes".into()),
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn code(result: Result<(), ManagerError>) -> Option<&'static str> {
        match result {
            Ok(()) => None,
            Err(ManagerError::Guardrail { code, .. }) => Some(code),
            Err(_) => Some("other"),
        }
    }

    fn message(result: Result<(), ManagerError>) -> String {
        match result {
            Err(ManagerError::Guardrail { message, .. }) => message,
            _ => String::new(),
        }
    }

    // ---- The limit ----

    #[test]
    fn the_limit_defaults_to_two_and_gridops_can_lower_it_but_not_raise_it() {
        let slots = VmSlots::default();
        assert_eq!(slots.limit(), 2);
        assert!(slots.set_limit(1).is_ok());
        assert_eq!(slots.limit(), 1);
        assert!(slots.set_limit(0).is_err());
        assert!(slots.set_limit(3).is_err());
        assert_eq!(slots.limit(), 1, "a refused limit changes nothing");
    }

    #[test]
    fn slots_in_use_count_gridops_vms_other_softwares_vms_and_starts_in_flight() {
        assert_eq!(slots_in_use(&usage(0, &[]), 0), 0);
        assert_eq!(slots_in_use(&usage(1, &[]), 0), 1);
        assert_eq!(slots_in_use(&usage(0, &[("warm-1", Some("ci"))]), 0), 1);
        assert_eq!(slots_in_use(&usage(1, &[("warm-1", None)]), 1), 3);
    }

    #[test]
    fn a_start_is_admitted_only_while_a_slot_is_free() {
        let slots = VmSlots::default();
        assert_eq!(
            code(slots.admit(
                "pool-a",
                Some(1),
                &usage(1, &[]),
                0,
                &BudgetRoom::unlimited()
            )),
            None
        );
        // Both slots taken, in each way they can be.
        for (usage, starting) in [
            (usage(2, &[]), 0),
            (usage(1, &[]), 1),
            (
                usage(0, &[("warm-1", Some("ci")), ("warm-2", Some("ci"))]),
                0,
            ),
            (usage(1, &[("warm-1", Some("ci"))]), 0),
        ] {
            assert_eq!(
                code(slots.admit(
                    "pool-b",
                    Some(1),
                    &usage,
                    starting,
                    &BudgetRoom::unlimited()
                )),
                Some("vm_slot_busy"),
                "{usage:?} starting {starting}"
            );
        }
    }

    #[test]
    fn a_lower_limit_is_enforced() {
        let slots = VmSlots::default();
        assert!(slots.set_limit(1).is_ok());
        assert_eq!(
            code(slots.admit(
                "pool-a",
                Some(1),
                &usage(1, &[]),
                0,
                &BudgetRoom::unlimited()
            )),
            Some("vm_slot_busy")
        );
    }

    #[test]
    fn a_refusal_says_where_the_start_stands_and_who_holds_the_slots() {
        let slots = VmSlots::default();
        let refused = message(slots.admit(
            "pool-a",
            Some(1),
            &usage(1, &[("runner-warm-1", Some("ci"))]),
            0,
            &BudgetRoom::unlimited(),
        ));
        assert_eq!(
            refused,
            "Waiting for a macOS VM slot: position 1 of 1 in the queue. 2 of 2 slots in use (GridOps runners: 1; other software: 1 — runner-warm-1 (ci))."
        );
        let refused = message(slots.admit(
            "pool-b",
            Some(1),
            &usage(2, &[]),
            0,
            &BudgetRoom::unlimited(),
        ));
        assert!(
            refused.starts_with("Waiting for a macOS VM slot: position 2 of 2 in the queue."),
            "{refused}"
        );
    }

    /// `GridOps`'s own budget on the Mac (`GRIDOPS_TART_MAX_RUNNERS`, CPU,
    /// memory) can be full while a slot is free. Such a wait keeps its place
    /// in the same line, so the pool asked first cannot take the room that
    /// frees next from a pool that has waited longer.
    #[test]
    fn a_full_tart_budget_waits_in_the_same_line_as_the_slots() {
        let slots = VmSlots::default();
        let full = || BudgetRoom {
            room: 0,
            refusal: Some("Host runner limit reached (2/1 including reservations).".into()),
        };
        let refused = slots.admit("pool-b", Some(1), &usage(1, &[]), 0, &full());
        assert_eq!(code(refused), Some("host_capacity_exhausted"));
        let refused = message(slots.admit("pool-b", Some(1), &usage(1, &[]), 0, &full()));
        assert_eq!(
            refused,
            "Waiting for room on the Tart host: position 1 of 1 in the queue. Host runner limit reached (2/1 including reservations)."
        );
        // The one free slot is held for B, so A waits for a slot.
        assert_eq!(
            code(slots.admit("pool-a", Some(1), &usage(1, &[]), 0, &full())),
            Some("vm_slot_busy")
        );
        // The running VM ends: room for one, and both slots free. A asks first.
        let one = || BudgetRoom {
            room: 1,
            refusal: None,
        };
        let refused = message(slots.admit("pool-a", Some(2), &usage(0, &[]), 0, &one()));
        assert!(
            refused.starts_with("Waiting for room on the Tart host: position 2 of 2 in the queue. GridOps's own budget there has room for 1 more runner."),
            "{refused}"
        );
        assert_eq!(
            code(slots.admit("pool-b", Some(2), &usage(0, &[]), 0, &one())),
            None
        );
    }

    // ---- The line ----

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn a_freed_slot_goes_to_the_pool_that_waited_longest_whatever_order_pools_ask_in() {
        let mut queue = SlotQueue::default();
        // Pass 1: no slot. A asks first, then B.
        assert!(matches!(
            queue.decide("pool-a", Some(1), 0, now()),
            SlotDecision::Wait { position: 1, .. }
        ));
        assert!(matches!(
            queue.decide("pool-b", Some(1), 0, now()),
            SlotDecision::Wait { position: 2, .. }
        ));
        // Pass 2: one slot frees. B happens to ask first; it is A's.
        assert_eq!(
            queue.decide("pool-b", Some(2), 1, now()),
            SlotDecision::Wait {
                position: 2,
                waiting: 2
            }
        );
        assert_eq!(
            queue.decide("pool-a", Some(2), 1, now()),
            SlotDecision::Admit
        );
        // A is out of the line; B is at its head.
        assert_eq!(
            queue
                .waiting()
                .into_iter()
                .map(|(pool, _)| pool)
                .collect::<Vec<_>>(),
            ["pool-b"]
        );
    }

    /// Two pools that always have a queued macOS job, asked in the same
    /// order every pass (the reconciler visits pools oldest first), on a Mac
    /// where one slot frees per pass. Without the line, the pool asked first
    /// would take every slot and the other would never run.
    #[test]
    fn two_pools_that_always_want_a_slot_take_turns_and_neither_starves() {
        let mut queue = SlotQueue::default();
        let mut admitted = Vec::new();
        for pass in 1..=20 {
            let mut free = 1;
            for pool in ["pool-a", "pool-b"] {
                if queue.decide(pool, Some(pass), free, now()) == SlotDecision::Admit {
                    admitted.push(pool);
                    free -= 1;
                }
            }
        }
        assert_eq!(admitted.len(), 20, "every freed slot is used: {admitted:?}");
        let a = admitted.iter().filter(|pool| **pool == "pool-a").count();
        assert_eq!((a, 20 - a), (10, 10), "{admitted:?}");
        assert!(
            admitted.windows(2).all(|pair| pair[0] != pair[1]),
            "the pools alternate: {admitted:?}"
        );
    }

    #[test]
    fn three_pools_are_served_in_the_order_they_began_waiting() {
        let mut queue = SlotQueue::default();
        for pool in ["pool-c", "pool-a", "pool-b"] {
            queue.decide(pool, Some(1), 0, now());
        }
        let mut served = Vec::new();
        for pass in 2..=4 {
            // Asked in another order each pass; one slot frees each time.
            let mut free = 1;
            for pool in ["pool-a", "pool-b", "pool-c"] {
                if queue.decide(pool, Some(pass), free, now()) == SlotDecision::Admit {
                    served.push(pool);
                    free -= 1;
                }
            }
        }
        assert_eq!(served[..3], ["pool-c", "pool-a", "pool-b"]);
    }

    #[test]
    fn a_pool_that_stops_asking_leaves_the_line_after_missing_a_whole_pass() {
        let mut queue = SlotQueue::default();
        queue.decide("pool-a", Some(1), 0, now());
        queue.decide("pool-b", Some(1), 0, now());
        // Pass 2: A's job was cancelled, so A does not ask. B is not at the
        // head and A may yet ask later in this pass, so B waits.
        assert!(matches!(
            queue.decide("pool-b", Some(2), 1, now()),
            SlotDecision::Wait { position: 2, .. }
        ));
        // Pass 3: A missed all of pass 2 and is gone; the slot is B's.
        assert_eq!(
            queue.decide("pool-b", Some(3), 1, now()),
            SlotDecision::Admit
        );
        assert!(queue.waiting().is_empty());
    }

    #[test]
    fn a_pool_asking_again_in_the_same_pass_keeps_its_place() {
        let mut queue = SlotQueue::default();
        queue.decide("pool-a", Some(1), 0, now());
        queue.decide("pool-b", Some(1), 0, now());
        queue.decide("pool-a", Some(1), 0, now());
        assert_eq!(
            queue
                .waiting()
                .into_iter()
                .map(|(pool, _)| pool)
                .collect::<Vec<_>>(),
            ["pool-a", "pool-b"]
        );
    }

    #[test]
    fn two_free_slots_serve_the_first_two_in_line() {
        let mut queue = SlotQueue::default();
        for pool in ["pool-a", "pool-b", "pool-c"] {
            queue.decide(pool, Some(1), 0, now());
        }
        // Two slots free; C asks first but is third in line.
        assert!(matches!(
            queue.decide("pool-c", Some(2), 2, now()),
            SlotDecision::Wait { position: 3, .. }
        ));
        assert_eq!(
            queue.decide("pool-b", Some(2), 2, now()),
            SlotDecision::Admit
        );
        // One slot left, A at the head.
        assert_eq!(
            queue.decide("pool-a", Some(2), 1, now()),
            SlotDecision::Admit
        );
    }

    #[test]
    fn a_start_from_the_web_app_does_not_take_a_slot_a_pool_is_waiting_for() {
        let mut queue = SlotQueue::default();
        assert_eq!(queue.decide("pool-a", None, 1, now()), SlotDecision::Admit);
        assert!(queue.waiting().is_empty(), "it does not join the line");
        queue.decide("pool-b", Some(1), 0, now());
        assert_eq!(
            queue.decide("pool-a", None, 1, now()),
            SlotDecision::Wait {
                position: 2,
                waiting: 2
            }
        );
        assert_eq!(queue.decide("pool-a", None, 2, now()), SlotDecision::Admit);
    }

    #[test]
    fn the_health_report_shows_the_slots_and_the_line() {
        let slots = VmSlots::default();
        let _ = slots.admit(
            "pool-a",
            Some(1),
            &usage(2, &[]),
            0,
            &BudgetRoom::unlimited(),
        );
        let report = slots.report(&usage(1, &[("warm-1", Some("ci"))]), 0, Some(2));
        assert_eq!(report["limit"], 2);
        assert_eq!(report["inUse"], 2);
        assert_eq!(report["free"], 0);
        assert_eq!(report["external"][0]["name"], "warm-1");
        assert_eq!(report["queue"][0]["poolId"], "pool-a");
        assert_eq!(report["queue"][0]["position"], 1);
        assert_eq!(report["agentMaxRunners"], 2);
        assert!(report["warning"].is_null(), "{report}");
    }

    #[test]
    fn an_agent_cap_below_the_slot_limit_is_reported_and_one_at_or_above_it_is_not() {
        let slots = VmSlots::default();
        assert_eq!(
            agent_cap_warning(2, Some(1)).as_deref(),
            Some(
                "The Tart agent allows 1 runner (GRIDOPS_TART_MAX_RUNNERS); macOS VM slots are set to 2, so GridOps uses at most 1 of them. Raise GRIDOPS_TART_MAX_RUNNERS to 2 and run the Tart agent installer again, or set the slots to 1."
            )
        );
        let report = slots.report(&usage(0, &[]), 0, Some(1));
        assert_eq!(report["agentMaxRunners"], 1);
        assert_eq!(report["warning"], json!(agent_cap_warning(2, Some(1))));
        for cap in [Some(2), Some(16)] {
            assert_eq!(agent_cap_warning(2, cap), None, "{cap:?}");
        }
        assert!(slots.set_limit(1).is_ok(), "one slot is a valid limit");
        assert_eq!(agent_cap_warning(1, Some(1)), None);
        assert!(slots.report(&usage(0, &[]), 0, Some(1))["warning"].is_null());
        // Nothing to compare with when the agent's cap is unknown.
        assert_eq!(agent_cap_warning(2, None), None);
    }

    // ---- Through the manager's admission route, against a stand-in agent ----

    mod admission {
        use std::{
            collections::HashMap,
            sync::{Arc, atomic::AtomicBool},
        };

        use axum::{Json, Router, routing::get};
        use secrecy::SecretString;
        use serde_json::{Value, json};
        use tokio::sync::Mutex;

        use crate::{ManagerLimits, ManagerState, TartAgent, builtin_host::BuiltinHost};

        const TOKEN: &str = "vm-slot-test-token";

        /// The real manager router with a Tart agent whose health reports
        /// `slots` (`capacity.vmSlots`) and a budget roomy enough that only
        /// the slot limit can refuse.
        struct TestManager {
            base_url: String,
            agent_slots: Arc<std::sync::Mutex<Value>>,
            /// `(maxRunners, activeRunners)` in the agent's budget.
            agent_runners: Arc<std::sync::Mutex<(usize, usize)>>,
            socket: std::path::PathBuf,
        }

        impl TestManager {
            async fn start(slots: Value) -> anyhow::Result<Self> {
                let agent_slots = Arc::new(std::sync::Mutex::new(slots));
                let agent_runners = Arc::new(std::sync::Mutex::new((16_usize, 0_usize)));
                let agent = Router::new().route(
                    "/v1/health",
                    get({
                        let agent_slots = Arc::clone(&agent_slots);
                        let agent_runners = Arc::clone(&agent_runners);
                        move || {
                            let slots = agent_slots
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .clone();
                            let (max_runners, active) = *agent_runners
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            async move {
                                let mut capacity = json!({
                                    "cpuBudget": 64.0, "memoryBudgetMb": 262_144, "maxRunners": max_runners,
                                    "active": {"activeRunners": active, "cpu": 2.0 * active as f64, "memoryMb": 4096 * active},
                                });
                                if !slots.is_null() {
                                    capacity["vmSlots"] = slots;
                                }
                                Json(json!({"status": "ok", "provider": "tart", "capacity": capacity}))
                            }
                        }
                    }),
                );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                let agent_url = format!("http://{}/", listener.local_addr()?);
                tokio::spawn(async move { axum::serve(listener, agent).await });

                let socket = std::env::temp_dir().join(format!(
                    "gridops-vm-slot-test-{}.sock",
                    uuid::Uuid::new_v4()
                ));
                std::fs::write(&socket, "")?;
                let state = ManagerState {
                    docker: bollard::Docker::connect_with_socket(
                        &socket.to_string_lossy(),
                        2,
                        bollard::API_DEFAULT_VERSION,
                    )?,
                    tart: Some(TartAgent {
                        base_url: url::Url::parse(&agent_url)?,
                        token: SecretString::from("agent-token"),
                        http: reqwest::Client::new(),
                    }),
                    token: SecretString::from(TOKEN),
                    limits: ManagerLimits {
                        host_name: "docker-host".into(),
                        available_cpus: 8.0,
                        total_memory_mb: 16_384,
                        cpu_budget: 8.0,
                        memory_budget_mb: 16_384,
                        max_runners: 4,
                        min_free_disk_mb: 1_024,
                        min_free_disk_percent: 10,
                        runner_network: "gridops-runners".into(),
                        log_max_size: "20m".into(),
                        log_max_files: 5,
                        runner_pids_limit: 1_024,
                        runner_swap_percent: 100,
                        budget_origin: crate::BudgetOrigin::default(),
                    },
                    shared_docker_socket: None,
                    reservations: Arc::new(Mutex::new(HashMap::new())),
                    provisioning_paused: Arc::new(AtomicBool::new(false)),
                    builtin_host: BuiltinHost::new(false, Some(true)),
                    provision_lock: Arc::new(Mutex::new(())),
                    vm_slots: Arc::default(),
                };
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                let base_url = format!("http://{}", listener.local_addr()?);
                let app = crate::router(state);
                tokio::spawn(async move { axum::serve(listener, app).await });
                Ok(Self {
                    base_url,
                    agent_slots,
                    agent_runners,
                    socket,
                })
            }

            fn set_agent_runners(&self, max_runners: usize, active: usize) {
                *self
                    .agent_runners
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = (max_runners, active);
            }

            fn set_agent_slots(&self, slots: Value) {
                *self
                    .agent_slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = slots;
            }

            fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
                reqwest::Client::new()
                    .request(method, format!("{}{path}", self.base_url))
                    .bearer_auth(TOKEN)
            }

            /// Asks for a Tart VM runner for `pool`; answers the status, the
            /// error code (empty when admitted) and the message.
            async fn admit(
                &self,
                pool: &str,
                runtime: Option<&str>,
                pass: Option<i64>,
            ) -> anyhow::Result<(u16, String, String)> {
                let mut body = json!({
                    "runnerId": uuid::Uuid::new_v4().to_string(), "poolId": pool,
                    "provider": "tart", "cpuLimit": 2.0, "memoryLimitMb": 4096,
                });
                if let Some(runtime) = runtime {
                    body["runtime"] = json!(runtime);
                }
                if let Some(pass) = pass {
                    body["reconcilePass"] = json!(pass);
                }
                let response = self
                    .request(reqwest::Method::POST, "/v1/admissions")
                    .json(&body)
                    .send()
                    .await?;
                let status = response.status().as_u16();
                let body = response.json::<Value>().await.unwrap_or(Value::Null);
                Ok((
                    status,
                    body["code"].as_str().unwrap_or_default().to_owned(),
                    body["error"].as_str().unwrap_or_default().to_owned(),
                ))
            }
        }

        impl Drop for TestManager {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.socket);
            }
        }

        fn slots(gridops: usize, external: &[&str]) -> Value {
            json!({
                "gridops": gridops,
                "external": external.iter().map(|name| json!({"name": name, "user": "ci", "source": "tart-process"})).collect::<Vec<_>>(),
                "detection": "processes",
            })
        }

        #[tokio::test]
        async fn two_pools_racing_for_the_last_slot_get_one_lease_between_them()
        -> anyhow::Result<()> {
            for _ in 0..10 {
                let manager = TestManager::start(slots(1, &[])).await?;
                let (a, b) = tokio::join!(
                    manager.admit("pool-a", Some("vm"), Some(1)),
                    manager.admit("pool-b", Some("vm"), Some(1)),
                );
                let mut outcomes = [a?, b?]
                    .into_iter()
                    .map(|(status, code, _)| (status, code))
                    .collect::<Vec<_>>();
                outcomes.sort();
                assert_eq!(
                    outcomes,
                    [(201, String::new()), (429, "vm_slot_busy".to_owned())]
                );
            }
            Ok(())
        }

        #[tokio::test]
        async fn many_concurrent_starts_never_hold_more_slots_than_the_mac_has()
        -> anyhow::Result<()> {
            let manager = TestManager::start(slots(0, &[])).await?;
            let starts = (0..8)
                .map(|index| {
                    manager.admit(if index % 2 == 0 { "pool-a" } else { "pool-b" }, None, None)
                })
                .collect::<Vec<_>>();
            let admitted = futures_util::future::join_all(starts)
                .await
                .into_iter()
                .filter(|outcome| matches!(outcome, Ok((201, _, _))))
                .count();
            assert_eq!(admitted, 2);
            Ok(())
        }

        #[tokio::test]
        async fn vms_other_software_runs_take_slots_and_are_named_in_the_reason()
        -> anyhow::Result<()> {
            let manager = TestManager::start(slots(0, &["runner-warm-1", "runner-warm-2"])).await?;
            let (status, code, message) = manager.admit("pool-a", Some("vm"), Some(1)).await?;
            assert_eq!((status, code.as_str()), (429, "vm_slot_busy"));
            assert!(
                message.contains("other software: 2 — runner-warm-1 (ci), runner-warm-2 (ci)"),
                "{message}"
            );

            // One of them stops: the slot is GridOps's to give.
            manager.set_agent_slots(slots(0, &["runner-warm-1"]));
            assert_eq!(manager.admit("pool-a", Some("vm"), Some(2)).await?.0, 201);
            Ok(())
        }

        #[tokio::test]
        async fn an_agent_that_does_not_report_slots_is_still_held_to_the_limit()
        -> anyhow::Result<()> {
            // An agent that predates slot reporting: its running runners are
            // taken to be VMs.
            let manager = TestManager::start(Value::Null).await?;
            assert_eq!(manager.admit("pool-a", None, Some(1)).await?.0, 201);
            assert_eq!(manager.admit("pool-a", None, Some(1)).await?.0, 201);
            assert_eq!(
                manager.admit("pool-b", None, Some(1)).await?.1,
                "vm_slot_busy"
            );
            Ok(())
        }

        #[tokio::test]
        async fn native_runners_take_no_vm_slot() -> anyhow::Result<()> {
            let manager = TestManager::start(slots(2, &[])).await?;
            assert_eq!(
                manager.admit("pool-a", Some("vm"), Some(1)).await?.1,
                "vm_slot_busy"
            );
            assert_eq!(
                manager.admit("pool-a", Some("native"), Some(1)).await?.0,
                201
            );
            Ok(())
        }

        #[tokio::test]
        async fn the_policy_sets_the_limit_and_health_shows_the_slots_and_the_line()
        -> anyhow::Result<()> {
            let manager = TestManager::start(slots(1, &["runner-warm-1"])).await?;
            let policy = |slots: Value| json!({"provisioningPaused": false, "tartVmSlots": slots});
            for refused in [json!(0), json!(3)] {
                let response = manager
                    .request(reqwest::Method::PUT, "/v1/policy")
                    .json(&policy(refused))
                    .send()
                    .await?;
                assert_eq!(response.status().as_u16(), 400);
            }
            let response = manager
                .request(reqwest::Method::PUT, "/v1/policy")
                .json(&policy(json!(1)))
                .send()
                .await?
                .json::<Value>()
                .await?;
            assert_eq!(response["tartVmSlots"], 1);

            manager.set_agent_slots(slots(0, &[]));
            assert_eq!(manager.admit("pool-a", Some("vm"), Some(1)).await?.0, 201);
            assert_eq!(
                manager.admit("pool-b", Some("vm"), Some(1)).await?.1,
                "vm_slot_busy",
                "the lease held for pool-a's VM fills the single slot"
            );

            let health = manager
                .request(reqwest::Method::GET, "/v1/health")
                .send()
                .await?
                .json::<Value>()
                .await?;
            let report = &health["providers"]["tart"]["vmSlots"];
            assert_eq!(report["limit"], 1, "{health}");
            assert_eq!(report["starting"], 1);
            assert_eq!(report["queue"][0]["poolId"], "pool-b");
            assert_eq!(report["agentMaxRunners"], 16);
            assert!(
                report["warning"].is_null(),
                "a higher agent cap is fine: {report}"
            );

            // A policy that says nothing about slots leaves the limit alone.
            let response = manager
                .request(reqwest::Method::PUT, "/v1/policy")
                .json(&json!({"provisioningPaused": false}))
                .send()
                .await?
                .json::<Value>()
                .await?;
            assert_eq!(response["tartVmSlots"], 1);
            Ok(())
        }

        #[tokio::test]
        async fn health_warns_when_the_agents_cap_is_below_the_slot_limit() -> anyhow::Result<()> {
            let manager = TestManager::start(slots(0, &[])).await?;
            let vm_slots = || async {
                let health = manager
                    .request(reqwest::Method::GET, "/v1/health")
                    .send()
                    .await?
                    .json::<Value>()
                    .await?;
                anyhow::Ok(health["providers"]["tart"]["vmSlots"].clone())
            };
            let set_limit = |slots: usize| {
                manager
                    .request(reqwest::Method::PUT, "/v1/policy")
                    .json(&json!({"provisioningPaused": false, "tartVmSlots": slots}))
                    .send()
            };

            // An agent installed with the old default of one runner, against
            // the default two slots: the lower of the two wins, so say so.
            manager.set_agent_runners(1, 0);
            let report = vm_slots().await?;
            assert_eq!(report["limit"], 2, "{report}");
            assert_eq!(report["agentMaxRunners"], 1);
            assert_eq!(
                report["warning"],
                "The Tart agent allows 1 runner (GRIDOPS_TART_MAX_RUNNERS); macOS VM slots are set to 2, so GridOps uses at most 1 of them. Raise GRIDOPS_TART_MAX_RUNNERS to 2 and run the Tart agent installer again, or set the slots to 1."
            );

            // They agree.
            manager.set_agent_runners(2, 0);
            assert!(vm_slots().await?["warning"].is_null());
            set_limit(1).await?.error_for_status()?;
            manager.set_agent_runners(1, 0);
            let report = vm_slots().await?;
            assert_eq!(report["limit"], 1, "{report}");
            assert!(report["warning"].is_null(), "{report}");

            // The agent allows more than the slots: the slot limit governs.
            manager.set_agent_runners(2, 0);
            assert!(vm_slots().await?["warning"].is_null());
            Ok(())
        }

        /// `GridOps`'s own budget on the Mac (`GRIDOPS_TART_MAX_RUNNERS`, 1
        /// here) can be full before both slots are. The pool that asks first
        /// on every pass still cannot take the next runner from the pool that
        /// has waited longer.
        #[tokio::test]
        async fn the_tart_runner_budget_does_not_let_a_later_pool_jump_the_line()
        -> anyhow::Result<()> {
            let manager = TestManager::start(slots(1, &[])).await?;
            manager.set_agent_runners(1, 1);
            // Pass 1: one GridOps VM runs. B asks, then A.
            let (status, code, message) = manager.admit("pool-b", Some("vm"), Some(1)).await?;
            assert_eq!((status, code.as_str()), (429, "host_capacity_exhausted"));
            assert!(
                message.starts_with("Waiting for room on the Tart host: position 1 of 1"),
                "{message}"
            );
            assert_eq!(manager.admit("pool-a", Some("vm"), Some(1)).await?.0, 429);
            // Pass 2: that VM is gone. A happens to ask first.
            manager.set_agent_runners(1, 0);
            manager.set_agent_slots(slots(0, &[]));
            let (_, code, message) = manager.admit("pool-a", Some("vm"), Some(2)).await?;
            assert_eq!(code, "host_capacity_exhausted", "{message}");
            assert!(message.contains("position 2 of 2"), "{message}");
            assert_eq!(manager.admit("pool-b", Some("vm"), Some(2)).await?.0, 201);
            Ok(())
        }
    }
}
