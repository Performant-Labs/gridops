/**
 * #269 (epic #268): every screen at phone width (360 and 390 px), light and dark, in each of its
 * states (empty, loaded, a row expanded, a dialog open), checked for what a script can check: no sideways
 * scrolling, targets of at least 24 px (WCAG 2.2 2.5.8), no text under 11 px, and no ellipsis
 * hiding text that has no other form. The checks are in `mobile-checks.ts`; `mobile-checks.spec.ts`
 * shows each one catching its problem.
 *
 * One entry per route in `src/routes`. A screen that fails today is `fixme` with the epic issue
 * that fixes it, and that issue turns its check on. The data is the API answers in
 * `api-fixtures.json`, recorded from the #204 walkthrough seed plus `seed-screens.py` (the recipe
 * is in `README.md`).
 *
 * `GRIDOPS_MOBILE_SHOTS=<dir>` saves screenshots of every screen and state into `<dir>` (outside
 * the repository), in segments, since the app scrolls inside a panel. `GRIDOPS_MOBILE_ALL=1` runs
 * the fixme'd screens too, to see what they still fail.
 */
import { mkdirSync } from "node:fs";

import { expect, test, type Page } from "@playwright/test";

import { type EditAnswer, open, saveRecordingAfterAll, type Role, type Theme } from "./api";
import { findMobileProblems, saveScreens } from "./mobile-checks";

const SHOTS = process.env.GRIDOPS_MOBILE_SHOTS;
const RUN_ALL = Boolean(process.env.GRIDOPS_MOBILE_ALL);
const WIDTHS = [360, 390] as const;
const THEMES: Theme[] = ["light", "dark"];

const POOL = "/runner-pools/0f000004-0000-4000-8000-000000000001";
const IO = "/fleet/machines/0f000001-0000-4000-8000-000000000001";

type State = { name: string; act: (page: Page) => Promise<void> };
type Screen = {
  /** The epic issue that owns the screen. */
  issue: number;
  /** The route file in `src/routes`. */
  route: string;
  path: string;
  /** A selector that is visible once the screen has loaded. */
  ready: string;
  /** Where the path lands, when the route redirects. */
  lands?: string;
  /** Who is signed in; the administrator unless said. */
  role?: Role;
  /** Tells apart two entries for one route, as in "empty". */
  variant?: string;
  states?: State[];
  /** Failing today: the check is `fixme` until `issue` fixes it. */
  fixme?: boolean;
  /** Changes the recorded answers, for data the recording lacks. */
  edit?: EditAnswer;
};

const heading = (text: string) => `h1:has-text("${text}"):visible`;

/** Clicks the first `name` button and waits for `expanded` (or a dialog) to show. */
const expand = (name: string | RegExp): State["act"] => async (page) => {
  const button = page.getByRole("button", { name }).first();
  await button.click();
  await expect(button).toHaveAttribute("aria-expanded", "true");
};
const dialog = (name: string | RegExp): State["act"] => async (page) => {
  await page.getByRole("button", { name }).first().click();
  await expect(page.getByRole("dialog").first()).toBeVisible();
  await page.waitForLoadState("networkidle");
};

/**
 * Creates an API token, answered here with a made-up token (no secret is recorded or real), and
 * waits for the dialog that shows its secret once.
 */
const createToken: State["act"] = async (page) => {
  const secret = `gat_${"Ab1-_x9Z".repeat(5)}Ab0`;
  await page.route(/\/api\/v1\/.*api-tokens/, async (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    const now = await page.evaluate(() => Date.now());
    const iso = (ms: number) => new Date(ms).toISOString();
    await route.fulfill({ status: 201, json: {
      id: "0f0000aa-0000-4000-8000-000000000001", name: "ci deploy", displayPrefix: secret.slice(4, 12), scopes: ["fleet:read"],
      createdAt: iso(now), expiresAt: iso(now + 90 * 24 * 3_600_000), lastUsedAt: null, revokedAt: null, tokenSecret: secret,
    } });
  });
  await page.locator("#api-token-name").fill("ci deploy");
  const scope = page.locator('[id="api-token-scope-fleet:read"]');
  if ((await scope.getAttribute("aria-checked")) !== "true") await scope.click();
  await page.getByRole("button", { name: "Create token" }).click();
  await expect(page.getByRole("dialog")).toContainText("ci deploy");
};

