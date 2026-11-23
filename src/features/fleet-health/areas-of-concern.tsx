import { PlaceholderCard } from "./placeholder-card";
import type { FleetHealthViewProps } from "./types";

/**
 * The areas of concern (#195): the merged concern list, worst first then oldest, at most five until
 * "Show all N". Each row is keyed by the concern's `key` (`data-concern-key`), so a refresh keeps
 * it, and a concern that wasn't in the previous data is marked `data-new="true"`; "No areas of
 * concern" when the list is empty.
 *
 * #197 renders a placeholder card; #195 fills it in.
 */
export function AreasOfConcern(_props: FleetHealthViewProps) {
  return <PlaceholderCard issue={195} title="Areas of concern" />;
}
