/**
 * #256: axe over the fleet health page, the Overview (with its Fleet health tile) and the
 * job-categories settings, in light and dark at 360 px and 1280 px. A violation of serious or
 * critical impact fails the test, under the tags #204's walkthrough used (WCAG 2.0, 2.1 and 2.2
 * A/AA, plus best practice). Run with `npm run test:a11y` after `npm run build`.
 *
 * Two modes:
 * - Recorded (the default, and CI): `vite preview` serves `dist/`, and every `/api/v1` request is
 *   answered from `tests/a11y/api-fixtures.json`, responses recorded from a seeded GridOps (the
 *   #204 walkthrough's fleet, `seed_for_walkthrough` in `crates/gridops-api/src/fleet/
 *   health_e2e_tests.rs`) for an administrator and for a member.
 * - Live: `GRIDOPS_A11Y_BASE_URL` names a running GridOps (the API serving the built SPA) and
 *   `GRIDOPS_A11Y_ADMIN_SESSION` / `GRIDOPS_A11Y_MEMBER_SESSION` the `gridops_session` cookie
 *   values to sign in with. Nothing is stubbed. With `GRIDOPS_A11Y_RECORD=<file>` as well (and
 *   `--workers=1`), the API's answers are written to `<file>`: that is how the fixtures were made
 *   (`api.ts`; the recipe, with #269's extra screen data, is in `README.md`).
 *
 * Either way, `GRIDOPS_A11Y_REPORT=<file>` appends one JSON line per scan: the state, the width and
 * theme, and its violations by impact (moderate and minor ones too), for a written record.
 */
import { appendFileSync } from "node:fs";

import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { open, saveRecordingAfterAll } from "./api";

const REPORT = process.env.GRIDOPS_A11Y_REPORT;
const THEMES = ["light", "dark"] as const;
const WIDTHS = [360, 1280] as const;
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa", "best-practice"];
const BLOCKING = new Set(["serious", "critical"]);

saveRecordingAfterAll();

/**
 * Runs axe over the whole page; fails on any serious or critical violation, listing each node. The
 * check is soft, so a test reports every state it scans rather than stopping at the first.
 */
async function expectNoBlockingViolations(page: Page, state: string) {
  const results = await new AxeBuilder({ page }).withTags(TAGS).analyze();
  if (REPORT) {
    const viewport = page.viewportSize();
    const theme = (await page.locator("html").getAttribute("class"))?.includes("dark") ? "dark" : "light";
    const nodes = (impact: string) => results.violations.filter((violation) => violation.impact === impact).reduce((sum, violation) => sum + violation.nodes.length, 0);
    const rules = results.violations.map((violation) => `${violation.impact} ${violation.id} ×${violation.nodes.length}`);
    appendFileSync(REPORT, `${JSON.stringify({ state, width: viewport?.width, theme, critical: nodes("critical"), serious: nodes("serious"), moderate: nodes("moderate"), minor: nodes("minor"), rules })}\n`);
  }
  const blocking = results.violations
    .filter((violation) => BLOCKING.has(violation.impact ?? ""))
    .flatMap((violation) =>
      violation.nodes.map((node) => `${violation.impact} ${violation.id}: ${node.target.join(" ")} (${node.failureSummary?.split("\n").slice(1).join("; ").trim()})`),
    );
  expect.soft(blocking, `${state}: serious or critical axe violations`).toEqual([]);
}

/**
 * The health page's machine rows, and `choose(window)`: switches to `window` with the picker and
 * waits for its answer to be on screen. Until then the page shows loading rows, which a scan must
 * not mistake for the window.
 */
function healthPage(page: Page) {
  const picker = page.getByRole("group", { name: "Window" });
  const rows = page.locator('[data-health-section="machine-table"] button[aria-expanded]');
  const choose = async (window: string) => {
    const button = picker.locator("button").filter({ hasText: new RegExp(`^${window}$`) });
    if ((await button.getAttribute("aria-pressed")) !== "true") {
      const answered = page.waitForResponse((response) => {
        const url = new URL(response.url());
        return url.pathname === "/api/v1/fleet/health" && url.searchParams.get("window") === window;
      });
      await button.click();
      await answered;
    }
    await expect(picker.getByRole("button", { pressed: true })).toHaveText(window);
    await expect(rows.first()).toBeVisible();
    await page.waitForLoadState("networkidle");
  };
  return { choose, rows };
}

