/**
 * The fleet health page's colours: the score bands (shared with every score chart, from
 * `~/components/charts/chart-theme`) and one theme variable per job category (`--category-*` in
 * `src/styles/app.css`, light and dark).
 */
import type { JobCategory } from "./types";

export { FAIR_SCORE, GOOD_SCORE, TRACK_COLOR, bandColor, bandTextColor, bandTone, scoreBand, type ScoreBand } from "~/components/charts/chart-theme";

const CATEGORY_LABELS: Record<JobCategory, string> = {
  test: "Test",
  build: "Build",
  check: "Check",
  deploy: "Deploy",
  "fix-agent": "Fix agent",
  other: "Other",
};

/** The colour of a job category, as a CSS variable. */
export const categoryColor = (category: JobCategory) => `var(--category-${category})`;

/** How a job category is named on the page. */
export const categoryLabel = (category: JobCategory) => CATEGORY_LABELS[category];