/**
 * The Overview with what the recording lacks (#271): activity with long messages, machine
 * performance with and without a speed, and enough kinds of failure to fill a tile's hint.
 * Made up here, as the rest of the fixtures are.
 */
const busyOverview: EditAnswer = (path, body) => {
  // By path alone, so a change to the query doesn't drop the data; the state below checks it shows.
  const pathname = path.split("?")[0];
  if (pathname === "/api/v1/agent-runs") {
    const runs = body as { items: { title: string | null }[] };
    if (runs.items[0]) runs.items[0].title = "Pin the flaky snapshot test's clock to a fixed time zone so it passes in CI";
    return runs;
  }
  if (pathname !== "/api/v1/overview") return body;
  const overview = body as { recordedAt?: string; machines: unknown; activity: unknown; slo: { failures: unknown } };
  const at = (minutesAgo: number) => new Date(Date.now() - minutesAgo * 60_000).toISOString();
  overview.slo.failures = [{ reason: "failure", count: 12 }, { reason: "startup_failure", count: 3 }, { reason: "timed_out", count: 2 }, { reason: "action_required", count: 1 }];
  overview.machines = { windowDays: 7, items: [
    { id: IO.split("/").pop(), name: "io", os: "linux", architecture: "x64", jobs: 42, succeeded: 40, failed: 2, medianSeconds: 312, relativePercent: 82, comparedJobs: 17, fastest: true },
    { id: "0f000001-0000-4000-8000-000000000002", name: "callisto", os: "macos", architecture: "arm64", jobs: 2, succeeded: 2, failed: 0, medianSeconds: 540, relativePercent: null, comparedJobs: 0, fastest: false },
  ] };
  overview.activity = [
    { id: "activity-1", level: "error", event: "runner_start_failed", message: "The runner on callisto did not register with GitHub within 5 minutes; the machine was left out of new placements.", runnerId: "0f000003-0000-4000-8000-000000000001", poolId: null, createdAt: at(4) },
    { id: "activity-2", level: "warning", event: "pool_scaled", message: "linux-docker asked for 3 runners for 3 queued jobs but the fleet has room for 2.", runnerId: null, poolId: POOL.split("/").pop(), createdAt: at(12) },
    { id: "activity-3", level: "info", event: "runner_assigned", message: "unit tests in acme/app (CI) went to io.", runnerId: null, poolId: null, createdAt: at(30) },
  ];
  return overview;
};

