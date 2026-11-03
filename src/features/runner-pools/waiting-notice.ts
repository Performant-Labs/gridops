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
