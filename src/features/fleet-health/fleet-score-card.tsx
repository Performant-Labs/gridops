import { PlaceholderCard } from "./placeholder-card";
import type { FleetHealthViewProps } from "./types";

/**
 * The fleet score card (#195): the score in its band (`ScoreGauge` from
 * `~/components/charts/score-gauge`, which carries `data-band`), the trend, and one row per
 * component (`data-component-key`, `data-insufficient="true"` when it has no score) with its
 * deduction sentence.
 *
 * #197 renders a placeholder card; #195 fills it in.
 */
export function FleetScoreCard(_props: FleetHealthViewProps) {
  return <PlaceholderCard issue={195} title="Fleet score" />;
}
