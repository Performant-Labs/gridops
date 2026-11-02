import type { DashboardOverview } from "./types";

type SuccessRateMetrics = Pick<DashboardOverview["metrics"], "successRate" | "successRateWindowDays" | "successRateRuns">;

/** Value and hint for the Overview "Success rate" tile; the period comes from the API, not a constant. */
export function successRateTile({ successRate, successRateWindowDays, successRateRuns }: SuccessRateMetrics) {
  const value = successRate === null ? "—" : `${successRate}%`;
  if (successRateWindowDays === undefined || successRateRuns === undefined) {
    // An older server that does not report the window.
    return { value, hint: successRate === null ? "No completed runs" : "Completed runs" };
  }
  const days = `${successRateWindowDays} ${successRateWindowDays === 1 ? "day" : "days"}`;
  const runs = `${successRateRuns.toLocaleString("en-US")} ${successRateRuns === 1 ? "run" : "runs"}`;
  return { value, hint: `Last ${days} · ${runs}` };
}
