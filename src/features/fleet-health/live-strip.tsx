import { PlaceholderCard } from "./placeholder-card";
import type { FleetHealthViewProps } from "./types";

/**
 * The live strip (#203): what is queued and running right now, by category
 * (`data-queued-category` / `data-running-category`), the totals (`data-testid="live-queued-total"`
 * and `"live-running-total"`) and the oldest queued job's wait.
 *
 * #197 renders a placeholder card; #203 fills it in.
 */
export function LiveStrip(_props: FleetHealthViewProps) {
  return <PlaceholderCard issue={203} title="Right now" />;
}

/**
 * How long the oldest queued job has waited: whole seconds under a minute ("45 s"), whole minutes
 * under an hour ("12 min"), then hours and minutes ("3 h 5 min", "1 h"). #203 implements it.
 */
export function formatOldestWait(_waitMs: number): string {
  return "";
}
