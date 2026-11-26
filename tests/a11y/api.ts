/**
 * The browser tests' API (#256, #269): every `/api/v1` request answered from
 * `tests/a11y/api-fixtures.json` (recorded mode, the default and CI), or, with
 * `GRIDOPS_A11Y_BASE_URL`, a running GridOps signed in with `GRIDOPS_A11Y_ADMIN_SESSION` /
 * `GRIDOPS_A11Y_MEMBER_SESSION`. With `GRIDOPS_A11Y_RECORD=<file>` as well (and `--workers=1`), the
 * live answers are merged into `<file>`: that is how the fixtures are made. The recording recipe is
 * in `tests/a11y/README.md`.
 */
import { existsSync, readFileSync, writeFileSync } from "node:fs";

import { expect, test, type Page, type Request } from "@playwright/test";

/** Who is signed in: an administrator, a member, or nobody (the sign-in page). */
export type Role = "admin" | "member" | "anonymous";
export type Theme = "light" | "dark";
type Recorded = { status: number; body: unknown };
type Fixtures = Record<string, Recorded>;

export const LIVE = process.env.GRIDOPS_A11Y_BASE_URL;
const RECORD = process.env.GRIDOPS_A11Y_RECORD;
const FIXTURES_PATH = new URL("./api-fixtures.json", import.meta.url);
const recorded: Fixtures = {};
/** The fixtures' key for when they were recorded (ms since the epoch): the page's clock in recorded mode. */
const RECORDED_AT = "__recordedAt";

const fixtures: Fixtures = LIVE ? {} : (JSON.parse(readFileSync(FIXTURES_PATH, "utf8")) as Fixtures);

/** The key of a request in the fixtures: role, method and path with its query. */
function fixtureKey(role: Role, request: Request) {
  const url = new URL(request.url());
  return `${role} ${request.method()} ${url.pathname}${url.search}`;
}

/** Answers `/api/v1` from the fixtures (recorded mode), or records the live answers. */
export async function serveApi(page: Page, role: Role) {
  if (LIVE) {
    if (role !== "anonymous") {
      const cookie = process.env[role === "admin" ? "GRIDOPS_A11Y_ADMIN_SESSION" : "GRIDOPS_A11Y_MEMBER_SESSION"];
      if (!cookie) throw new Error(`GRIDOPS_A11Y_${role.toUpperCase()}_SESSION is required with GRIDOPS_A11Y_BASE_URL`);
      await page.context().addCookies([{ name: "gridops_session", value: cookie, url: LIVE }]);
    }
    if (RECORD) {
      page.on("response", async (response) => {
        if (!new URL(response.url()).pathname.startsWith("/api/v1/")) return;
        const body = await response.json().catch(() => undefined);
        if (body !== undefined) recorded[fixtureKey(role, response.request())] = { status: response.status(), body };
      });
    }
    return;
  }
  // The answers hold absolute times; the page shows them relative to now ("12m ago"). Fixing the
  // page's clock at the recording time keeps that text, and so what it wraps or cuts off, stable.
  const recordedAt = fixtures[RECORDED_AT]?.body;
  if (typeof recordedAt === "number") await page.clock.setFixedTime(recordedAt);
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const key = fixtureKey(role, request);
    // Exact matches only (query included): a scan of one window never gets another's answer.
    const answer = fixtures[key];
    if (!answer) {
      await route.fulfill({ status: 404, json: { error: `Not recorded: ${key}`, code: "not_found" } });
      return;
    }
    await route.fulfill({ status: answer.status, json: answer.body });
  });
}

/** Writes what this file's tests recorded into `GRIDOPS_A11Y_RECORD`, merged with what is there. */
export function saveRecordingAfterAll() {
  test.afterAll(() => {
    if (!RECORD || Object.keys(recorded).length === 0) return;
    const previous = existsSync(RECORD) ? (JSON.parse(readFileSync(RECORD, "utf8")) as Fixtures) : {};
    const merged: Fixtures = { ...previous, ...recorded, [RECORDED_AT]: { status: 200, body: Date.now() } };
    const sorted = Object.fromEntries(Object.keys(merged).sort().map((key) => [key, merged[key]]));
    writeFileSync(RECORD, `${JSON.stringify(sorted, null, 2)}\n`);
  });
}

/** Opens `path` signed in as `role`, in `theme` (and on the health `window`), and waits for `ready`. */
export async function open(page: Page, { path, role, theme, ready, window: healthWindow }: { path: string; role: Role; theme: Theme; ready: string; window?: string }) {
  await page.addInitScript((value) => window.localStorage.setItem("gridops-theme", value), theme);
  if (healthWindow) await page.addInitScript((value) => window.localStorage.setItem("gridops.fleetHealth.window", value), healthWindow);
  await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
  await serveApi(page, role);
  await page.goto(path);
  await expect(page.locator(ready).first()).toBeVisible();
  await expect(page.locator("html")).toHaveClass(theme === "dark" ? /\bdark\b/ : /^(?!.*\bdark\b)/);
  await page.waitForLoadState("networkidle");
}