export const SCREENS: Screen[] = [
  // #270: the app shell, navigation and sign-in.
  { issue: 270, route: "login.tsx", path: "/login", ready: heading("Log in to GridOps"), role: "anonymous" },
  { issue: 270, route: "_app.tsx", path: "/queued-jobs", ready: heading("Queued jobs"), states: [{ name: "navigation open", act: dialog("Open navigation") }] },
  { issue: 270, route: "settings.tsx", path: "/settings/general", ready: heading("General"), states: [{ name: "settings navigation open", act: dialog("Open navigation") }] },
  // #271: the Overview.
  { issue: 271, route: "_app/index.tsx", path: "/", ready: heading("Overview"), states: [{
    // A section's controls wrap below its heading on a phone rather than squeezing it onto two lines.
    name: "section headings on one line",
    act: async (page) => {
      const wrapped = await page.locator("#main h2:visible").evaluateAll((headings) => headings
        .filter((h2) => h2.getBoundingClientRect().height > parseFloat(getComputedStyle(h2).lineHeight) * 1.5)
        .map((h2) => h2.textContent));
      expect(wrapped).toEqual([]);
    },
  }] },
  { issue: 271, route: "_app/index.tsx", path: "/", ready: heading("Overview"), variant: "busy", edit: busyOverview, states: [{
    // A machine's speed is explained in a hover tooltip on a wider screen; a phone shows it. The
    // other edited data is checked to show too, so a request the edit stops matching fails here.
    name: "machine speed explained",
    act: async (page) => {
      await expect(page.getByText("Pin the flaky snapshot test's clock to a fixed time zone so it passes in CI")).toBeVisible();
      await expect(page.getByText("The runner on callisto did not register with GitHub")).toBeVisible();
      await expect(page.getByText("12 failed")).toBeVisible();
      await expect(page.getByText("Takes 82% of the usual time, compared on 17 jobs")).toBeVisible();
      await expect(page.getByText("Needs 3 successful jobs that also ran on another machine")).toBeVisible();
    },
  }] },
  // #272: fleet machines and a machine's page.
  { issue: 272, route: "_app/fleet.index.tsx", path: "/fleet", ready: heading("Machines"), states: [{ name: "add machine open", act: dialog("Add machine") }] },
  { issue: 272, route: "_app/fleet.machines.$hostId.tsx", path: IO, ready: heading("io"), states: [{ name: "agent log open", act: dialog("Agent log") }] },
  // #273: runner pools.
  { issue: 273, route: "_app/runner-pools.index.tsx", path: "/runner-pools", ready: heading("Runner pools") },
  { issue: 273, route: "_app/runner-pools.new.tsx", fixme: true, path: "/runner-pools/new", ready: heading("New pool") },
  { issue: 273, route: "_app/runner-pools.$poolId.tsx", fixme: true, path: POOL, ready: heading("linux-docker") },
  { issue: 273, route: "_app/runner-pools.$poolId.index.tsx", fixme: true, path: POOL, ready: heading("linux-docker") },
  { issue: 273, route: "_app/runner-pools.$poolId.activity.tsx", path: `${POOL}/activity`, ready: heading("linux-docker") },
  { issue: 273, route: "_app/runner-pools.$poolId.settings.tsx", path: `${POOL}/settings`, ready: heading("linux-docker") },
  // #274: runners and queued jobs.
  { issue: 274, route: "_app/runners.tsx", path: "/runners", ready: heading("Runners") },
  { issue: 274, route: "_app/queued-jobs.tsx", path: "/queued-jobs", ready: heading("Queued jobs") },
  // #275: workflow runs, a run's page and live logs.
  { issue: 275, route: "_app/workflow-runs.index.tsx", fixme: true, path: "/workflow-runs", ready: heading("Workflow runs") },
  { issue: 275, route: "_app/workflow-runs.$runId.tsx", fixme: true, path: "/workflow-runs/960002", ready: "h1" },
  { issue: 275, route: "_app/live-logs.tsx", path: "/live-logs", ready: heading("Live logs") },
  // #276: agent runs.
  { issue: 276, route: "_app/agent-runs.tsx", path: "/agent-runs", ready: heading("Agent runs"), states: [{ name: "run open", act: async (page) => {
      await dialog(/^Show agent run for/)(page);
      // The edited stage shows, so a request the edit stops matching fails here.
      await expect(page.getByRole("dialog")).toContainText("running the repository's tests");
    } }],
    // A stage longer than the recorded one, as the agent reports them: the run's sheet must show it whole.
    edit: (path, body) => path.startsWith("/api/v1/agent-runs") && body && typeof body === "object"
      ? { ...body, ...("stage" in body ? { stage: "running the repository's tests" } : {}), ...("items" in body ? { items: (body as { items: object[] }).items.map((item) => ("stage" in item && (item as { stage: unknown }).stage ? { ...item, stage: "running the repository's tests" } : item)) } : {}) }
      : body },
  // #277: repositories, platform connections and webhooks.
  { issue: 277, route: "_app/repositories.tsx", path: "/repositories", ready: heading("Repositories") },
  { issue: 277, route: "_app/platform-connections.tsx", fixme: true, path: "/platform-connections", lands: "/settings/connections", ready: 'h2:has-text("Connections")' },
  { issue: 277, route: "settings.connections.tsx", fixme: true, path: "/settings/connections", ready: 'h2:has-text("Connections")', states: [{ name: "add connection open", act: dialog("Add connection") }] },
  { issue: 277, route: "settings.github.tsx", fixme: true, path: "/settings/github", lands: "/settings/connections", ready: 'h2:has-text("Connections")' },
  { issue: 277, route: "settings.integrations.tsx", fixme: true, path: "/settings/integrations", lands: "/settings/connections", ready: 'h2:has-text("Connections")' },
  { issue: 277, route: "_app/webhooks.tsx", fixme: true, path: "/webhooks", ready: heading("Webhooks"), states: [{ name: "a delivery inspected", act: dialog(/^Inspect delivery/) }] },
  // #278: the audit log.
  { issue: 278, route: "_app/audit-log.tsx", path: "/audit-log", ready: heading("Audit log"), states: [{ name: "an event expanded", act: expand("Toggle event details") }] },
  // #279: settings.
  { issue: 279, route: "settings.index.tsx", path: "/settings", lands: "/settings/general", ready: heading("General") },
  { issue: 279, route: "settings.general.tsx", path: "/settings/general", ready: heading("General") },
  { issue: 279, route: "settings.preferences.tsx", fixme: true, path: "/settings/preferences", ready: heading("Preferences") },
  { issue: 279, route: "settings.members.tsx", fixme: true, path: "/settings/members", ready: heading("Members") },
  { issue: 279, route: "settings.api-tokens.tsx", path: "/settings/api-tokens", ready: heading("API tokens"), states: [{ name: "a new token's secret shown", act: createToken }] },
  { issue: 279, route: "settings.runner-host.tsx", path: "/settings/runner-host", ready: heading("Runner host") },
  { issue: 279, route: "settings.ai.tsx", path: "/settings/ai", ready: heading("AI agent") },
  // #280: fleet health (fixed by #281).
  { issue: 280, route: "_app/fleet.health.tsx", path: "/fleet/health", ready: '[data-testid="fleet-health-sections"]', states: [{ name: "a machine expanded", act: async (page) => {
    const row = page.locator('[data-health-section="machine-table"] button[aria-expanded]').first();
    await row.click();
    await expect(row).toHaveAttribute("aria-expanded", "true");
  } }] },
  // Empty: a member with no installation access sees each list's empty state. Each entry waits for
  // that state's own text, so a recording where the member sees rows fails instead of passing as
  // a second check of the loaded list.
  ...[
    { issue: 273, route: "_app/runner-pools.index.tsx", path: "/runner-pools", empty: "Create your first runner pool" },
    { issue: 274, route: "_app/runners.tsx", path: "/runners", empty: "No managed runners" },
    { issue: 274, route: "_app/queued-jobs.tsx", path: "/queued-jobs", empty: "No queued jobs" },
    { issue: 275, route: "_app/workflow-runs.index.tsx", path: "/workflow-runs", empty: "No workflow runs yet" },
    { issue: 275, route: "_app/live-logs.tsx", path: "/live-logs", empty: "No job logs yet" },
    { issue: 276, route: "_app/agent-runs.tsx", path: "/agent-runs", empty: "No agent runs yet" },
    { issue: 277, route: "_app/repositories.tsx", path: "/repositories", empty: "No repositories yet" },
  ].map(({ empty, ...screen }): Screen => ({ ...screen, ready: `#main :text-is("${empty}"):visible`, role: "member", variant: "empty" })),
  // The root layout: every screen above renders inside it.
  { issue: 270, route: "__root.tsx", path: "/login", ready: heading("Log in to GridOps"), role: "anonymous" },
];

