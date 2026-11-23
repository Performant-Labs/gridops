import { PlaceholderCard } from "./placeholder-card";
import type { FleetHealthViewProps } from "./types";

/**
 * One machine's detail (#202): its per-category usage, the job keys behind its speed index and its
 * live stats.
 *
 * #197 renders a placeholder card; #202 fills it in.
 */
export function MachineDetail(_props: FleetHealthViewProps) {
  return <PlaceholderCard issue={202} title="Machine detail" />;
}
