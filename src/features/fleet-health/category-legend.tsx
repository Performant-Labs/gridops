import { Link } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";

import { categoryColor, categoryLabel } from "./chart-theme";
import { JOB_CATEGORIES, type FleetHealthViewProps, type JobCategory } from "./types";

/** The one line each category gets in the legend. */
const CATEGORY_DESCRIPTIONS: Record<JobCategory, string> = {
  test: "Unit, integration and end-to-end tests.",
  build: "Compiles code and builds images and artifacts.",
  check: "Lint, format and typecheck gates.",
  deploy: "Publish and roll out releases.",
  "fix-agent": "Agent runs that fix failed jobs; counted live only.",
  other: "Jobs no category rule matched.",
};

/**
 * The job category legend (#203): each category's colour (`categoryColor`) with its one-line
 * description and its share of the window's CPU time, and the way into the rules that decide the
 * categories (the Settings section, #199). `fix-agent` never carries window hours: it is counted
 * live only.
 */
export function CategoryLegend({ data }: FleetHealthViewProps) {
  const totalCpuHours = data.fleet.categories.reduce((total, usage) => total + usage.cpuHours, 0);
  const cpuHours = new Map(data.fleet.categories.map((usage) => [usage.category, usage.cpuHours]));
  return (
    <section aria-label="Job categories" className="rounded-lg border border-border bg-panel">
      <div className="flex items-center justify-between gap-3 border-b border-border px-4 py-2.5">
        <h2 className="text-sm font-medium text-foreground">Job categories</h2>
        <Link className="flex min-h-6 shrink-0 items-center gap-1 text-xs text-muted-foreground hover:text-foreground" to="/settings/general">
          Edit job categories <ArrowRight className="size-3" />
        </Link>
      </div>
      <ul className="grid gap-x-8 px-4 py-1.5 sm:grid-cols-2">
        {JOB_CATEGORIES.map((category) => {
          const hours = cpuHours.get(category) ?? 0;
          const share = totalCpuHours > 0 ? Math.round((hours / totalCpuHours) * 100) : null;
          return (
            <li className="flex min-w-0 items-baseline gap-2 py-1.5" key={category}>
              <span aria-hidden className="size-2 shrink-0 self-center rounded-full" style={{ backgroundColor: categoryColor(category) }} />
              <span className="shrink-0 text-sm font-medium text-foreground">{categoryLabel(category)}</span>
              <span className="min-w-0 flex-1 text-xs text-muted-foreground sm:truncate" data-category-description={category} title={CATEGORY_DESCRIPTIONS[category]}>{CATEGORY_DESCRIPTIONS[category]}</span>
              {share !== null ? (
                <span className="tabular shrink-0 text-xs text-faint" title={`${hours} CPU hours in the window`}>{share}% of CPU time</span>
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
