import { describe, expect, it } from "vitest";

import { successRateTile } from "~/features/dashboard/success-rate";

describe("success rate tile", () => {
  it("names the period and the number of runs counted", () => {
    expect(successRateTile({ successRate: 86.4, successRateWindowDays: 30, successRateRuns: 5372 })).toEqual({
      value: "86.4%",
      hint: "Last 30 days · 5,372 runs",
    });
  });

  it("uses singular wording for one run", () => {
    expect(successRateTile({ successRate: 100, successRateWindowDays: 30, successRateRuns: 1 }).hint).toBe("Last 30 days · 1 run");
  });

  it("follows the window the API reports rather than a hard-coded one", () => {
    expect(successRateTile({ successRate: 90, successRateWindowDays: 7, successRateRuns: 12 }).hint).toBe("Last 7 days · 12 runs");
    expect(successRateTile({ successRate: 90, successRateWindowDays: 1, successRateRuns: 12 }).hint).toBe("Last 1 day · 12 runs");
  });

  it("shows a dash and still names the window when no runs count", () => {
    expect(successRateTile({ successRate: null, successRateWindowDays: 30, successRateRuns: 0 })).toEqual({
      value: "—",
      hint: "Last 30 days · 0 runs",
    });
  });

  it("keeps a dash with no period when an older server omits the window", () => {
    expect(successRateTile({ successRate: null })).toEqual({ value: "—", hint: "No completed runs" });
    expect(successRateTile({ successRate: 55.7 })).toEqual({ value: "55.7%", hint: "Completed runs" });
  });
});
