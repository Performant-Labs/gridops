/**
 * Score bands and their colours for every chart and badge that shows a 0–100 score (the fleet
 * health page, its Overview tile). Colours are the theme's CSS variables (`src/styles/app.css`), so
 * they switch with `.dark`; there is no hex palette here. The thresholds are `successTone`'s
 * (`src/features/dashboard/metric-tiles.ts`): good from 90, fair from 70, poor below.
 */
import type { HealthTone } from "~/components/status-icon";

export type ScoreBand = "good" | "fair" | "poor" | "none";

export const GOOD_SCORE = 90;
export const FAIR_SCORE = 70;

/** The band of a score; `none` when there is no score (not enough data). */
export function scoreBand(score: number | null): ScoreBand {
  if (score === null) return "none";
  return score >= GOOD_SCORE ? "good" : score >= FAIR_SCORE ? "fair" : "poor";
}

const BAND_TONES: Record<ScoreBand, HealthTone> = { good: "success", fair: "warning", poor: "danger", none: "neutral" };

/** The status tone of a band, shared by the gauge, the cards, the machine table and the tile. */
export const bandTone = (band: ScoreBand): HealthTone => BAND_TONES[band];

/** The colour of a band, as a CSS variable: for gauges, bars and dots. */
export const bandColor = (band: ScoreBand) => `var(--${bandTone(band)})`;

const BAND_TEXT: Record<ScoreBand, string> = {
  good: "var(--success-text)",
  fair: "var(--warning-text)",
  poor: "var(--danger)",
  none: "var(--muted-foreground)",
};

/**
 * The colour of a band's name written as text ("Good", "Poor"): the status colours are too light
 * for text in the light theme, so text takes their `-text` variants, which read at 4.5:1 (#256).
 */
export const bandTextColor = (band: ScoreBand) => BAND_TEXT[band];

/** The colour of the empty part of a gauge or bar. */
export const TRACK_COLOR = "var(--border)";
