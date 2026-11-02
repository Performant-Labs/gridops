import { describe, expect, it } from "vitest";

import { metricTiles, queueParts, queuedTone, runnerParts } from "~/features/dashboard/metric-tiles";
import type { DashboardOverview } from "~/features/dashboard/types";

type Metrics = DashboardOverview["metrics"];

const metrics = (overrides: Partial<Metrics>): Metrics => ({
  runners: 0, online: 0, busy: 0, idle: 0, starting: 0, unavailable: 0,
  queuedJobs: 0, queuedWaiting: 0, queuedUnmatched: 0, successRate: null,
  ...overrides,
});

const tile = (data: Metrics, label: string) => {
  const found = metricTiles(data).find((candidate) => candidate.label === label);
  if (!found) throw new Error(`no ${label} tile`);
  return found;
};

/** A number leading each `·`-separated part of a hint. */
const hintNumbers = (hint: string) => hint.split(" · ").map((part) => Number.parseInt(part, 10));
const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);

// Nine runners, eight queued jobs: the shape of the instance the tiles were reported on.
const live = metrics({
  runners: 9, online: 5, busy: 4, idle: 1, starting: 1, unavailable: 3,
  queuedJobs: 8, queuedWaiting: 4, queuedUnmatched: 4,
});

describe("overview metric tiles", () => {
  it("splits the live-shaped fleet into parts that add up to each headline", () => {
    const runners = tile(live, "Runners");
    expect(runners.value).toBe(9);
    expect(runners.hint).toBe("4 busy · 1 idle · 1 starting · 3 unavailable");
    expect(sum(hintNumbers(runners.hint))).toBe(9);

    const queued = tile(live, "Queued jobs");
    expect(queued.value).toBe(8);
    expect(queued.hint).toBe("4 waiting for capacity · 4 no matching pool");
    expect(sum(hintNumbers(queued.hint))).toBe(8);
    expect(queued.tone).toBe("danger");
  });

  it("shows saturation as busy out of the runners that are online, and idle as its own number", () => {
    expect(tile(live, "Busy").value).toBe("4 of 5");
    expect(tile(live, "Idle").value).toBe(1);
    expect(tile(metrics({}), "Busy").value).toBe("0 of 0");
  });

  it("keeps parts that add up for any counts", () => {
    for (let seed = 0; seed < 200; seed += 1) {
      const pick = (salt: number) => ((seed * 2654435761 + salt * 40503) >>> 0) % 7;
      const [busy, idle, starting, unavailable] = [pick(1), pick(2), pick(3), pick(4)];
      const [queuedWaiting, queuedUnmatched, queuedManual] = [pick(5), pick(6), pick(7)];
      const data = metrics({
        runners: busy + idle + starting + unavailable, online: busy + idle, busy, idle, starting, unavailable,
        queuedJobs: queuedWaiting + queuedManual + queuedUnmatched, queuedWaiting, queuedManual, queuedUnmatched,
      });
      expect(sum(runnerParts(data).map((part) => part.count))).toBe(data.runners);
      expect(sum(queueParts(data).map((part) => part.count))).toBe(data.queuedJobs);
      const runnersHint = tile(data, "Runners").hint;
      if (data.runners > 0) expect(sum(hintNumbers(runnersHint))).toBe(data.runners);
      const queuedHint = tile(data, "Queued jobs").hint;
      if (data.queuedJobs > 0) expect(sum(hintNumbers(queuedHint))).toBe(data.queuedJobs);
    }
  });

  it("counts jobs only a manual-mode pool can run as their own part: a warning, not unmatched", () => {
    const data = metrics({ queuedJobs: 6, queuedWaiting: 2, queuedManual: 3, queuedUnmatched: 1 });
    const queued = tile(data, "Queued jobs");
    expect(queued.hint).toBe("2 waiting for capacity · 3 on a manual pool · 1 no matching pool");
    expect(sum(hintNumbers(queued.hint))).toBe(6);
    expect(sum(queueParts(data).map((part) => part.count))).toBe(6);
    expect(queuedTone(metrics({ queuedManual: 1 }))).toBe("warning");
    expect(queuedTone(metrics({ queuedManual: 1, queuedUnmatched: 1 }))).toBe("danger");
    // Older servers send no manual count; the hint is as it was.
    expect(tile(metrics({ queuedJobs: 1, queuedWaiting: 1 }), "Queued jobs").hint).toBe("1 waiting for capacity · 0 no matching pool");
  });

  it("goes red only for jobs no pool can serve, and says the queue is clear when it is", () => {
    expect(queuedTone(metrics({ queuedWaiting: 3 }))).toBe("warning");
    expect(queuedTone(metrics({ queuedUnmatched: 1 }))).toBe("danger");
    expect(queuedTone(metrics({}))).toBe("success");
    expect(tile(metrics({}), "Queued jobs").hint).toBe("Queue is clear");
    expect(tile(metrics({}), "Runners").hint).toBe("No runners yet");
  });
});