saveRecordingAfterAll();

/** Fails, softly, on each kind of problem the screen has in `state`. */
async function expectReadableOnPhone(page: Page, state: string) {
  const problems = await findMobileProblems(page);
  expect.soft(problems.sideways, `${state}: scrolls sideways`).toEqual([]);
  expect.soft(problems.smallTargets, `${state}: targets under 24 px, too close together`).toEqual([]);
  expect.soft(problems.smallText, `${state}: text under 11 px`).toEqual([]);
  expect.soft(problems.truncated, `${state}: text cut off`).toEqual([]);
}

for (const theme of THEMES) {
  for (const width of WIDTHS) {
    test.describe(`${width} px, ${theme}`, () => {
      test.use({ viewport: { width, height: 780 }, isMobile: true, hasTouch: true });

      for (const screen of SCREENS) {
        const title = `#${screen.issue} ${screen.route} (${screen.path})${screen.variant ? `, ${screen.variant}` : ""}`;
        (screen.fixme && !RUN_ALL ? test.fixme : test)(title, async ({ page }) => {
          test.setTimeout(60_000);
          await open(page, { path: screen.path, role: screen.role ?? "admin", theme, ready: screen.ready, edit: screen.edit });
          if (screen.lands) expect(new URL(page.url()).pathname).toBe(screen.lands);
          const base = `${screen.route.replace(/[/$.]/g, "_")}${screen.variant ? `-${screen.variant}` : ""}-${width}-${theme}`;
          if (SHOTS) mkdirSync(SHOTS, { recursive: true });
          await expectReadableOnPhone(page, "loaded");
          if (SHOTS) await saveScreens(page, SHOTS, `${base}-loaded`);
          for (const state of screen.states ?? []) {
            await state.act(page);
            await expectReadableOnPhone(page, state.name);
            if (SHOTS) await saveScreens(page, SHOTS, `${base}-${state.name.replace(/\W+/g, "_")}`);
            await page.keyboard.press("Escape");
          }
        });
      }
    });
  }
}
