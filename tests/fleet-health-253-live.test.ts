// @vitest-environment jsdom
/**
 * #253 items 6, 7 and 8 on the page: the live strip shows the queue by pool (each linked to its
 * pool page) and the unmatched count the API already sends; a fleet component concern's link
 * lands on its component's row of the fleet score card; and the slowest job keys say why they are
 * empty on a machine that is at or below the fleet median.
 */
import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { FleetScoreCard } from "~/features/fleet-health/fleet-score-card";
import { LiveStrip } from "~/features/fleet-health/live-strip";
import { MachineDetail } from "~/features/fleet-health/machine-detail";
import type { FleetHealth, HostHealth } from "~/features/fleet-health/types";

import { all, fixture, mountEachTest, one, render } from "./fleet-health-dom";

// `Link` as a plain anchor, its `$param`s filled in from `params`, so a test sees the real href.
vi.mock("@tanstack/react-router", async () => {
  const { createElement: element } = await import("react");
  return {
    Link: ({ children, params, to, ...props }: { children?: unknown; params?: Record<string, string>; to: string } & Record<string, unknown>) =>
      element(
        "a",
        { ...props, href: to.replace(/\$(\w+)/g, (_, name: string) => encodeURIComponent(params?.[name] ?? `$${name}`)) },
        children as never,
      ),
  };
});

mountEachTest();

const live = (data: FleetHealth) => createElement(LiveStrip, { data, window: "24h" });

describe("the live strip's queue by pool (item 6)", () => {
  it("lists every pool with queued jobs, linked to its pool page, and the unmatched jobs", async () => {
    const data = fixture();
    data.live.queuedByPool = [
      { poolId: "pool-linux", poolName: "Linux builders", queued: 4 },
      { poolId: "pool/mac os", poolName: "Mac minis", queued: 1 },
    ];
    data.live.queuedUnmatched = 3;
    data.live.queuedTotal = 8;
    await render(live(data));

    const pools = all("[data-queued-pool]");
    expect(pools.map((pool) => pool.getAttribute("data-queued-pool"))).toEqual(["pool-linux", "pool/mac os"]);
    const linux = one('[data-queued-pool="pool-linux"]');
    expect(linux.tagName).toBe("A");
    expect(linux.getAttribute("href")).toBe("/runner-pools/pool-linux");
    expect(linux.textContent).toContain("Linux builders");
    expect(linux.textContent).toContain("4");
    expect(one('[data-queued-pool="pool/mac os"]').getAttribute("href")).toBe("/runner-pools/pool%2Fmac%20os");

    const unmatched = one('[data-testid="live-queued-unmatched"]');
    expect(unmatched.textContent).toContain("3");
    expect(unmatched.textContent).toContain("no matching pool");
    expect(unmatched.closest("a")?.getAttribute("href")).toBe("/queued-jobs");
  });

  it("says one job, not jobs, and shows nothing for an empty queue", async () => {
    const data = fixture();
    data.live.queuedByPool = [{ poolId: "pool-linux", poolName: "Linux builders", queued: 1 }];
    data.live.queuedUnmatched = 1;
    await render(live(data));
    expect(one('[data-testid="live-queued-unmatched"]').textContent).toContain("1 job with no matching pool");

    data.live = { ...data.live, queuedByPool: [], queuedUnmatched: 0, queuedTotal: 0, queuedByCategory: {}, oldestQueued: null };
    await render(live({ ...data }));
    expect(all("[data-queued-pool]")).toEqual([]);
    expect(all('[data-testid="live-queued-unmatched"]')).toEqual([]);
  });

  it("renders the fixture's pools and unmatched count", async () => {
    const data = fixture();
    expect(data.live.queuedByPool.length).toBeGreaterThan(0);
    await render(live(data));
    for (const pool of data.live.queuedByPool) {
      const element = one(`[data-queued-pool="${pool.poolId}"]`);
      expect(element.textContent).toContain(pool.poolName);
      expect(element.textContent).toContain(String(pool.queued));
    }
    expect(one('[data-testid="live-queued-unmatched"]').textContent).toContain(String(data.live.queuedUnmatched));
  });
});

describe("fleet component anchors (item 7)", () => {
  it("gives each component row the anchor its concern links to", async () => {
    const data = fixture();
    await render(createElement(FleetScoreCard, { data, window: "24h" }));
    for (const component of data.fleet.components) {
      // The link `concerns::fleet_component` gives `fleet-component:<key>`.
      const link = `/fleet/health#fleet-component-${component.key}`;
      const target = document.getElementById(link.slice(link.indexOf("#") + 1));
      expect(target, component.key).not.toBeNull();
      expect(target?.getAttribute("data-component-key")).toBe(component.key);
    }
    expect(all("[data-component-key]").every((row) => row.id === `fleet-component-${row.getAttribute("data-component-key")}`)).toBe(true);
  });
});

describe("the slowest job keys' empty state (item 8)", () => {
  const detail = (host: HostHealth, data: FleetHealth) => createElement(MachineDetail, { data, host, window: "24h" });
  const slowest = () => one('[aria-label="Slowest job keys"]');
  const firstHost = (data: FleetHealth) => {
    const [host] = data.hosts;
    if (!host) throw new Error("the fixture has machines");
    return host;
  };

  it("says no job runs slower here when the machine has an index but no key above the fleet median", async () => {
    const data = fixture();
    const host = { ...firstHost(data), speed: [{ cpuMillis: 2000, index: 0.94, topKeys: [] }] };
    await render(detail(host, data));
    expect(slowest().textContent).toContain("No job runs slower here than on the rest of the fleet");
    expect(slowest().textContent).not.toContain("Not enough data");
  });

  it("still says not enough data when no runner size has an index", async () => {
    const data = fixture();
    const host = { ...firstHost(data), speed: [{ cpuMillis: 2000, index: null, topKeys: [] }] };
    await render(detail(host, data));
    expect(slowest().textContent).toContain("Not enough data: fewer than 3 job keys qualify.");
    expect(slowest().textContent).not.toContain("No job runs slower");
  });
});
