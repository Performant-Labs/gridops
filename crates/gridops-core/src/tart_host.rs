//! The Tart host's budget beyond CPU and memory: its macOS VM slots (#95).
//!
//! An Apple-silicon Mac runs at most two macOS VMs at once, whoever starts
//! them. `GridOps` keeps that limit as a setting, sends it to the runner
//! manager with the policy, and the manager enforces it at admission.

use sqlx::SqlitePool;

/// The settings key for the Tart host's macOS VM slots.
pub const VM_SLOTS_KEY: &str = "macosVmSlots";
/// What a Mac allows when nothing is set.
pub const DEFAULT_VM_SLOTS: i64 = 2;
/// macOS runs no more than two macOS VMs at once.
pub const MAX_VM_SLOTS: i64 = 2;

/// Whether `slots` is a limit a Mac can have.
#[must_use]
pub fn valid_vm_slots(slots: i64) -> bool {
    (1..=MAX_VM_SLOTS).contains(&slots)
}

/// The Tart host's macOS VM slots as set, or the default. A stored value a
/// Mac cannot have reads as the default.
pub async fn vm_slots(database: &SqlitePool) -> i64 {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
        .bind(VM_SLOTS_KEY)
        .fetch_optional(database)
        .await
        .ok()
        .flatten()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|slots| valid_vm_slots(*slots))
        .unwrap_or(DEFAULT_VM_SLOTS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_has_one_or_two_slots() {
        assert!(!valid_vm_slots(0));
        assert!(valid_vm_slots(1));
        assert!(valid_vm_slots(2));
        assert!(!valid_vm_slots(3));
    }
}
