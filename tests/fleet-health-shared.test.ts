/**
 * #197: the shared fleet health frontend pieces wave 1 builds on, live before any wave-1 issue:
 * the fixture fits the zod schema, the browser call, the score bands and category colours, the
 * `ApiError` that carries a 422's per-rule details (A warn 22), and the Overview tiles staying as
 * they are until #203 adds the Fleet health tile.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import fixture from "../src/features/fleet-health/fixture.json";
import { fetchFleetHealth } from "~/features/fleet-health/api";
import { categoryColor, scoreBand } from "~/features/fleet-health/chart-theme";
import { fleetHealthSchema } from "~/features/fleet-health/types";
import { FleetRequestError } from "~/features/fleet/registry-api";
import { metricTiles } from "~/features/dashboard/metric-tiles";
import type { DashboardOverview } from "~/features/dashboard/types";
import { ApiError, api } from "~/lib/api";

const mockFetch = (body: unknown, status = 200) => {
  const fetcher = vi.fn().mockResolvedValue(Response.json(body, { status }));
  vi.stubGlobal("fetch", fetcher);
  return fetcher;
};

afterEach(() => vi.unstubAllGlobals());

describe("fleet health fixture and schema", () => {
  it("parses the fixture, and refuses a field the schema doesn't know", () => {
    const parsed = fleetHealthSchema.parse(fixture);
    expect(parsed.window).toBe("24h");
    expect(parsed.stale).toBe(false);
    expect(parsed.hosts.map((host) => host.name).sort()).toEqual(["Jupiter", "MacBookPro", "callisto", "europa", "io"]);
    expect(fleetHealthSchema.safeParse({ ...fixture, surprise: true }).success).toBe(false);
  });

  it("has more concerns than the five Areas of concern shows, of every severity", () => {
    expect(fixture.concerns.length).toBeGreaterThan(5);
    const severities = new Set(fixture.concerns.map((concern) => concern.severity));
    expect([...severities].sort()).toEqual(["critical", "info", "warning"]);
  });
});

describe("what the wave-1 contract tests need from the fixture", () => {
  const parsed = fleetHealthSchema.parse(fixture);

  it("has machines with and without a score, and a machine at two runner sizes", () => {
    const scores = parsed.hosts.map((host) => host.score);
    expect(scores.some((score) => score === null)).toBe(true);
    expect(new Set(scores.filter((score) => score !== null)).size).toBeGreaterThanOrEqual(3);
    expect(parsed.hosts.some((host) => host.speed.length >= 2)).toBe(true);
    const indexes = parsed.hosts.flatMap((host) => host.speed.map((size) => size.index));
    expect(indexes.some((index) => index === null)).toBe(true);
    expect(indexes.some((index) => index !== null)).toBe(true);
    expect(parsed.hosts.some((host) => host.categories.length >= 2)).toBe(true);
  });

  it("has a fleet score with components, and live counts by category that add up", () => {
    expect(parsed.fleet.score).not.toBeNull();
    expect(parsed.fleet.components.length).toBeGreaterThanOrEqual(2);
    const total = (counts: Record<string, number | undefined>) => Object.values(counts).reduce<number>((sum, count) => sum + (count ?? 0), 0);
    expect(parsed.live.runningTotal).toBeGreaterThan(0);
    expect(total(parsed.live.runningByCategory)).toBe(parsed.live.runningTotal);
    expect(parsed.live.queuedTotal).toBeGreaterThan(0);
    expect(total(parsed.live.queuedByCategory)).toBe(parsed.live.queuedTotal);
  });
});

describe("fetchFleetHealth", () => {
  it("GETs the window same-origin and uncached, and returns the parsed body", async () => {
    const fetcher = mockFetch({ ...fixture, window: "7d" });
    const health = await fetchFleetHealth("7d");
    expect(health.window).toBe("7d");
    const [path, init] = fetcher.mock.calls[0] as [string, RequestInit];
    expect(path).toBe("/api/v1/fleet/health?window=7d");
    expect(init.method ?? "GET").toBe("GET");
    expect(init.cache).toBe("no-store");
    expect(init.credentials).toBe("same-origin");
  });

  it("throws a FleetRequestError with the status on a refusal and on a body that doesn't parse", async () => {
    mockFetch({ error: { code: "forbidden", message: "Forbidden.", requestId: "req-1" } }, 403);
    const refused = await fetchFleetHealth("24h").catch((error: unknown) => error);
    expect(refused).toBeInstanceOf(FleetRequestError);
    expect((refused as FleetRequestError).status).toBe(403);

    mockFetch({ window: "24h" });
    await expect(fetchFleetHealth("24h")).rejects.toBeInstanceOf(FleetRequestError);
  });
});

describe("chart theme", () => {
  it("bands a score good from 90, fair from 70, poor below, and none without a score", () => {
    expect([100, 90, 89, 70, 69, 0].map(scoreBand)).toEqual(["good", "good", "fair", "fair", "poor", "poor"]);
    expect(scoreBand(null)).toBe("none");
  });

  it("gives each category its theme variable", () => {
    for (const category of ["test", "build", "check", "deploy", "fix-agent", "other"] as const) {
      expect(categoryColor(category)).toBe(`var(--category-${category})`);
    }
  });
});

describe("ApiError details (A warn 22)", () => {
  it("keeps a 422's code and per-rule details", async () => {
    const details = [{ position: 1, message: "regex parse error: unclosed group" }];
    mockFetch({ error: "1 rule has an invalid pattern.", code: "invalid_job_category_rules", details }, 422);
    const error = await api("/api/v1/settings/job-categories", { method: "PUT", body: { rules: [] } }).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).message).toBe("1 rule has an invalid pattern.");
    expect((error as ApiError).status).toBe(422);
    expect((error as ApiError).code).toBe("invalid_job_category_rules");
    expect((error as ApiError).details).toEqual(details);
  });

  it("leaves code and details undefined where the body has none, and keeps the two-argument constructor", async () => {
    mockFetch({ error: "bad" }, 400);
    const error = await api("/api/v1/settings").catch((caught: unknown) => caught);
    expect((error as ApiError).message).toBe("bad");
    expect((error as ApiError).code).toBeUndefined();
    expect((error as ApiError).details).toBeUndefined();
    const plain = new ApiError("gone", 404);
    expect([plain.message, plain.status, plain.code, plain.details]).toEqual(["gone", 404, undefined, undefined]);
  });
});

describe("Overview tiles before #203", () => {
  const metrics: DashboardOverview["metrics"] = {
    runners: 2, online: 2, busy: 1, idle: 1, starting: 0, unavailable: 0,
    queuedJobs: 0, queuedWaiting: 0, queuedUnmatched: 0, successRate: null,
  };

  it("takes the fleet health as an optional second argument and adds no tile yet", () => {
    const without = metricTiles(metrics);
    expect(without.map((tile) => tile.label)).toEqual(["Runners", "Busy", "Idle", "Queued jobs", "Success rate"]);
    expect(metricTiles(metrics, undefined)).toEqual(without);
  });
});
