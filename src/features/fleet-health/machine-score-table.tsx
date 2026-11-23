import { PlaceholderCard } from "./placeholder-card";
import type { FleetHealthViewProps } from "./types";

/**
 * The machine score table (#202): every machine worst score first, its components, its speed index
 * per runner size, and a row that expands into `MachineDetail`; `#machine-<host>` deep-links to
 * an expanded row.
 *
 * #197 renders a placeholder card; #202 fills it in.
 */
export function MachineScoreTable(_props: FleetHealthViewProps) {
  return <PlaceholderCard issue={202} title="Machines" />;
}
