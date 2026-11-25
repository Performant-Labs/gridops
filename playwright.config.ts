import { defineConfig, devices } from "@playwright/test";

/**
 * #256: the browser tests (`npm run test:a11y`), today the axe pass over the fleet health page,
 * the Overview and the job-categories settings (`tests/a11y/`). Headless Chromium only.
 *
 * By default it serves the built SPA (`dist/`, so `npm run build` first) with `vite preview` and
 * answers the API from recorded responses (`tests/a11y/api-fixtures.json`). With
 * `GRIDOPS_A11Y_BASE_URL` set it drives a running GridOps instead (the API serving the SPA, with
 * its own data); see `tests/a11y/fleet-health-axe.spec.ts`.
 */
const live = process.env.GRIDOPS_A11Y_BASE_URL;
const PREVIEW_PORT = 4317;

export default defineConfig({
  testDir: "tests/a11y",
  testMatch: "*.spec.ts",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  workers: process.env.CI ? 2 : undefined,
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
  use: {
    ...devices["Desktop Chrome"],
    baseURL: live ?? `http://127.0.0.1:${PREVIEW_PORT}`,
    headless: true,
    locale: "en-US",
    timezoneId: "America/Los_Angeles",
    reducedMotion: "reduce",
  },
  webServer: live
    ? undefined
    : {
        command: `npx vite preview --host 127.0.0.1 --port ${PREVIEW_PORT} --strictPort`,
        url: `http://127.0.0.1:${PREVIEW_PORT}`,
        reuseExistingServer: !process.env.CI,
        timeout: 60_000,
      },
});
