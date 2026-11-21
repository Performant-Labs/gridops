import { chromium } from "playwright";
import { createHmac } from "node:crypto";
const OUT = process.argv[2];
const SECRET = "audit-session-secret-for-s-walk";
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const cookieFor = (t) => t + "." + b64u(createHmac("sha256", SECRET).update(t).digest());
const browser = await chromium.launch();
for (const [w, h] of [[360, 800], [768, 1024], [1280, 800]]) {
  for (const [port, name] of [[18097, "add-chooser-empty"], [18098, "add-chooser-populated"]]) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h } });
    await ctx.addCookies([{ name: "gridops_session", value: cookieFor("audit-admin-token-0001"), url: `http://127.0.0.1:${port}` }]);
    const page = await ctx.newPage();
    await page.goto(`http://127.0.0.1:${port}/settings/connections`, { waitUntil: "networkidle" });
    await page.getByRole("button", { name: "Add connection" }).first().click();
    await page.locator("[role=dialog]").waitFor();
    await page.waitForTimeout(900);
    await page.screenshot({ path: `${OUT}/s-${name}-${w}.png` });
    await ctx.close();
  }
}
await browser.close();
