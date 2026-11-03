/**
 * A pool waiting for one of the Tart host's macOS VM slots (#95): the pool page
 * says so, with the runner manager's reason (its place in the queue and who
 * holds the slots), instead of the generic "host capacity" notice.
 */
import { describe, expect, it } from "vitest";

import { waitingNotice } from "../src/features/runner-pools/waiting-notice";

const reason = "Waiting for a macOS VM slot: position 2 of 2 in the queue. 2 of 2 slots in use (GridOps runners: 1; other software: 1 — runner-warm-1 (ci)).";

describe("waitingNotice", () => {
  it("names the VM slot wait and shows the queue position and who holds the slots", () => {
    const notice = waitingNotice({ provisionWaitReason: reason }, "shortly");
    expect(notice.title).toBe("Waiting for a macOS VM slot");
    expect(notice.body).toContain("position 2 of 2 in the queue");
    expect(notice.body).toContain("runner-warm-1 (ci)");
    expect(notice.body).toContain("not a failure");
  });

  it("shows another wait's reason with the host-capacity notice", () => {
    const notice = waitingNotice({ provisionWaitReason: "Host runner limit reached (5/4 including reservations)." }, "at 10:30");
    expect(notice.title).toBe("Waiting for host capacity");
    expect(notice.body).toContain("Host runner limit reached");
    expect(notice.body).toContain("next check at 10:30");
  });

  it("keeps the generic notice for a server that gives no reason", () => {
    for (const pool of [{}, { provisionWaitReason: null }]) {
      const notice = waitingNotice(pool, "shortly");
      expect(notice.title).toBe("Waiting for host capacity");
      expect(notice.body).toBe("The runner host is at its safe limit. GridOps starts more runners as capacity frees up, next check shortly.");
    }
  });
});
