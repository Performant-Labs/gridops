# Handoff-A-dup: Phase 7 - Prune fleet_inventory_snapshots (anti-duplication gate)

**Date:** 2026-11-22
**Branch:** claude/fix-152-prune-snapshots
**Diff base:** 9e357e0686656cbae0be6e1b2bea8747f1407f44   **Diff head:** a3da78b3345b718ecc9104cb662fae011cba2694
**Reuse map:** docs/handoffs/152-brief.md (extend-vs-new section) and handoff-A.md
**Verdict:** PASS

## Summary
F extended the objects the brief named: `cleanup_retention` and `setting_i64` in the reconciler, `SystemSettings`, `settings` and `save_settings` in the API (following the `macos_vm_slots` optional-field precedent), and the migration sequence in the 0034 style. The one new object, `fleet::registry::snapshot_retention`, is justified in writing by the brief (not `inventory.rs`, which is the private transactional ingest path) and follows the `registry::logs` and `fleet::targets` free-fn placement. No parallel path found.

## Findings
| # | Severity | File:line | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | warn | `crates/gridops-core/src/fleet/registry/snapshot_retention.rs:23-27` | Key, default and range live as core consts shared by the API and reconciler, where older retention settings repeat the literals in each crate. This follows the `tart_host::{VM_SLOTS_KEY, ...}` precedent and removes a drift risk, so it is not duplication. Noted only because the two styles now coexist. | None needed. |
| 2 | warn | `crates/gridops-reconciler/src/main.rs` (`prune_inventory_snapshots`) | The reconciler wrapper shares its name with the core fn it calls. Resolved by the `use ... as retention` alias, but a reader grepping the name finds two. | Optional rename (e.g. `prune_snapshots_step`). |

No other existing helper or table-pruning path was bypassed. `inventory.rs` and `queries.rs` are untouched. The reconciler and API changes are additive hooks into existing functions. The scope matches the brief (no `webhook_deliveries` or other retention changes, no UI change).

No architectural drift: layering holds (API to core, reconciler to core), no new dependencies, and the migration is numbered next in sequence with a 0034-style header.

## Notes for F
None; PASS.
