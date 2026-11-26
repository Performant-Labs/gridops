/**
 * #269: each phone check (`mobile-checks.ts`) catches its kind of problem on a small test page,
 * and passes the same page without it, so a screen that passes `mobile.spec.ts` has none.
 */
import { expect, test, type Page } from "@playwright/test";

import { findMobileProblems } from "./mobile-checks";

test.use({ viewport: { width: 360, height: 780 }, isMobile: true, hasTouch: true });

/** A page shaped like the app: a fixed shell, `#main`, and a scrolling page body inside it. */
async function page(page: Page, body: string) {
  await page.setContent(`<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1"><style>
    body { margin: 0; font: 14px/1.4 sans-serif; }
    #shell { position: fixed; inset: 0; display: flex; overflow: hidden; }
    #main { display: flex; flex: 1; flex-direction: column; min-width: 0; overflow: hidden; }
    .body { flex: 1; min-height: 0; overflow: auto; padding: 16px; }
    .truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
    button { font: inherit; }
  </style></head><body><div id="shell"><a class="sr-only" href="#main">Skip to content</a><main id="main"><div class="body">${body}</div></main></div></body></html>`);
  return findMobileProblems(page);
}

const NONE = { sideways: [], smallTargets: [], smallText: [], truncated: [] };

test("a page with none of the problems passes", async ({ page: p }) => {
  expect(await page(p, `
    <h1>Runners</h1>
    <p>Read the <a href="/docs">runner docs</a> before you add one.</p>
    <div style="display: flex; gap: 8px"><button style="min-width: 24px; min-height: 24px">A</button><button style="min-width: 24px; min-height: 24px">B</button></div>
    <div style="display: flex; gap: 24px; margin-top: 24px"><button style="width: 16px; height: 16px; padding: 0">x</button><button style="width: 16px; height: 16px; padding: 0">y</button></div>
    <p class="truncate" style="width: 200px">acme/app · unit tests</p>
    <p class="truncate" style="width: 160px">acme/app · unit testss</p>
    <p style="font-size: 11px">Last seen 4 min ago</p>`)).toEqual(NONE);
});

test("it catches a page body wider than the phone", async ({ page: p }) => {
  const problems = await page(p, `<h1>Runs</h1><table style="width: 640px"><tr><td>a very wide table</td></tr></table><div style="height: 1200px"></div>`);
  expect(problems.sideways).toEqual([expect.stringMatching(/^the scrolling panel is \d+ px wide in \d+ px; widest: <table>/)]);
});

test("a wide table may scroll sideways inside its own scroller", async ({ page: p }) => {
  const problems = await page(p, `<h1>Runs</h1><div style="overflow-x: auto"><table style="width: 640px; height: 400px"><tr><td>a very wide, tall table</td></tr></table></div><div style="height: 1200px"></div>`);
  expect(problems.sideways).toEqual([]);
});

test("it catches a document wider than the phone", async ({ page: p }) => {
  await p.setContent(`<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1"><body style="margin: 0"><div style="width: 500px">wide</div></body>`);
  expect((await findMobileProblems(p)).sideways).toEqual([expect.stringMatching(/^the document is 500 px wide in a 360 px viewport$/)]);
});

test("it catches small targets that crowd each other, not small ones with room", async ({ page: p }) => {
  const problems = await page(p, `
    <div style="display: flex; gap: 2px"><button aria-label="Edit" style="width: 16px; height: 16px; padding: 0"></button><button aria-label="Delete" style="width: 16px; height: 16px; padding: 0"></button></div>
    <div style="display: flex; gap: 24px; margin-top: 40px"><button aria-label="Up" style="width: 16px; height: 16px; padding: 0"></button><button aria-label="Down" style="width: 16px; height: 16px; padding: 0"></button></div>`);
  expect(problems.smallTargets).toEqual([
    '<button> "Edit" is 16 × 16 px, too close to <button> "Delete"',
    '<button> "Delete" is 16 × 16 px, too close to <button> "Edit"',
  ]);
});

test("it catches small targets whose 24 px circles overlap, though neither circle reaches the other button", async ({ page: p }) => {
  // Centres 22 px apart: each circle stops 2 px short of the other button, but the circles overlap.
  const problems = await page(p, `<div style="display: flex; gap: 6px"><button aria-label="Up" style="width: 16px; height: 16px; padding: 0"></button><button aria-label="Down" style="width: 16px; height: 16px; padding: 0"></button></div>`);
  expect(problems.smallTargets).toEqual([
    '<button> "Up" is 16 × 16 px, too close to <button> "Down"',
    '<button> "Down" is 16 × 16 px, too close to <button> "Up"',
  ]);
});

test("it catches text under 11 px, once per place", async ({ page: p }) => {
  const problems = await page(p, `<div>Go to Runs <kbd style="font-size: 10px">G</kbd> then <kbd style="font-size: 10px">R</kbd></div><p style="font-size: 11px">fine</p>`);
  expect(problems.smallText).toEqual(['10 px in <div> "Go to Runs G then R"']);
});

test("it catches an ellipsis that hides more than a few characters", async ({ page: p }) => {
  const problems = await page(p, `<p class="truncate" style="width: 120px">The sandbox could not reach the package registry</p>`);
  expect(problems.truncated).toEqual([expect.stringMatching(/^<p> "The sandbox could not reach the package " hides about \d+ characters$/)]);
});
