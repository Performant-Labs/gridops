// usage node measure.mjs <label> <emptyPort> <popPort>
import { chromium } from "/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-ui-145-platform-neutral-cf5832/fff39e50-a1ac-48a4-a4c2-15939665982d/scratchpad/pw/node_modules/playwright/index.mjs";
import { createHmac } from "node:crypto";
import fs from "node:fs";
const [LABEL, EMPTY, POP] = process.argv.slice(2);
const seed = fs.readFileSync("/Users/andreangelantoni/Projects/gridops/.claude/worktrees/ui-145-platform-neutral-cf5832/docs/handoffs/145/screenshots/repro/seed.mjs", "utf8");
const SECRET = seed.match(/const SECRET = "([^"]+)"/)[1];
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const cookieFor = (t) => t + "." + b64u(createHmac("sha256", SECRET).update(t).digest());
const tokens = { admin: "audit-admin-token-0001", member: "audit-member-token-0001" };
const browser = await chromium.launch();
const out = {};
for (const who of ["admin", "member"]) {
  for (const w of [320, 360, 480, 639, 640, 768, 1024, 1280]) {
    const ctx = await browser.newContext({ viewport: { width: w, height: 4200 } });
    await ctx.addCookies([{ name: "gridops_session", value: cookieFor(tokens[who]), url: "http://127.0.0.1:" + POP }]);
    const page = await ctx.newPage();
    await page.goto("http://127.0.0.1:" + POP + "/settings/connections", { waitUntil: "networkidle" });
    await page.waitForTimeout(600);
    const geo = await page.evaluate(() => {
      const rect = (el) => { const b = el.getBoundingClientRect(); return [Math.round(b.left), Math.round(b.top), Math.round(b.width), Math.round(b.height)]; };
      const leaf = (t) => [...document.querySelectorAll("span,p,div,a,h2,button")].find((e) => e.children.length === 0 && e.textContent.trim() === t);
      const names = {};
      for (const n of ["acme-labs", "ada-admin", "Acme Bitbucket", "Platform Team", "Extremely-long-bitbucket-connection-name-for-the-platform-engineering-team-workspace"]) {
        const el = leaf(n);
        if (el) names[n.slice(0, 14)] = { w: rect(el)[2], full: Math.round(el.scrollWidth), clipped: el.scrollWidth > el.clientWidth };
      }
      const headers = [...document.querySelectorAll("section > div.mb-3")].map((hd) => ({ t: hd.querySelector("h2")?.textContent, h2w: rect(hd.querySelector("h2"))[2], actions: hd.children[1] ? [...hd.children[1].children].map((c) => ({ t: c.textContent.trim().slice(0, 18), r: rect(c) })) : [] }));
      const buttons = [...document.querySelectorAll("a,button")].filter((e) => e.offsetParent).map((e) => ({ t: e.textContent.trim().slice(0, 30), r: rect(e) }));
      const offscreen = buttons.filter((b) => b.r[0] < 0 || b.r[0] + b.r[2] > window.innerWidth).map((b) => b.t);
      const overlap = (a, b) => { const x = a.getBoundingClientRect(), y = b.getBoundingClientRect(); return x.left < y.right && y.left < x.right && x.top < y.bottom && y.top < x.bottom; };
      const owner = leaf("Owner approval required"), org = leaf("Organization");
      // any two sibling leaf text nodes inside a row overlapping
      const rows = [...document.querySelectorAll("div.px-4.py-3")].map((r) => ({ h: rect(r)[3], w: rect(r)[2] }));
      return { names, headers, offscreen, rows, ownerOverlapsOrg: owner && org ? overlap(owner, org) : null, overflow: document.documentElement.scrollWidth > document.documentElement.clientWidth || document.body.scrollWidth > document.documentElement.clientWidth };
    });
    out[who + "@" + w] = geo;
    await ctx.close();
  }
}
fs.writeFileSync("/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-ui-145-platform-neutral-cf5832/fff39e50-a1ac-48a4-a4c2-15939665982d/scratchpad/s5/" + LABEL + "-measure.json", JSON.stringify(out, null, 1));
for (const [k, v] of Object.entries(out)) {
  console.log(k, "overflow=" + v.overflow, "offscreen=" + JSON.stringify(v.offscreen), "ownerOverlap=" + v.ownerOverlapsOrg);
  console.log("   names:", JSON.stringify(Object.fromEntries(Object.entries(v.names).map(([n, x]) => [n, x.clipped ? x.w + "/" + x.full + " CLIPPED" : x.w]))));
  console.log("   headers:", JSON.stringify(v.headers.map((h) => h.t + " h2w=" + h.h2w + " actions@" + h.actions.map((a) => a.r[0] + "," + a.r[1]).join(";"))));
}
await browser.close();
