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
 *   `--workers=1`), the API's answers are written to `<file>`: that is how the fixtures were made.
 *
 * Either way, `GRIDOPS_A11Y_REPORT=<file>` appends one JSON line per scan: the state, the width and
 * theme, and its violations by impact (moderate and minor ones too), for a written record.
 */
import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";

import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page, type Request } from "@playwright/test";

type Role = "admin" | "member";
type Recorded = { status: number; body: unknown };
type Fixtures = Record<string, Recorded>;

const LIVE = process.env.GRIDOPS_A11Y_BASE_URL;
const RECORD = process.env.GRIDOPS_A11Y_RECORD;
const REPORT = process.env.GRIDOPS_A11Y_REPORT;
const FIXTURES_PATH = new URL("./api-fixtures.json", import.meta.url);
const THEMES = ["light", "dark"] as const;
const WIDTHS = [360, 1280] as const;
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa", "best-practice"];
const BLOCKING = new Set(["serious", "critical"]);
const recorded: Fixtures = {};

const fixtures: Fixtures = LIVE ? {} : (JSON.parse(readFileSync(FIXTURES_PATH, "utf8")) as Fixtures);

/** The key of a request in the fixtures: role, method and path with its query. */
function fixtureKey(role: Role, request: Request) {
  const url = new URL(request.url());
  return `${role} ${request.method()} ${url.pathname}${url.search}`;
}

/** Answers `/api/v1` from the fixtures (recorded mode), or records the live answers. */
async function serveApi(page: Page, role: Role) {
  if (LIVE) {
    const cookie = process.env[role === "admin" ? "GRIDOPS_A11Y_ADMIN_SESSION" : "GRIDOPS_A11Y_MEMBER_SESSION"];
    if (!cookie) throw new Error(`GRIDOPS_A11Y_${role.toUpperCase()}_SESSION is required with GRIDOPS_A11Y_BASE_URL`);
    await page.context().addCookies([{ name: "gridops_session", value: cookie, url: LIVE }]);
    if (RECORD) {
      page.on("response", async (response) => {
        if (!new URL(response.url()).pathname.startsWith("/api/v1/")) return;
        const body = await response.json().catch(() => undefined);
        if (body !== undefined) recorded[fixtureKey(role, response.request())] = { status: response.status(), body };
      });
    }
    return;
  }
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const key = fixtureKey(role, request);
    const pathOnly = key.replace(/\?.*$/, "");
    const answer = fixtures[key] ?? Object.entries(fixtures).find(([other]) => other.replace(/\?.*$/, "") === pathOnly)?.[1];
    if (!answer) {
      await route.fulfill({ status: 404, json: { error: `Not recorded: ${key}`, code: "not_found" } });
      return;
    }
    await route.fulfill({ status: answer.status, json: answer.body });
  });
}

test.afterAll(() => {
  if (!RECORD || Object.keys(recorded).length === 0) return;
  const previous = existsSync(RECORD) ? (JSON.parse(readFileSync(RECORD, "utf8")) as Fixtures) : {};
  const merged = { ...previous, ...recorded };
  const sorted = Object.fromEntries(Object.keys(merged).sort().map((key) => [key, merged[key]]));
  writeFileSync(RECORD, `${JSON.stringify(sorted, null, 2)}\n`);
});

/** Opens `path` signed in as `role`, in `theme`, and waits for `ready`. */
async function open(page: Page, { path, role, theme, ready }: { path: string; role: Role; theme: "light" | "dark"; ready: string }) {
  await page.addInitScript((value) => window.localStorage.setItem("gridops-theme", value), theme);
  await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
  await serveApi(page, role);
  await page.goto(path);
  await expect(page.locator(ready).first()).toBeVisible();
  await expect(page.locator("html")).toHaveClass(theme === "dark" ? /\bdark\b/ : /^(?!.*\bdark\b)/);
  await page.waitForLoadState("networkidle");
}

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

      test("the fleet health page: every window, every concern, each machine expanded", async ({ page }) => {
        await open(page, { path: "/fleet/health", role: "admin", theme, ready: '[data-testid="fleet-health-sections"]' });
        const picker = page.getByRole("group", { name: "Window" });
        for (const window of ["1h", "7d", "24h"]) {
          await picker.locator("button").filter({ hasText: new RegExp(`^${window}$`) }).click();
          await expect(picker.getByRole("button", { pressed: true })).toHaveText(window);
          await page.waitForLoadState("networkidle");
          await expectNoBlockingViolations(page, `health ${window}`);
        }
        const showAll = page.getByRole("button", { name: /^Show all/ });
        if (await showAll.count()) await showAll.click();
        const rows = page.locator('[data-health-section="machine-table"] button[aria-expanded]');
        await expect(rows.first()).toBeVisible();
        for (const window of ["24h", "1h", "7d"]) {
          await picker.locator("button").filter({ hasText: new RegExp(`^${window}$`) }).click();
          await page.waitForLoadState("networkidle");
          for (let index = 0; index < (await rows.count()); index += 1) {
            const row = rows.nth(index);
            if ((await row.getAttribute("aria-expanded")) !== "true") await row.click();
            await expect(row).toHaveAttribute("aria-expanded", "true");
            const machine = `health ${window}, every concern, machine ${index + 1} expanded`;
            if (window === "24h") await expectNoBlockingViolations(page, machine);
            await expectTicksInsideCharts(page, machine);
          }
        }
      });

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
