import { chromium } from "/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-gitlab-159-pool-pages-fe8fd1/c2780b4c-a68f-4a22-a2f0-a16b556459de/scratchpad/pw/node_modules/playwright/index.mjs";
import fs from "node:fs";
const [OUT, RUN] = process.argv.slice(2);
const cookie = fs.readFileSync(`${RUN}/cookie.txt`, "utf8").trim();
const avatar = fs.readFileSync(`${RUN}/avatar.png`);
const SERVERS = { baseline: 19411, live: 19412 };
const VIEWPORTS = [[360, 800], [768, 1024], [1280, 800]];
const PAGES = [
  ["list", "/runner-pools", "gh-one"],
  ["gh-many", "/runner-pools/pool-gh-many", "Destination"],
  ["gh-nine", "/runner-pools/pool-gh-nine", "Destination"],
  ["gh-org", "/runner-pools/pool-gh-org", "Destination"],
  ["bb", "/runner-pools/pool-bb", "Destination"],
  ["gl-one", "/runner-pools/pool-gl-one", "Destination"],
  ["gl-two", "/runner-pools/pool-gl-two", "Destination"],
  ["gl-zero", "/runner-pools/pool-gl-zero", "Destination"],
  ["unknown", "/runner-pools/pool-unknown", "Destination"],
];
const browser = await chromium.launch();
const facts = {};
const problems = [];
for (const [build, port] of Object.entries(SERVERS)) {
  for (const [w, h] of VIEWPORTS) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: 1 });
    await ctx.addCookies([{ name: "gridops_session", value: cookie, url: `http://127.0.0.1:${port}` }]);
    await ctx.clock.setFixedTime(new Date("2026-11-21T10:00:00Z"));
    await ctx.route("**/*", (route) => {
      const url = route.request().url();
      if (url.startsWith(`http://127.0.0.1:${port}`)) return route.continue();
      if (/github\.com\/.+\.png/.test(url)) return route.fulfill({ status: 200, contentType: "image/png", body: avatar });
      return route.abort();
    });
    for (const [key, path, ready] of PAGES) {
      const page = await ctx.newPage();
      page.on("console", (m) => { if (["error", "warning"].includes(m.type())) problems.push(`[${build} ${w} ${key}] ${m.type()}: ${m.text().slice(0, 200)}`); });
      page.on("pageerror", (e) => problems.push(`[${build} ${w} ${key}] pageerror: ${String(e).slice(0, 200)}`));
      await page.goto(`http://127.0.0.1:${port}${path}`, { waitUntil: "networkidle" });
      await page.getByText(ready, { exact: false }).first().waitFor({ timeout: 10000 });
      await page.waitForTimeout(600);
      await page.screenshot({ path: `${OUT}/${key}-${w}-${build}.png`, fullPage: true });
      const f = await page.evaluate((isList) => {
        const o = { scrollW: document.documentElement.scrollWidth, clientW: document.documentElement.clientWidth };
        if (isList) {
          o.rows = [...document.querySelectorAll("a[aria-label^='Open ']")].map((a) => {
            const row = a.parentElement;
            const cell = [...row.querySelectorAll("[title]")].find((e) => !/busy · /.test(e.title) && !["svg", "img"].includes(e.tagName.toLowerCase()) && e.matches(".xl\\:flex"));
            return { pool: a.getAttribute("aria-label"), shown: !!cell && getComputedStyle(cell).display !== "none", text: cell?.textContent?.trim(), title: cell?.title, img: cell?.querySelector("img")?.getAttribute("src") ?? null, svg: cell?.querySelector("svg")?.getAttribute("class")?.split(/\s+/).find((c) => c.startsWith("lucide-")) ?? null, placeholder: !!cell?.querySelector("span[aria-hidden='true']") };
          });
        } else {
          const h = [...document.querySelectorAll("h3")].find((e) => e.textContent === "Destination");
          const g = h?.parentElement?.parentElement;
          o.rows = g ? [...g.querySelectorAll("dt")].map((dt) => `${dt.textContent}: ${dt.nextElementSibling?.textContent}`) : null;
          o.text = g?.textContent ?? null;
          o.imgs = g ? [...g.querySelectorAll("img")].map((i) => i.getAttribute("src")) : null;
          o.btn = g?.querySelector("button")?.textContent ?? null;
        }
        return o;
      }, key === "list");
      facts[`${key}@${w}-${build}`] = f;
      await page.close();
    }
    await ctx.close();
  }
}
await browser.close();
fs.writeFileSync(`${OUT}/facts.json`, JSON.stringify({ facts, problems }, null, 1));
console.log("done; console/page problems:", problems.length);
for (const p of problems.slice(0, 10)) console.log(p);
