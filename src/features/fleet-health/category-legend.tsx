import { PlaceholderCard } from "./placeholder-card";
import type { FleetHealthViewProps } from "./types";

/**
 * The job category legend (#203): each category's colour (`categoryColor`) and its share of the
 * window's CPU time.
 *
 * #197 renders a placeholder card; #203 fills it in.
 */
export function CategoryLegend(_props: FleetHealthViewProps) {
  return <PlaceholderCard issue={203} title="Job categories" />;
}
