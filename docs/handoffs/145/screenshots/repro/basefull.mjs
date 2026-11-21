import { chromium } from "playwright";
import { createHmac } from "node:crypto";
const OUT = process.argv[2];
const SECRET = "audit-session-secret-for-s-walk";
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const cookieFor = (t) => t + "." + b64u(createHmac("sha256", SECRET).update(t).digest());
const browser = await chromium.launch();
for (const [w, h] of [[360, 2600], [768, 2000], [1280, 2000]]) {
  const ctx = await browser.newContext({ viewport: { width: w, height: h } });
  await ctx.addCookies([{ name: "gridops_session", value: cookieFor("audit-admin-token-0001"), url: "http://127.0.0.1:18100" }]);
  const page = await ctx.newPage();
  await page.goto("http://127.0.0.1:18100/settings/github", { waitUntil: "networkidle" });
  await page.waitForTimeout(500);
  await page.screenshot({ path: `${OUT}/base-github-page-full-${w}.png` });
  await ctx.close();
}
await browser.close();