/**
 * #258 item 2: the tick labels of every chart on the page lie inside the chart's SVG, which clips
 * what falls outside it (the usage chart's last time label was cut off at the right edge).
 */
async function expectTicksInsideCharts(page: Page, state: string) {
  const clipped = await page.locator("svg.recharts-surface").evaluateAll((charts) =>
    charts.flatMap((chart, index) => {
      const box = chart.getBoundingClientRect();
      return [...chart.querySelectorAll(".recharts-cartesian-axis-tick-value")].flatMap((tick) => {
        const label = tick.getBoundingClientRect();
        const inside = label.left >= box.left - 0.5 && label.right <= box.right + 0.5 && label.top >= box.top - 0.5 && label.bottom <= box.bottom + 0.5;
        return inside ? [] : [`chart ${index + 1}: "${tick.textContent}" at ${Math.round(label.left - box.left)}–${Math.round(label.right - box.left)} px of ${Math.round(box.width)}`];
      });
    }),
  );
  expect.soft(clipped, `${state}: chart labels cut off`).toEqual([]);
}

for (const theme of THEMES) {
  for (const width of WIDTHS) {
    test.describe(`${width} px, ${theme}`, () => {
      test.use({ viewport: { width, height: width < 640 ? 780 : 900 } });

      test("the Overview and its Fleet health tile", async ({ page }) => {
        await open(page, { path: "/", role: "admin", theme, ready: 'a[href="/fleet/health"]:has-text("Fleet health")' });
        await expectNoBlockingViolations(page, "overview");
        await expectTicksInsideCharts(page, "overview");
      });

      test("the fleet health page on each window, switched to with the picker", async ({ page }) => {
        await open(page, { path: "/fleet/health", role: "admin", theme, ready: '[data-testid="fleet-health-sections"]' });
        const { choose } = healthPage(page);
        for (const window of ["1h", "7d", "24h"]) {
          await choose(window);
          await expectNoBlockingViolations(page, `health ${window}`);
        }
      });

      for (const window of ["24h", "1h", "7d"]) {
        test(`the fleet health page on ${window}: every concern, each machine expanded`, async ({ page }) => {
          // Five machines, each scanned: slower than the default 30 s on a busy runner.
          test.setTimeout(120_000);
          await open(page, { path: "/fleet/health", role: "admin", theme, ready: '[data-testid="fleet-health-sections"]', window });
          const { choose, rows } = healthPage(page);
          await choose(window);
          const showAll = page.getByRole("button", { name: /^Show all/ });
          if (await showAll.count()) await showAll.click();
          const machines = await rows.count();
          for (let index = 0; index < machines; index += 1) {
            const row = rows.nth(index);
            if ((await row.getAttribute("aria-expanded")) !== "true") await row.click();
            await expect(row).toHaveAttribute("aria-expanded", "true");
            const machine = `health ${window}, every concern, machine ${index + 1} expanded`;
            // The usage chart's tooltip is scanned too: hovering the chart draws it (#256).
            const usage = page.locator(`[id="${await row.getAttribute("aria-controls")}"] [data-testid="machine-usage-chart"]`);
            if (await usage.count()) {
              await usage.hover();
              await expect(usage.locator(".recharts-tooltip-wrapper")).toContainText(" h");
            }
            await expectNoBlockingViolations(page, machine);
            await expectTicksInsideCharts(page, machine);
          }
        });
      }

      test("the job-categories settings", async ({ page }) => {
        await open(page, { path: "/settings/general", role: "admin", theme, ready: 'text="Job categories"' });
        await expectNoBlockingViolations(page, "settings › general (job categories)");
      });

      test("a member: the Overview asks for no fleet health, and the page says administrators only", async ({ page }) => {
        const healthRequests: string[] = [];
        page.on("request", (request) => {
          if (new URL(request.url()).pathname === "/api/v1/fleet/health") healthRequests.push(request.url());
        });
        await open(page, { path: "/", role: "member", theme, ready: 'section[aria-label="Runner metrics"]' });
        expect.soft(healthRequests, "the Overview asked a member's fleet health").toEqual([]);
        await expectNoBlockingViolations(page, "member overview");
        await page.goto("/fleet/health");
        await expect(page.getByText("Administrators only")).toBeVisible();
        await page.waitForLoadState("networkidle");
        await expectNoBlockingViolations(page, "member /fleet/health");
      });
    });
  }
}
