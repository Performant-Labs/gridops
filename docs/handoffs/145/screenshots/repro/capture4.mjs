import { chromium } from "playwright";
import { createHmac } from "node:crypto";
import fs from "node:fs";
const OUT = process.argv[2];
const SECRET = "audit-session-secret-for-s-walk";
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const cookieFor = (t) => t + "." + b64u(createHmac("sha256", SECRET).update(t).digest());
const POP = 18098;
const log = [];
const browser = await chromium.launch();
const sess = async (w, h) => {
  const ctx = await browser.newContext({ viewport: { width: w, height: h } });
  await ctx.addCookies([{ name: "gridops_session", value: cookieFor("audit-admin-token-0001"), url: `http://127.0.0.1:${POP}` }]);
  const page = await ctx.newPage();
  page.on("console", (m) => { if (["error", "warning"].includes(m.type())) log.push(`[${w}] console ${m.type()}: ${m.text().slice(0, 200)}`); });
  page.on("pageerror", (e) => log.push(`[${w}] pageerror: ${String(e).slice(0, 200)}`));
  return { ctx, page, w };
};
const go = async (s, path) => { await s.page.goto(`http://127.0.0.1:${POP}${path}`, { waitUntil: "networkidle" }); await s.page.waitForTimeout(500); };
const shot = async (s, name) => {
  await s.page.screenshot({ path: `${OUT}/s-${name}-${s.w}.png` });
  const o = await s.page.evaluate(() => ({ sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth, mainSw: (document.querySelector("main")||document.body).scrollWidth, mainCw: (document.querySelector("main")||document.body).clientWidth }));
  log.push(`shot ${name} @${s.w}: doc ${o.sw}/${o.cw} main ${o.mainSw}/${o.mainCw}${o.sw > o.cw || o.mainSw > o.mainCw ? " HORIZONTAL OVERFLOW" : ""}`);
};
const attempt = async (label, fn) => { try { await fn(); } catch (e) { log.push(`FAILED ${label}: ${String(e).split("\n")[0]}`); } };
const TALL = { 360: 3600, 768: 2600, 1280: 2200 };

for (const w of [360, 768, 1280]) {
  // New pool: full-length form
  let s = await sess(w, TALL[w]);
  await go(s, "/runner-pools/new");
  await shot(s, "new-pool-form-full");
  await s.ctx.close();

  // New pool: picker open, then a repository picked
  s = await sess(w, w === 360 ? 900 : 800);
  await go(s, "/runner-pools/new");
  await attempt("new picker", async () => {
    await s.page.getByText("Choose where this pool’s runners register…").click();
    await s.page.waitForTimeout(1200);
    await shot(s, "new-pool-targets-open");
    await s.page.getByText("acme-labs/web-app", { exact: true }).click();
    await s.page.waitForTimeout(500);
    await shot(s, "new-pool-targets-repo-picked-open");
    await s.page.keyboard.press("Escape");
    await s.page.waitForTimeout(500);
    await shot(s, "new-pool-after-repo-picked");
  });
  await s.ctx.close();

  // New pool: repo + bitbucket picked, full form with Platform options expanded
  s = await sess(w, TALL[w]);
  await go(s, "/runner-pools/new");
  await attempt("new mixed", async () => {
    await s.page.getByText("Choose where this pool’s runners register…").click();
    await s.page.waitForTimeout(1000);
    await s.page.getByText("acme-labs/web-app", { exact: true }).click();
    await s.page.getByText("Acme Bitbucket", { exact: true }).click();
    await s.page.waitForTimeout(400);
    await s.page.keyboard.press("Escape");
    await s.page.waitForTimeout(400);
    const po = s.page.getByRole("button", { name: /Platform options/ });
    await po.click();
    await s.page.waitForTimeout(500);
    await shot(s, "new-pool-mixed-platform-options-full");
  });
  await s.ctx.close();

  // Pool settings: full, picker open, platform options expanded
  s = await sess(w, TALL[w]);
  await go(s, "/runner-pools/pool-1/settings");
  await attempt("settings platform options", async () => {
    await s.page.getByRole("button", { name: /Platform options/ }).click();
    await s.page.waitForTimeout(500);
    await shot(s, "pool-settings-mixed-full");
  });
  await s.ctx.close();

  s = await sess(w, w === 360 ? 900 : 800);
  await go(s, "/runner-pools/pool-1/settings");
  await attempt("settings picker", async () => {
    await s.page.getByText("3 targets selected").click();
    await s.page.waitForTimeout(1200);
    await shot(s, "pool-settings-targets-open");
  });
  await s.ctx.close();

  s = await sess(w, TALL[w]);
  await go(s, "/runner-pools/pool-2/settings");
  await shot(s, "pool-settings-org-full");
  await s.ctx.close();
}
fs.writeFileSync(`${OUT}/s-capture-log-forms.txt`, log.join("\n") + "\n");
console.log(log.join("\n"));
await browser.close();
