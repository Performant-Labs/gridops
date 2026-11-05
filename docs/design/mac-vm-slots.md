# Design: GridOps owns a Mac's macOS VM slots

Issue: [#95](https://github.com/Performant-Labs/gridops/issues/95). Related: #86 and #87 (the slot wait in the Tart agent), #94 and #96 (making that wait fit the manager's time limit), #80 (a release needs a macOS runner).

Status: **phase 1 is implemented** (marked **[phase 1]** below). Of phase 2, the immediate hand-off of a freed slot is implemented in 0.2.0 (marked **[0.2.0]**), with Tart starts run beside the reconciler's pass. Everything else is the plan.

## The problem in one paragraph

An Apple-silicon Mac runs at most two macOS VMs at once, whoever starts them. Before this change nothing arbitrated those two slots. The Tart agent counted only its own runners (`GRIDOPS_TART_MAX_RUNNERS`), the manager's admission counted only CPU, memory and the runner count, and the reconciler visits pools in creation order, so on every pass the oldest pool got the first chance at a freed slot. Another client's warm-VM scripts held both slots most of the day, a GridOps start found none, waited in the agent (#87), failed, and three failures opened the pool's provisioning circuit. The #87 wait is a poll, not a queue: whoever retries `tart run` at the right moment wins.

## How runners reach the Mac today

```
reconciler (one process, one pass at a time, pools in creation order)
  └─ provision_on_host ─ POST /v1/admissions ─▶ manager.reserve_capacity
     (a Tart start, once admitted, runs in its own task, one at a time; the pass carries on [0.2.0])
                                                 ├─ provision_lock (tokio Mutex; one per provider since 0.2.0)
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
- *Relationship to `GRIDOPS_TART_MAX_RUNNERS`:* that stays the agent's cap on GridOps's own runners (VM and native) and its CPU and memory budget stay too. The slot limit is separate because it counts other software's VMs and excludes native runners. **[0.2.0]** The cap defaults to 2 (it was 1), so by default it no longer holds GridOps to one slot. When an agent's cap is still below the slot limit, the lower one wins, so the manager's health reports the agent's cap next to the limit (`agentMaxRunners`) with a `warning`, and Settings → General shows the same warning under the slot setting. A cap at or above the limit gives no warning.

**API [phase 1].** `GET /api/v1/settings` returns `settings.macosVmSlots`; `PUT /api/v1/settings` accepts an optional `macosVmSlots` (left out, it is unchanged, so older clients keep working; out of range is `400`), audited with `settings.updated`. Pool list and detail gain `provisionWaitReason` (see 4). The manager's health gains `providers.tart.vmSlots`:

```json
{ "limit": 2, "inUse": 2, "free": 0, "gridops": 1, "runnerIds": ["…"], "starting": 0, "startingVms": [],
  "external": [{ "name": "runner-warm-1", "user": "ci", "source": "tart-process" }],
  "detection": "processes",
  "queue": [{ "poolId": "…", "position": 1, "waitingSince": "2026-11-03T09:00:00Z" }],
  "agentMaxRunners": 2, "warning": null }
```

`agentMaxRunners` and `warning` were added in 0.2.0 (see "Relationship to `GRIDOPS_TART_MAX_RUNNERS`" above). `GET /api/v1/settings` passes this object on as `manager.tartVmSlots` (null without a Tart agent). `runnerIds` (the runners of GridOps's running VMs, from the agent; null from an agent that predates it) and `startingVms` (each start in flight as `{ runnerId, poolId }`, the leases counted in `starting`) were added in 0.2.0 for the Tart host card.

**UI [phase 1].** One number field in Settings → General, read-only for members. The operator also sees the reason on the waiting pool's page.

**UI [phase 2, 0.2.0]: the Tart host card.** The Overview has a **Tart host** card: the limit, who holds each slot (a GridOps runner with its pool, name and when it was started; a VM started outside GridOps; a start in progress; or free), the queue in order with how long each pool has waited, and the agent cap warning. It reads `GET /api/v1/overview/tart-host` (session, or a token with `pools:read`), which adds to the manager's report the names GridOps holds, only for pools the caller may see (others are `poolHidden`). It is on the Overview rather than the Machines page because the Tart host is not an enrolled machine: the manager reaches it through `GRIDOPS_TART_AGENT_URL`, and nothing links it to a row on the Machines page; the Overview is also where every signed-in user, not only administrators, looks at capacity and queued work. The card fetches on its own, so a slow agent never holds up the page; it is hidden without a Tart agent or when the manager cannot be asked, and says so when the agent is unreachable.

## 2. Central, atomic admission

**[phase 1]** In `reserve_capacity`, for `provider: "tart"` and `runtime` other than `native` (the admission request now says which; a missing runtime counts as a VM, the safe reading):

1. under `provision_lock` (the Tart start lock since 0.2.0, when each provider got its own), fetch the agent's health once (budget, active runners, `vmSlots`);
2. under the reservations lock, run the existing CPU/memory/runner checks, then the slot check, then insert the lease, all in one critical section;
3. slots in use = agent's `gridops` (GridOps VMs that are running) + `external.len()` + leases with `vm_slot: true` whose VM the agent does not yet report running (starts in flight, see below). Free = limit − in use.
4. The queue (3) is asked with the number of starts that may go ahead now: the free slots, capped by the room left in GridOps's own Tart budget for a runner of this size (`GRIDOPS_TART_MAX_RUNNERS`, CPU, memory). If it admits the pool, the lease is created with `vm_slot: true`. Otherwise the answer is `429` with the pool's place in line: `vm_slot_busy` ("Waiting for a macOS VM slot: position p of n in the queue. u of l slots in use (GridOps runners: g; other software: e — name (user), …).") when the slots are what is short, or `host_capacity_exhausted` ("Waiting for room on the Tart host: position p of n in the queue." and the budget it hit) when GridOps's own budget is.

   *Why the budget is in the same line:* the agent's runner cap was 1 by default when this was written (2 since 0.2.0), and a cap of 1 or large VMs against the CPU and memory budget fill it before both slots are. Checked outside the queue, the pool the reconciler visits first would take every runner that frees and the other would starve exactly as before; a test pins this (`the_tart_runner_budget_does_not_let_a_later_pool_jump_the_line`).

The VM slot lease is released when `POST /v1/runners` ends: success, failure, the manager's time limit, or the caller abandoning the request. When the start ends in the handler (started, failed, or over the manager's time limit), the handler waits for the lease map and removes the lease while it still holds the Tart start lock, so the next Tart admission, which waits for that lock, can never find it. Only when the caller abandons the request, and the server drops the handler, does a guard release it instead; if the lease map is busy at that moment, the guard hands the removal to a background task, which may finish after the lock is released (0.2.0; before, the guard released every start's lease, so a busy map could delay a finished start's release past the lock too). The VM runs before that: Tart lists it running from boot, while the agent still sets up the runner inside it, which takes seconds to minutes. *Corrected in #101:* phase 1 said that holding the start lock (then `provision_lock`, one for every provider) for the whole start meant "the lease and the running VM are never both visible to another admission". That held for admission, but not for the manager's health, which takes no lock and read `inUse 3` of 2 in a live run, and not for a lease left behind when the caller gave up on the request: the server then drops the handler and the release written after its `.await` never ran, so the lease held a slot (and `GridOps`'s Tart budget) for its full five minutes on top of the running VM. So the agent now names the runner of each running VM (`runnerIds`), and a lease whose runner's VM the agent reports running counts neither as `starting` nor against the Tart budget (the agent's runner cap, `agentMaxRunners` in the health, CPU and memory): each VM is counted once, from admission (lease) through booting (lease) and running (the agent) to stopping (the agent, until `tart list` no longer shows it running). The same count is in the health's `starting`, `inUse` and `free`, and so in Settings' `manager.tartVmSlots`. An agent that does not send `runnerIds` has every lease counted, as before: a VM may count twice for a moment, never not at all.

An agent that predates slot reporting is read as "every active runner is a VM, nobody else's VMs are visible", so the limit still holds for GridOps's own VMs.

Both the reconciler and the web app go through this check, so neither can bypass it. Tests: two pools racing for the last slot get exactly one lease (10 rounds), eight concurrent starts never get more than two; both fail if the check and the insert are split.

## 3. Queue and fairness

**Policy [phase 1]: one FIFO queue per host, one entry per pool, ordered by when the pool began waiting.**

- A request from pool P with `f` free slots is admitted only if P is among the first `f` live entries of the queue. A pool not yet in the queue joins at the back. So a freed slot is held for the head of the queue and nobody else can take it, whichever pool happens to ask first.
- An admitted pool leaves the queue; if it wants another slot it joins at the back. Two pools that always want a slot therefore alternate, and with N pools each waits at most N − 1 grants: **no starvation**, independent of the order the reconciler visits pools.
- **Liveness without clocks.** Every admission from the reconciler carries `reconcilePass` (the pass's start time, used only as an identifier). The manager counts passes as *rounds* when the identifier changes. An entry that did not ask in a whole round (it is two rounds behind) has stopped asking (its job ran elsewhere or was cancelled, or its pool was paused) and is dropped, so a slot is never held for a pool that no longer wants it for more than one pass. No timestamps from different machines are compared, so clock skew does not matter.
- **Rounds while a Tart start runs [0.2.0].** A Tart start runs beside the pass, one at a time (`crates/gridops-reconciler/src/tart_lane.rs`). While it runs, the reconciler asks for no other Tart runner: the manager's Tart lock would hold that admission for the whole start, and the pass with it. A pool that wanted one is *held back*; it has not stopped asking. So a pass that held a pool back does not begin a new round: the next pass sends the same `reconcilePass`, and so on until a pass has asked for every pool that wanted a Tart runner. A pool waiting in line therefore keeps its place however long the start in front of it takes (up to the manager's 15-minute limit), and the round advances again as soon as nobody is held back. The cost: while a Tart start runs, a pool that stopped wanting a slot stays in the line until the round advances. That holds no slot, since nobody else can be admitted then either.
- The reconciler does **not** pause a pool for 30 s after `vm_slot_busy`, nor after a Tart `host_capacity_exhausted` (it does for other waits), so a waiting pool asks on every pass and keeps its place. Native Tart runners take no slot and do not wait in the line; they still count against the Tart budget, so a native start can take budget room a waiting VM pool wanted (an accepted edge in phase 1; native and VM pools rarely share one Mac's budget).
- A start without a pass (the web app's **Start runner**) is admitted only if free slots exceed the number of waiting pools, and does not join the queue; it cannot jump it.

"Handed over at once" in phase 1 meant: from the moment a slot is free, it belongs to the head of the queue, and the head gets it the next time it asks, within one reconciliation pass (30 s by default). Nobody else can take it meanwhile. Since 0.2.0 the head asks at once (below).

**Immediate hand-off [0.2.0].** The reconciler runs its next pass as soon as a slot frees, instead of at the end of its interval:

- *A Tart start of the reconciler's ends* (started, failed, deferred): the start's task wakes the pass when it gives up its claim on the Tart lane. A failed start's lease is released by the manager before it answers (#111), so the slot is free by then.
- *Anything else frees a slot* (a GridOps VM's job ends and the agent stops it, a VM of other software stops, the limit is raised, a start the reconciler did not make gives its lease back): while a pool waits in the line (a start in the last pass was refused with `vm_slot_busy`, or with a Tart `host_capacity_exhausted`) and no Tart start of the reconciler's runs, the reconciler reads `GET /v1/vm-slots` on the manager every 3 s. That answers what the health's `providers.tart.vmSlots` answers, asking the Tart agent only. When fewer slots are in use or more are free than at the last look, it wakes the pass. On the first look after the wait began, a free slot counts too: it may have freed between the refusal and that look.
- The woken pass is an ordinary pass: every pool, in creation order. The head of the line asks first only if it is visited first, but that does not matter: the manager keeps the slot for the head and refuses everyone else (section 3), so the head gets it in this pass whatever the order. FIFO and the round rule are unchanged; a woken pass is one more round, and a waiting pool asks in every pass.

*Why the reconciler watches rather than the manager nudging.* The manager learns that a VM stopped only by asking the Tart agent (`tart list`, `ps`), so somebody has to poll the agent either way; the manager has no event to forward. The reconciler has no HTTP endpoint for the manager to call, and adding one (with its own token) to push a nudge that is itself the result of a poll adds a moving part without saving one. A long poll on the manager would do the same polling inside the manager. Polling from the reconciler only while a pool waits keeps the agent's load where it was when nobody waits, and is 20 light requests a minute while someone does. The latency is at most 3 s plus the pass, against up to 30 s before.

**Later [phase 2]:**

- *Weights and priority.* An optional per-pool `vmSlotWeight` (default 1): deficit round robin across waiting pools, where a grant costs 1/weight; priority as a strict class above weights, with an aging bound (a waiting entry gains one class per K minutes) so a low class still cannot starve. FIFO stays the default; no weights are added until someone needs them.
- *Persistence.* The queue lives in the manager's memory. A manager restart rebuilds it in one pass in the reconciler's visiting order, losing seniority once. If that matters, the pool's `waiting since` can move to the database and be sent with the admission.

## 4. A start waiting for a slot: visible, and outside the circuit

**[phase 1]**

- The reconciler treats `[vm_slot_busy]` as a deferred manager error, like `host_capacity_exhausted`: no runner row is created, nothing is recorded as a failure, `provision_failure_count` is untouched, so the provisioning circuit never sees it. The pool's state is `waiting`, and the manager's message, without the transport prefix, is stored in a new table, `runner_pool_waits` (migration 0042: one row per waiting pool, deleted with the pool). A table rather than a column on `runner_pools`, so the upgrade changes no existing row or table definition (the schema upgrade test checks exactly that). The row is removed when the pool leaves `waiting` and when a runner starts, and the API shows it only while the pool's state is `waiting`.
- The pool page shows **Waiting for a macOS VM slot** with that reason (position, holders) and says it is not a failure. The **Runner provisioning waiting** event is written once when the wait begins, not every pass.
- **State precedence (#102).** The reconciler picks a pool's state as: paused → provisioning paused → built-in host off → **waiting** → circuit open (`blocked`) → a failure's pause (`backoff`) → updating, draining, active. A pool is waiting when its start was deferred in this pass, or when it is in the pause after a start deferred in an earlier pass (its wait row exists; a failure deletes that row, so a failure's pause is still `backoff`). Phase 1 put the open circuit and earlier failures first, so a pool that waited in the queue with its circuit open from earlier failures was shown `blocked`, its wait row was deleted at the end of every pass (no `provisionWaitReason`), and the **Runner provisioning waiting** event was written again on every pass.
- **The circuit while a pool waits.** A pool whose circuit is open makes no start during the cool-down, so it is not in the queue then and is shown `blocked`. When the cool-down has passed it asks again; if that start is refused for a slot it joins the queue and is shown waiting. The circuit stays open, and `provisionFailureCount` keeps its value, while it waits: a wait is no failure, so it does not count, and it shows nothing about whatever made the earlier starts fail, so it does not close the circuit either. The deferred start is the circuit's probe, postponed until a slot is free: if it succeeds the circuit closes (as any successful start resets it), and if it fails the circuit opens for another cool-down. The pool asks on every pass meanwhile (a slot wait sets no pause), so it keeps its place in line. The web app lists such a pool under waiting and its page shows the wait first, then that the circuit is open.
- The agent's safety-net timeout (`503 vm_slot_unavailable`, see 6) is also deferred, not counted. *This reverses #87's choice* ("three timeouts open the circuit"), because #95's acceptance says a delayed start for lack of a slot never counts towards the circuit. It keeps the 30-second pause, so a slot held by software the agent cannot see does not make GridOps retry back to back.
- The fleet start-failure gate (`fleet/start_health.rs`) never sees Tart VM starts, so it needs no change.

## 5. VMs started outside GridOps

**What the agent reports [phase 1]**, in its health under `capacity.vmSlots`: `gridops` (its VM records whose VM `tart list` shows `running`; a clone waiting to boot holds no slot and is not counted), `runnerIds` (the `GridOps` runner ids of those VMs, so the manager counts a start's VM once, see 2; added in #101), and `external`, each with name, user and source:

| Source | How | Reliable for | Misses |
|---|---|---|---|
| `tart-process` | `ps -axww -o user=,args=`, every `tart run …` process, VM name = last positional argument (values of known `tart run` options skipped), de-duplicated by (user, VM) so `sh -c "tart run x"` and the `tart` it runs count once | Tart VMs of **any user and any Tart home** on the Mac | VMs of other tools (UTM, Parallels, VirtualBuddy), Tart run through a renamed or wrapped binary that does not exec; the VM *name* is best effort if an unknown option takes a value |
| `tart-list` | `tart list --source local` in the agent's own Tart home: running VMs that are not its runners | VMs other scripts started in the same Tart home, even if `ps` cannot be read | other homes and users |

Its own `tart run` processes (including a retry during the safety-net wait) are excluded by VM name. If `ps` fails, `detection` is `tart-list` and the agent logs a warning: detection fails open, and the safety net covers it.

**Rejected:** counting `com.apple.Virtualization.VirtualMachine` XPC processes. It would see every tool's VMs, but it also counts Linux guests (Docker Desktop's and OrbStack's VMs among them, which the release's image job uses on this kind of Mac), and Linux guests do not take one of the two macOS slots. Over-counting would make GridOps wait forever for a slot that is free. This could return in phase 2 if the guest OS can be told apart reliably (needs a check on a real Mac).

**Phase 2: learning from the OS's refusal.** When the agent's `tart run` is refused although the manager's view said a slot was free, something invisible holds it. The manager can then count one "unidentified" slot for a few minutes (and say so in the reason), instead of starting clone-wait-fail cycles.

**UI:** phase 1 names the holders in the waiting reason and in the manager's health. The Tart host card (0.2.0, see 1) lists them.

## 6. The #87 wait becomes a safety net

**[phase 1]** Admission now refuses starts while the slots are full, so the agent's `tart run` retry loop only covers two cases: the race between admission and `tart run` (other software starting a VM in those seconds) and VMs the agent cannot see. #94 (fixed in #96) made the wait fit inside the manager's time limit for a macOS start (a 600 s maximum, against the manager's 900 s). Phase 1 left its default at 180 s; since admission holds starts back, 0.2.0 shortens the default to 60 s so pools wait in the fair queue rather than inside the agent (the maximum stays 600 s). A timeout is deferred, not counted (section 4).

## 6a. Starts the API asked for [0.2.0]

A start waits in the line for a slot for as long as it takes, and the API cuts a request off after 30 seconds, so the API does not start runners for a pool that can use Tart (open question 1 of #96):

- **Pool create** with a desired count, **scale**, **sync** (`reconcile`) and **rebuild** record what the pool should look like and answer at once. The reconciler's next pass (every 30 seconds by default) starts the runners, in its Tart lane and the same line as every other start.
- **The response** keeps its shape. `provisioned` is empty for such a pool, and a new `handedToReconciler` says how many runners were left to the reconciler (pool create: the desired count; sync and scale: the missing runners). The pool page shows the pool as waiting or starting, as for any other start.
- **A pool with only Docker** keeps starting in the request: a Docker start does not wait in the line. A pool with Docker and Tart hands over too, so its Tart starts cannot be cut off; its Docker starts then come with the next pass.
- **Rebuild** deletes the runner and leaves the replacement to the reconciler, which restores the pool's desired size.
- **Not changed:** no migration, no new column, no new setting. The reconciler already converges every pool to its desired size.

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

**Phase 2.** Immediate hand-off (the reconciler runs a pass as soon as a slot frees; done in 0.2.0); keep-warm per pool with cross-pool reclaim; learning from the OS's refusal; a Tart host card in the UI (slots, holders, queue; done in 0.2.0); optional weights and priority with aging. (Shortening the safety-net wait's default was decided for 0.2.0: 60 s.)

**Phase 3.** Several Tart hosts (a `tart_hosts` table with per-host slots and queue); several VM stores per agent (`GRIDOPS_TART_IMAGE_SOURCES`).

## Not verified

Nothing here was run on a Mac. Assumptions to check there: that `ps -axww -o user=,args=` shows other users' `tart run` processes to the agent's user; that a suspended Tart VM holds no slot (phase 1 counts only `running`); the option names in `TART_RUN_VALUE_OPTIONS` against the installed Tart's `tart run --help`.
