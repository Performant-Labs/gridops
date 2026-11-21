// F rework (#145, D1/D2): geometry + screenshots for the fixed build vs the pre-fix HEAD build.
// usage: node f-measure.mjs <outDir> <label> <emptyPort> <popPort>
import { chromium } from "playwright";
import { createHmac } from "node:crypto";
import fs from "node:fs";
const [OUT, LABEL, EMPTY, POP] = process.argv.slice(2);
const SECRET = fs.readFileSync("/Users/andreangelantoni/Projects/gridops/.claude/worktrees/ui-145-platform-neutral-cf5832/docs/handoffs/145/screenshots/repro/measure360.mjs", "utf8").match(/const SECRET = "([^"]+)"/)[1];
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const cookieFor = (t) => t + "." + b64u(createHmac("sha256", SECRET).update(t).digest());
const tokens = { admin: "audit-admin-token-0001", member: "audit-member-token-0001" };
const browser = await chromium.launch();
const result = {};
const surfaces = [
  ["connections-populated-admin", POP, "admin", "/settings/connections"],
  ["connections-populated-member", POP, "member", "/settings/connections"],
  ["connections-empty-admin", EMPTY, "admin", "/settings/connections"],
  ["connections-empty-member", EMPTY, "member", "/settings/connections"],
  ["pool-settings-repository", POP, "admin", "/runner-pools/pool-1/settings"],
  ["pool-settings-organization", POP, "admin", "/runner-pools/pool-2/settings"],
  ["new-pool", POP, "admin", "/runner-pools/new"],
  ["settings-general", POP, "admin", "/settings/general"],
];
for (const [w, h] of [[360, 3400], [768, 3400], [1280, 3400]]) {
  for (const [name, port, who, path] of surfaces) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h } });
    await ctx.addCookies([{ name: "gridops_session", value: cookieFor(tokens[who]), url: `http://127.0.0.1:${port}` }]);
    const page = await ctx.newPage();
    const errors = [];
    page.on("console", (m) => { if (m.type() === "error") errors.push(m.text().slice(0, 200)); });
    await page.goto(`http://127.0.0.1:${port}${path}`, { waitUntil: "networkidle" });
    await page.waitForTimeout(700);
    const geo = await page.evaluate(() => {
      const rect = (el) => { const b = el.getBoundingClientRect(); return [Math.round(b.left), Math.round(b.top), Math.round(b.width), Math.round(b.height)]; };
      const leaf = (t) => [...document.querySelectorAll("span,p,div,a,h2,button")].find((e) => e.children.length === 0 && e.textContent.trim() === t);
      const names = {};
      for (const n of ["acme-labs", "ada-admin", "Acme Bitbucket", "Platform Team"]) {
        const el = leaf(n);
        if (el) names[n] = { rect: rect(el), clipped: el.scrollWidth > el.clientWidth };
      }
      const headers = [...document.querySelectorAll("section > div.mb-3")].map((hd) => ({ text: hd.textContent.slice(0, 40), rect: rect(hd), children: [...hd.children].map(rect) }));
      const buttons = [...document.querySelectorAll("a,button")].filter((e) => e.offsetParent).map((e) => ({ t: e.textContent.trim().slice(0, 30), r: rect(e) }));
      const offscreen = buttons.filter((b) => b.r[0] < 0 || b.r[0] + b.r[2] > window.innerWidth).map((b) => b.t);
      const overlap = (a, b) => { const x = a.getBoundingClientRect(), y = b.getBoundingClientRect(); return x.left < y.right && y.left < x.right && x.top < y.bottom && y.top < x.bottom; };
      const owner = leaf("Owner approval required"), org = leaf("Organization");
      const rows = [...document.querySelectorAll("div.px-4.py-3")].map(rect);
      return {
        names, headers, offscreen, rows,
        ownerOverlapsOrg: owner && org ? overlap(owner, org) : null,
        overflow: document.documentElement.scrollWidth > document.documentElement.clientWidth,
      };
    });
    geo.consoleErrors = errors;
    result[`${name}@${w}`] = geo;
    await page.screenshot({ path: `${OUT}/${LABEL}-${name}-${w}.png`, fullPage: true });
    await ctx.close();
  }
}
fs.writeFileSync(`${OUT}/${LABEL}-geometry.json`, JSON.stringify(result, null, 1));
await browser.close();
console.log("done", Object.keys(result).length);
