import { chromium } from "playwright";
import { createHmac } from "node:crypto";
import fs from "node:fs";
const OUT = process.argv[2];
const SECRET = "audit-session-secret-for-s-walk";
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const cookieFor = (t) => t + "." + b64u(createHmac("sha256", SECRET).update(t).digest());
const tokens = { admin: "audit-admin-token-0001", member: "audit-member-token-0001" };
const EMPTY = 18097, POP = 18098;
const VIEWPORTS = [[360, 800], [768, 1024], [1280, 800]];
const log = [];
const browser = await chromium.launch();

async function session(port, who, [w, h]) {
  const ctx = await browser.newContext({ viewport: { width: w, height: h } });
  await ctx.addCookies([{ name: "gridops_session", value: cookieFor(tokens[who]), url: `http://127.0.0.1:${port}` }]);
  const page = await ctx.newPage();
  page.on("console", (m) => { if (["error", "warning"].includes(m.type())) log.push(`[${port} ${who} ${w}] ${m.type()}: ${m.text().slice(0, 220)}`); });
  return { ctx, page, w, port };
}
async function open(s, path) {
  await s.page.goto(`http://127.0.0.1:${s.port}${path}`, { waitUntil: "networkidle" });
  await s.page.waitForTimeout(400);
}
async function shot(s, name, full = true) {
  const file = `${OUT}/s-${name}-${s.w}.png`;
  await s.page.screenshot({ path: file, fullPage: full });
  const o = await s.page.evaluate(() => ({ sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth }));
  log.push(`shot ${name} @${s.w}: scrollWidth=${o.sw} clientWidth=${o.cw}${o.sw > o.cw ? "  HORIZONTAL OVERFLOW" : ""}`);
}
const attempt = async (label, fn) => { try { await fn(); } catch (e) { log.push(`FAILED ${label}: ${String(e).split("\n")[0]}`); } };

for (const vp of VIEWPORTS) {
  // Connections page, nothing connected, App not configured (admin) + chooser + steps
  let s = await session(EMPTY, "admin", vp);
  await open(s, "/settings/connections");
  await shot(s, "connections-empty-admin");
  await attempt("empty chooser", async () => {
    await s.page.getByRole("button", { name: "Add connection" }).first().click();
    await s.page.waitForSelector("[role=dialog]");
    await shot(s, "add-chooser-empty", false);
    await s.page.locator("[role=dialog]").getByText("GitHub", { exact: true }).first().click();
    await s.page.waitForTimeout(300);
    await shot(s, "add-github-linkout-unconfigured", false);
  });
  await s.ctx.close();

  s = await session(EMPTY, "member", vp);
  await open(s, "/settings/connections");
  await shot(s, "connections-empty-member");
  await s.ctx.close();

  // Populated, App configured
  s = await session(POP, "admin", vp);
  await open(s, "/settings/connections");
  await shot(s, "connections-populated-admin");
  await attempt("populated chooser", async () => {
    await s.page.getByRole("button", { name: "Add connection" }).first().click();
    await s.page.waitForSelector("[role=dialog]");
    await shot(s, "add-chooser-populated", false);
    await s.page.locator("[role=dialog]").getByText("GitHub", { exact: true }).first().click();
    await s.page.waitForTimeout(300);
    await shot(s, "add-github-linkout-configured", false);
    await s.page.getByRole("button", { name: "Back" }).click();
    await s.page.waitForTimeout(200);
    await s.page.locator("[role=dialog]").getByText("Bitbucket", { exact: true }).first().click();
    await s.page.waitForTimeout(300);
    await shot(s, "add-bitbucket-form", false);
  });
  await s.ctx.close();

  s = await session(POP, "member", vp);
  await open(s, "/settings/connections");
  await shot(s, "connections-populated-member");
  await s.ctx.close();

  // New pool form + open Targets picker
  s = await session(POP, "admin", vp);
  await open(s, "/runner-pools/new");
  await shot(s, "new-pool-form");
  await attempt("new-pool picker", async () => {
    await s.page.getByRole("combobox", { name: /targets/i }).first().click().catch(async () => {
      await s.page.getByText("Targets", { exact: true }).first().click();
    });
    await s.page.waitForTimeout(800);
    await shot(s, "new-pool-targets-open", false);
  });
  await s.ctx.close();

  // Pool settings (repository pool with Bitbucket target) + picker
  s = await session(POP, "admin", vp);
  await open(s, "/runner-pools/pool-1/settings");
  await shot(s, "pool-settings-mixed");
  await attempt("settings picker", async () => {
    await s.page.getByRole("combobox", { name: /targets/i }).first().click().catch(async () => {
      await s.page.getByText("Targets", { exact: true }).first().click();
    });
    await s.page.waitForTimeout(800);
    await shot(s, "pool-settings-targets-open", false);
  });
  await s.ctx.close();

  s = await session(POP, "admin", vp);
  await open(s, "/runner-pools/pool-2/settings");
  await shot(s, "pool-settings-org");
  await s.ctx.close();
}
fs.writeFileSync(`${OUT}/../s-capture-log.txt`, log.join("\n") + "\n");
console.log(log.join("\n"));
await browser.close();
