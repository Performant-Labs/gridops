/**
 * #256: every text colour of the theme reads at 4.5:1 or more (WCAG 1.4.3, normal-size text) on
 * the surfaces text sits on, in light and in dark. #204's walkthrough found `--faint` at 2.44:1 and
 * the light `--danger` at 3.91:1, used for readable text across the app. The values are read from
 * `src/styles/app.css` itself, so a later palette change is checked too.
 */
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("../src/styles/app.css", import.meta.url), "utf8");

/** The custom properties a block (`:root {` or `.dark {`) sets, by name. */
function block(selector: string): Record<string, string> {
  const start = css.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`no ${selector} block in app.css`);
  const body = css.slice(start, css.indexOf("\n}", start));
  return Object.fromEntries([...body.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})\s*;/gi)].map(([, name, value]) => [name, value!.toLowerCase()]));
}

const light = block(":root");
const dark = { ...light, ...block(".dark") };

function luminance(hex: string) {
  const channels = [1, 3, 5].map((index) => Number.parseInt(hex.slice(index, index + 2), 16) / 255);
  const [r, g, b] = channels.map((value) => (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4)) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string) {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (high + 0.05) / (low + 0.05);
}

/** Surfaces text sits on: at rest (panel, subtle panel, canvas) and under the pointer (hover). */
const SURFACES = ["panel", "panel-subtle", "background", "hover"];
/** Every token used as a text colour (`--destructive` is a fill under white text, checked below). */
const TEXT = ["foreground", "secondary-foreground", "muted-foreground", "faint", "danger", "success-text", "warning-text", "progress-text", "info-text"];

describe.each([["light", light], ["dark", dark]] as const)("the %s theme", (_name, theme) => {
  it.each(TEXT)("--%s reads at 4.5:1 on every surface", (token) => {
    const color = theme[token];
    expect(color, `--${token} is not set`).toBeDefined();
    const failing = SURFACES.map((surface) => ({ surface, ratio: contrast(color!, theme[surface]!) })).filter(({ ratio }) => ratio < 4.5);
    expect(failing.map(({ surface, ratio }) => `--${surface}: ${ratio.toFixed(2)}:1`)).toEqual([]);
  });

  it("white text on a destructive button reads at 4.5:1", () => {
    expect(contrast("#ffffff", theme["destructive"]!)).toBeGreaterThanOrEqual(4.5);
  });
});
