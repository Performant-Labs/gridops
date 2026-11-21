import { chromium } from "playwright";
import { createHmac } from "node:crypto";
const SECRET = "audit-session-secret-for-s-walk";
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const cookieFor = (t) => t + "." + b64u(createHmac("sha256", SECRET).update(t).digest());
const browser = await chromium.launch();
async function measure(port, path, openText, w, h, label) {
  const ctx = await browser.newContext({ viewport: { width: w, height: h } });
  await ctx.addCookies([{ name: "gridops_session", value: cookieFor("audit-admin-token-0001"), url: `http://127.0.0.1:${port}` }]);
  const page = await ctx.newPage();
  await page.goto(`http://127.0.0.1:${port}${path}`, { waitUntil: "networkidle" });
  await page.waitForTimeout(600);
  // real mouse click at the trigger's centre (no auto-scroll side effects beyond a normal click)
  const trig = page.getByText(openText).first();
  const box = await trig.boundingBox();
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await page.waitForTimeout(1200);
  const res = await page.evaluate(() => {
    const opts = [...document.querySelectorAll("[role=option]")];
    const hit = opts.filter((o) => { const r = o.getBoundingClientRect(); if (r.height === 0) return false; const el = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); return el && o.contains(el); });
    const search = document.querySelector("input[placeholder*='Search']");
    let searchHit = false; if (search) { const r = search.getBoundingClientRect(); const el = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); searchHit = !!el && (el === search || search.contains(el)); }
    return { options: opts.length, optionsHitTestable: hit.length, searchVisible: searchHit };
  });
  console.log(label.padEnd(34), w, JSON.stringify(res));
  await ctx.close();
}
for (const w of [360, 768, 1280]) {
  const h = w === 360 ? 800 : w === 768 ? 1024 : 800;
  await measure(18100, "/runner-pools/new", "Choose one or more repositories…", w, h, "BASE new-pool repositories");
  await measure(18098, "/runner-pools/new", "Choose where this pool’s runners register…", w, h, "HEAD new-pool targets");
  await measure(18098, "/runner-pools/pool-1/settings", "3 targets selected", w, h, "HEAD pool-settings targets");
}
await browser.close();
