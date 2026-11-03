# Design: GridOps owns a Mac's macOS VM slots

Issue: [#95](https://github.com/Performant-Labs/gridops/issues/95). Related: #86 and #87 (the slot wait in the Tart agent), #94 (the wait does not fit the manager's route timeout), #80 (a release needs a macOS runner).

Status: **phase 1 is implemented** (marked **[phase 1]** below). Everything else is the plan.

## The problem in one paragraph

An Apple-silicon Mac runs at most two macOS VMs at once, whoever starts them. Before this change nothing arbitrated those two slots. The Tart agent counted only its own runners (`GRIDOPS_TART_MAX_RUNNERS`), the manager's admission counted only CPU, memory and the runner count, and the reconciler visits pools in creation order, so on every pass the oldest pool got the first chance at a freed slot. Another client's warm-VM scripts held both slots most of the day, a GridOps start found none, waited in the agent (#87), failed, and three failures opened the pool's provisioning circuit. The #87 wait is a poll, not a queue: whoever retries `tart run` at the right moment wins.

## How runners reach the Mac today

```
reconciler (one process, one pass at a time, pools in creation order)
  └─ provision_on_host ─ POST /v1/admissions ─▶ manager.reserve_capacity
                                                 ├─ provision_lock (tokio Mutex)
                                                 ├─ Tart agent GET /v1/health → budget, active
                                                 ├─ reservations lock: enforce_capacity, insert lease (5 min TTL)
                         POST /v1/runners ──────▶ manager.provision_runner (provision_lock held)
                                                 └─ Tart agent POST /v1/runners: clone, set, boot (#87 wait), start runner
                                                    lease removed when the request ends
web app "Start runner" ─ same two calls (no reconciler pass)
```

The fleet path (enrolled machines, `crates/gridops-core/src/fleet/admission.rs`) runs macOS runners natively, not in VMs, so it takes no VM slot and is not changed. `crates/gridops-core/src/autoscaling.rs` decides how many runners a pool wants; it is not where slots are arbitrated either, because it works per pool and the slots are per host.

The manager's admission is already the one place every Tart start passes through, under one lock. That is where the slot budget belongs.

## 1. The host VM slot budget

**Where it lives [phase 1].** A GridOps setting, `macosVmSlots` in the `settings` table (default 2, allowed 1–2), read by `gridops_core::tart_host::vm_slots`. It travels in the manager policy the reconciler pushes every pass (`PUT /v1/policy`, field `tartVmSlots`), exactly like the built-in host's budget (#72), and the web app pushes it when it is saved. The manager keeps it in `vm_slots::VmSlots` (default 2 until a policy arrives) and refuses a value outside 1–2.

- *Why a GridOps setting and not a Tart agent environment variable:* the issue asks for a first-class limit owned by GridOps, not "an environment variable that only one client reads". The manager is the arbiter; the agent only reports.
- *Why at most 2:* macOS refuses a third macOS VM. Allowing 3 would make GridOps hand out a slot the OS then refuses, and fairness would fall back to the agent's polling race. If Apple raises the limit, `MAX_VM_SLOTS` changes in one place in each of core and manager.
- *Why one number for the whole Mac:* the manager talks to exactly one Tart agent today (`GRIDOPS_TART_AGENT_URL`), so "the Tart host" is one Mac. With several Tart hosts (phase 3) the setting becomes a column of a `tart_hosts` row.
- *Relationship to `GRIDOPS_TART_MAX_RUNNERS`:* that stays the agent's cap on GridOps's own runners (VM and native) and its CPU and memory budget stay too. The slot limit is separate because it counts other software's VMs and excludes native runners.

**API [phase 1].** `GET /api/v1/settings` returns `settings.macosVmSlots`; `PUT /api/v1/settings` accepts an optional `macosVmSlots` (left out, it is unchanged, so older clients keep working; out of range is `400`), audited with `settings.updated`. Pool list and detail gain `provisionWaitReason` (see 4). The manager's health gains `providers.tart.vmSlots`:

```json
{ "limit": 2, "inUse": 2, "free": 0, "gridops": 1, "starting": 0,
  "external": [{ "name": "runner-warm-1", "user": "ci", "source": "tart-process" }],
  "detection": "processes",
  "queue": [{ "poolId": "…", "position": 1, "waitingSince": "2026-11-03T09:00:00Z" }] }
```

**UI [phase 1].** One number field in Settings → General, read-only for members. A per-host card (slots, holders, queue) is phase 2; the operator already sees the reason on the waiting pool's page.

## 2. Central, atomic admission

**[phase 1]** In `reserve_capacity`, for `provider: "tart"` and `runtime` other than `native` (the admission request now says which; a missing runtime counts as a VM, the safe reading):

1. under `provision_lock`, fetch the agent's health once (budget, active runners, `vmSlots`);
2. under the reservations lock, run the existing CPU/memory/runner checks, then the slot check, then insert the lease, all in one critical section;
3. slots in use = agent's `gridops` (GridOps VMs that are running) + `external.len()` + leases with `vm_slot: true` (starts in flight). Free = limit − in use.
4. The queue (3) is asked with the number of starts that may go ahead now: the free slots, capped by the room left in GridOps's own Tart budget for a runner of this size (`GRIDOPS_TART_MAX_RUNNERS`, CPU, memory). If it admits the pool, the lease is created with `vm_slot: true`. Otherwise the answer is `429` with the pool's place in line: `vm_slot_busy` ("Waiting for a macOS VM slot: position p of n in the queue. u of l slots in use (GridOps runners: g; other software: e — name (user), …).") when the slots are what is short, or `host_capacity_exhausted` ("Waiting for room on the Tart host: position p of n in the queue." and the budget it hit) when GridOps's own budget is.

   *Why the budget is in the same line:* the agent's runner cap is 1 by default, so it is usually full before both slots are. Checked outside the queue, the pool the reconciler visits first would take every runner that frees and the other would starve exactly as before; a test pins this (`the_tart_runner_budget_does_not_let_a_later_pool_jump_the_line`).

The VM slot lease is released when `POST /v1/runners` ends (success or failure), as every lease already is; from then on a successful VM is counted by the agent as running. Because the manager also holds `provision_lock` for the whole start, the lease and the running VM are never both visible to another admission.

An agent that predates slot reporting is read as "every active runner is a VM, nobody else's VMs are visible", so the limit still holds for GridOps's own VMs.

Both the reconciler and the web app go through this check, so neither can bypass it. Tests: two pools racing for the last slot get exactly one lease (10 rounds), eight concurrent starts never get more than two; both fail if the check and the insert are split.

## 3. Queue and fairness

**Policy [phase 1]: one FIFO queue per host, one entry per pool, ordered by when the pool began waiting.**

- A request from pool P with `f` free slots is admitted only if P is among the first `f` live entries of the queue. A pool not yet in the queue joins at the back. So a freed slot is held for the head of the queue and nobody else can take it, whichever pool happens to ask first.
- An admitted pool leaves the queue; if it wants another slot it joins at the back. Two pools that always want a slot therefore alternate, and with N pools each waits at most N − 1 grants: **no starvation**, independent of the order the reconciler visits pools.
- **Liveness without clocks.** Every admission from the reconciler carries `reconcilePass` (the pass's start time, used only as an identifier). The manager counts passes as *rounds* when the identifier changes. An entry that did not ask in a whole round (it is two rounds behind) has stopped asking (its job ran elsewhere or was cancelled, or its pool was paused) and is dropped, so a slot is never held for a pool that no longer wants it for more than one pass. No timestamps from different machines are compared, so clock skew does not matter.
- The reconciler does **not** pause a pool for 30 s after `vm_slot_busy`, nor after a Tart `host_capacity_exhausted` (it does for other waits), so a waiting pool asks on every pass and keeps its place. Native Tart runners take no slot and do not wait in the line; they still count against the Tart budget, so a native start can take budget room a waiting VM pool wanted (an accepted edge in phase 1; native and VM pools rarely share one Mac's budget).
- A start without a pass (the web app's **Start runner**) is admitted only if free slots exceed the number of waiting pools, and does not join the queue; it cannot jump it.

"Handed over at once" in phase 1 means: from the moment a slot is free, it belongs to the head of the queue, and the head gets it the next time it asks, within one reconciliation pass (30 s by default). Nobody else can take it meanwhile.

**Later [phase 2]:**

- *Immediate hand-off.* When the manager's view shows a free slot held for the head, it can tell the reconciler (the reconciler already receives `workflow_job` webhooks; a "slot freed" nudge or a long-poll on `GET /v1/admissions/queue`) to run that pool's start immediately instead of on the next pass.
- *Weights and priority.* An optional per-pool `vmSlotWeight` (default 1): deficit round robin across waiting pools, where a grant costs 1/weight; priority as a strict class above weights, with an aging bound (a waiting entry gains one class per K minutes) so a low class still cannot starve. FIFO stays the default; no weights are added until someone needs them.
- *Persistence.* The queue lives in the manager's memory. A manager restart rebuilds it in one pass in the reconciler's visiting order, losing seniority once. If that matters, the pool's `waiting since` can move to the database and be sent with the admission.

## 4. A start waiting for a slot: visible, and outside the circuit

**[phase 1]**

- The reconciler treats `[vm_slot_busy]` as a deferred manager error, like `host_capacity_exhausted`: no runner row is created, nothing is recorded as a failure, `provision_failure_count` is untouched, so the provisioning circuit never sees it. The pool's state is `waiting`, and the manager's message, without the transport prefix, is stored in a new table, `runner_pool_waits` (migration 0042: one row per waiting pool, deleted with the pool). A table rather than a column on `runner_pools`, so the upgrade changes no existing row or table definition (the schema upgrade test checks exactly that). The row is removed when the pool leaves `waiting` and when a runner starts, and the API shows it only while the pool's state is `waiting`.
- The pool page shows **Waiting for a macOS VM slot** with that reason (position, holders) and says it is not a failure. The **Runner provisioning waiting** event is written once when the wait begins, not every pass.
- The agent's safety-net timeout (`503 vm_slot_unavailable`, see 6) is also deferred, not counted. *This reverses #87's choice* ("three timeouts open the circuit"), because #95's acceptance says a delayed start for lack of a slot never counts towards the circuit. It keeps the 30-second pause, so a slot held by software the agent cannot see does not make GridOps retry back to back.
- The fleet start-failure gate (`fleet/start_health.rs`) never sees Tart VM starts, so it needs no change.

## 5. VMs started outside GridOps

**What the agent reports [phase 1]**, in its health under `capacity.vmSlots`: `gridops` (its VM records whose VM `tart list` shows `running`; a clone waiting to boot holds no slot and is not counted), and `external`, each with name, user and source:

| Source | How | Reliable for | Misses |
|---|---|---|---|
| `tart-process` | `ps -axww -o user=,args=`, every `tart run …` process, VM name = last positional argument (values of known `tart run` options skipped), de-duplicated by (user, VM) so `sh -c "tart run x"` and the `tart` it runs count once | Tart VMs of **any user and any Tart home** on the Mac | VMs of other tools (UTM, Parallels, VirtualBuddy), Tart run through a renamed or wrapped binary that does not exec; the VM *name* is best effort if an unknown option takes a value |
| `tart-list` | `tart list --source local` in the agent's own Tart home: running VMs that are not its runners | VMs other scripts started in the same Tart home, even if `ps` cannot be read | other homes and users |

Its own `tart run` processes (including a retry during the safety-net wait) are excluded by VM name. If `ps` fails, `detection` is `tart-list` and the agent logs a warning: detection fails open, and the safety net covers it.

**Rejected:** counting `com.apple.Virtualization.VirtualMachine` XPC processes. It would see every tool's VMs, but it also counts Linux guests (Docker Desktop's and OrbStack's VMs among them, which the release's image job uses on this kind of Mac), and Linux guests do not take one of the two macOS slots. Over-counting would make GridOps wait forever for a slot that is free. This could return in phase 2 if the guest OS can be told apart reliably (needs a check on a real Mac).

**Phase 2: learning from the OS's refusal.** When the agent's `tart run` is refused although the manager's view said a slot was free, something invisible holds it. The manager can then count one "unidentified" slot for a few minutes (and say so in the reason), instead of starting clone-wait-fail cycles.

**UI:** phase 1 names the holders in the waiting reason and in the manager's health. A Tart host card listing them is phase 2.

## 6. The #87 wait becomes a safety net

**[phase 1]** Admission now refuses starts while the slots are full, so the agent's `tart run` retry loop only covers two cases: the race between admission and `tart run` (other software starting a VM in those seconds) and VMs the agent cannot see. Its default (180 s) and maximum are unchanged here; making it fit the manager's route timeout is #94, handled separately. Recommended after #94: a short wait (for example 60 s). A timeout is deferred, not counted (section 4).

## 7. Several clients, several images, VM stores

- **One pool per client [phase 1, already possible].** A pool names its own base VM (`tart_image`) and is scoped to its repositories or organization; several pools on one Mac share the slots through the queue. Another organization is another installation and another pool.
- **More than one VM store [phase 3].** Today the agent clones from one Tart home (`TART_HOME` of its user). Plan: `GRIDOPS_TART_IMAGE_SOURCES`, a list of `name=path` Tart homes the agent may clone *from*; a pool's base VM becomes `source:vm`. Tart clones only within a home, so a VM from another home is cloned with `tart clone` after `tart export`/`import`, or the agent runs `tart` with that home's `TART_HOME` for both the clone and the run (clones stay APFS copy-on-write only within one volume). The agent must refuse a source path outside the configured list. This is not needed for a client that can prepare its image in the agent's home (migration guide).

## 8. Idle keep-warm policy [phase 2]

Runners start on demand (minimum 0). A per-pool `keepWarmMinutes` (default 0) keeps the pool's last ephemeral VM idle for that long after its job ends, trading the cold start (about 28 s measured) for a held slot. Rules: a warm VM counts as a slot in use; it is the only reason a slot may sit idle; and **a warm VM is reclaimed at once when another pool is waiting in the queue**, so keep-warm can never block another client. The reconciler already evicts idle runners on a full host (`evict_surplus_runner`); keep-warm extends that to cross-pool eviction when `vm_slot_busy` names a waiting pool.

## 9. Migration guide for an external client

In `docs/macos-runners.md` ("Moving another project's macOS CI onto GridOps"): prepare its image in the agent's Tart home, create a pool scoped to its repositories with minimum 0 and its own label, switch its workflows, then stop its warm-VM scripts, delete its VMs and runner registrations, and check that `providers.tart.vmSlots.external` no longer lists them. Until it moves, its VMs are counted as `external`, so GridOps waits instead of failing.

## 10. Threats and failures

| Event | What happens | Why it is safe |
|---|---|---|
| **Tart agent restart** | Records are on disk; running VMs keep running and are counted from `tart list` again. A start in flight fails; its lease is released when the manager's request returns. | The count is recomputed from the agent's view on every admission; nothing is cached across requests. |
| **Manager restart** | Leases and the queue are lost. Running VMs are still counted by the agent. The reconciler pushes the policy (limit) on its next pass; until then the default 2 applies. The queue re-forms in one pass in the reconciler's pool order. | Losing leases can at worst admit a start whose predecessor is still booting; the agent's safety net absorbs that. Fairness loses seniority once. |
| **Lost lease** (reconciler crashes between admission and start) | The slot is held by a lease until it expires (5 min). | Conservative: refuses, never over-admits. |
| **Crashed or stopped VM** | `tart list` no longer shows it running; the slot is free on the next admission. The reconciler sees the runner `exited`/`missing` and deletes it, as before. | |
| **External VM appears between admission and `tart run`** | `tart run` is refused; the agent's safety-net wait retries; on timeout the start is deferred, not counted. | The OS is the final authority; GridOps never exceeds it. |
| **Invisible VM (another tool)** | GridOps thinks a slot is free; starts wait in the agent and time out, deferred every 30 s. | Bounded by the wait and the pause; phase 2 "learning from the refusal" removes the repeated attempts. |
| **Clock skew** | Not relevant: the queue compares pass identifiers for equality only, and leases use the manager's monotonic clock. | |
| **Two managers (or two GridOps installs) on one Mac** | Each has its own leases and queue. A VM of the other is visible as `tart-process` (or `tart-list`) once running, so it is counted; two simultaneous starts can still race. | The OS refuses the extra VM and the agent's wait absorbs it. One arbiter per Mac is the supported setup; documented. |
| **Two reconcilers** (unsupported) | Pass identifiers interleave, rounds advance fast, and waiting pools drop out of the queue sooner. | Degrades to arrival order; the limit still holds. |
| **`ps` unavailable** | `detection: "tart-list"`, warning logged, other homes' VMs not seen. | Fails open into the safety net rather than stopping all macOS work. |
| **Agent or policy sends an impossible limit** | Refused with `400` at the API and the manager; the previous limit stays. | |

## Phased plan

**Phase 1 (this change).**
- `macosVmSlots` setting (default 2, 1–2), pushed in the manager policy, editable in Settings → General.
- Manager: slot check at admission, atomic with the lease, across all pools and both callers; one FIFO line for the Mac covering both its slots and GridOps's own Tart budget, with pass-based liveness; `vm_slot_busy` with position and holders; `providers.tart.vmSlots` in health.
- Tart agent: `capacity.vmSlots` with GridOps's running VMs and external VMs from `ps` and `tart list`.
- Reconciler: `vm_slot_busy` and `vm_slot_unavailable` deferred, never counted; no 30 s pause for `vm_slot_busy` or a Tart `host_capacity_exhausted`; the wait reason (`runner_pool_waits`, migration 0042) shown on the pool page.
- Docs: `docs/macos-runners.md` updated, with the migration guide.

**Phase 2.** Immediate hand-off (nudge the reconciler when a slot frees); keep-warm per pool with cross-pool reclaim; learning from the OS's refusal; a Tart host card in the UI (slots, holders, queue); optional weights and priority with aging; shorten the safety-net wait once #94 lands.

**Phase 3.** Several Tart hosts (a `tart_hosts` table with per-host slots and queue); several VM stores per agent (`GRIDOPS_TART_IMAGE_SOURCES`).

## Not verified

Nothing here was run on a Mac. Assumptions to check there: that `ps -axww -o user=,args=` shows other users' `tart run` processes to the agent's user; that a suspended Tart VM holds no slot (phase 1 counts only `running`); the option names in `TART_RUN_VALUE_OPTIONS` against the installed Tart's `tart run --help`.
