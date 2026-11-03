/** How a pool waiting for capacity is explained on its page. */

/** The runner manager's reason begins with this when a Tart VM waits for one of the Mac's slots (#95). */
const VM_SLOT_WAIT = "Waiting for a macOS VM slot";

/**
 * The notice for a pool in the `waiting` state. A wait for a macOS VM slot shows the manager's
 * reason, which gives the pool's place in the queue and who holds the slots; `when` is the
 * phrase for the next check, such as "at 10:30" or "shortly".
 */
export function waitingNotice(pool: { provisionWaitReason?: string | null }, when: string): { title: string; body: string } {
  const reason = pool.provisionWaitReason?.trim();
  if (reason?.startsWith(VM_SLOT_WAIT)) {
    return {
      title: VM_SLOT_WAIT,
      body: `${reason} The Mac runs at most two macOS VMs at once; GridOps gives the next free slot to the pool that has waited longest. This is not a failure and does not count towards the provisioning circuit.`,
    };
  }
  const generic = `The runner host is at its safe limit. GridOps starts more runners as capacity frees up, next check ${when}.`;
  return { title: "Waiting for host capacity", body: reason ? `${reason} ${generic}` : generic };
}

export type ProvisioningNotice = { title: string; body: string; tone: "danger" | "warning" | "neutral" };

type NoticePool = { state: string; provisionCircuitOpen: boolean; provisionFailureCount: number; provisionWaitReason?: string | null };

/**
 * What the pool page says about provisioning, or `null` when there is nothing to say. A pool the
 * server shows `waiting` gets its wait first, even with earlier failures or an open provisioning
 * circuit (#102), and the notice says what those are; otherwise an open circuit, then a backoff.
 */
export function provisioningNotice(pool: NoticePool, when: string): ProvisioningNotice | null {
  const failures = pool.provisionFailureCount;
  if (pool.state === "waiting") {
    const notice = waitingNotice(pool, when);
    if (pool.provisionCircuitOpen) {
      return {
        ...notice,
        body: `${notice.body} Provisioning stays paused after ${failures} earlier failures until a runner starts: this start closes the circuit if it succeeds, and a failure opens it for another cool-down.`,
        tone: "warning",
      };
    }
    if (failures > 0) {
      return {
        ...notice,
        body: `${notice.body} ${failures === 1 ? "The last attempt" : `The last ${failures} attempts`} failed before this wait.`,
        tone: "neutral",
      };
    }
    return { ...notice, tone: "neutral" };
  }
  if (pool.provisionCircuitOpen) {
    return { title: `Provisioning paused after ${failures} consecutive failures`, body: `GridOps tries again ${when}. The Activity tab shows the failing step; use the pool menu to retry now.`, tone: "danger" };
  }
  if (failures > 0) {
    return { title: "Provisioning is backing off", body: `The last ${failures === 1 ? "attempt" : `${failures} attempts`} failed. GridOps retries ${when}.`, tone: "warning" };
  }
  return null;
}
