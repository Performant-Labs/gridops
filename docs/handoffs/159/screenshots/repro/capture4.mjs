import { chromium } from "/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-gitlab-159-pool-pages-fe8fd1/c2780b4c-a68f-4a22-a2f0-a16b556459de/scratchpad/pw/node_modules/playwright/index.mjs";
import fs from "node:fs";
const [OUT, RUN] = process.argv.slice(2);
const cookie = fs.readFileSync(`${RUN}/cookie.txt`, "utf8").trim();
const avatar = fs.readFileSync(`${RUN}/avatar.png`);
const SERVERS = { baseline: 19411, live: 19412 };
const VIEWPORTS = [[1280, 800]];
const PAGES = [["gh-nine", "pool-gh-nine"], ["gh-nine-expanded", "pool-gh-nine"]];
const browser = await chromium.launch();
const problems = [];
const facts = {};
for (const [build, port] of Object.entries(SERVERS)) for (const [w, h] of VIEWPORTS) {
  const ctx = await browser.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: 1 });
  await ctx.addCookies([{ name: "gridops_session", value: cookie, url: `http://127.0.0.1:${port}` }]);
  await ctx.clock.setFixedTime(new Date("2026-11-21T10:00:00Z"));
  await ctx.route("**/*", (route) => { const u = route.request().url(); if (u.startsWith(`http://127.0.0.1:${port}`)) return route.continue(); if (/github\.com\/.+\.png/.test(u)) return route.fulfill({ status: 200, contentType: "image/png", body: avatar }); return route.abort(); });
  for (const [key, id] of PAGES) {
    const page = await ctx.newPage();
    page.on("console", (m) => { if (["error", "warning"].includes(m.type())) problems.push(`[${build} ${w} ${key}] ${m.type()}: ${m.text().slice(0, 200)}`); });
    await page.goto(`http://127.0.0.1:${port}/runner-pools/${id}`, { waitUntil: "networkidle" });
    const heading = page.locator("h3", { hasText: /^Destination$/ });
    await heading.waitFor({ timeout: 10000 });
    if (key.endsWith("-expanded")) await page.getByRole("button", { name: /Show \d+ more/ }).click();
    await heading.evaluate((e, atTop) => { let c = e.parentElement; while (c && !/(auto|scroll)/.test(getComputedStyle(c).overflowY)) c = c.parentElement; if (c) c.scrollTop = atTop ? 0 : c.scrollHeight; }, false);
    await page.waitForTimeout(700);
    await page.screenshot({ path: `${OUT}/${key}-dest-${w}-${build}.png` });
    const g = heading.locator("xpath=../..");
    facts[`${key}@${w}-${build}`] = { rows: await g.locator("dt").evaluateAll((els) => els.map((e) => `${e.textContent}: ${e.nextElementSibling?.textContent}`)), text: await g.textContent(), btn: (await g.locator("button").count()) ? await g.locator("button").first().textContent() : null, box: await g.boundingBox() };
    await page.close();
  }
  await ctx.close();
}
await browser.close();
fs.writeFileSync(`${OUT}/facts-dest3.json`, JSON.stringify({ facts, problems }, null, 1));
console.log("problems", problems.length, problems.slice(0, 5));
