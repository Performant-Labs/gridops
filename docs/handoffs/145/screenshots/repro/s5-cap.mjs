// S round 5 capture: usage node cap.mjs <label> <emptyPort> <popPort>
import { chromium } from "/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-ui-145-platform-neutral-cf5832/fff39e50-a1ac-48a4-a4c2-15939665982d/scratchpad/pw/node_modules/playwright/index.mjs";
import { createHmac } from "node:crypto";
import fs from "node:fs";
const [LABEL, EMPTY, POP] = process.argv.slice(2);
const OUT = "/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-ui-145-platform-neutral-cf5832/fff39e50-a1ac-48a4-a4c2-15939665982d/scratchpad/s5/shots";
const seed = fs.readFileSync("/Users/andreangelantoni/Projects/gridops/.claude/worktrees/ui-145-platform-neutral-cf5832/docs/handoffs/145/screenshots/repro/seed.mjs", "utf8");
const SECRET = seed.match(/const SECRET = "([^"]+)"/)[1];
const b64u = (b) => Buffer.from(b).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const cookieFor = (t) => t + "." + b64u(createHmac("sha256", SECRET).update(t).digest());
const tokens = { admin: "audit-admin-token-0001", member: "audit-member-token-0001" };
const browser = await chromium.launch();
const surfaces = [
  ["connections-pop-admin", POP, "admin", "/settings/connections"],
  ["connections-pop-member", POP, "member", "/settings/connections"],
  ["connections-empty-admin", EMPTY, "admin", "/settings/connections"],
  ["connections-empty-member", EMPTY, "member", "/settings/connections"],
  ["settings-general", POP, "admin", "/settings/general"],
  ["settings-ai", POP, "admin", "/settings/ai"],
  ["settings-members", POP, "admin", "/settings/members"],
  ["settings-preferences", POP, "admin", "/settings/preferences"],
  ["settings-runner-host", POP, "admin", "/settings/runner-host"],
  ["settings-api-tokens", POP, "admin", "/settings/api-tokens"],
  ["pool-settings-repo", POP, "admin", "/runner-pools/pool-1/settings"],
  ["pool-settings-org", POP, "admin", "/runner-pools/pool-2/settings"],
  ["new-pool", POP, "admin", "/runner-pools/new"],
];
const result = {};
for (const [w, h] of [[360, 4200], [768, 3400], [1280, 3000]]) {
  for (const [name, port, who, path] of surfaces) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h } });
    await ctx.addCookies([{ name: "gridops_session", value: cookieFor(tokens[who]), url: "http://127.0.0.1:" + port }]);
    const page = await ctx.newPage();
    const errors = [];
    page.on("console", (m) => { if (m.type() === "error") errors.push(m.text().slice(0, 160)); });
    await page.goto("http://127.0.0.1:" + port + path, { waitUntil: "networkidle" });
    await page.waitForTimeout(600);
    await page.screenshot({ path: OUT + "/" + LABEL + "-" + name + "-" + w + ".png", fullPage: true });
    result[name + "@" + w] = { errors };
    await ctx.close();
  }
}
fs.writeFileSync(OUT + "/" + LABEL + "-console.json", JSON.stringify(result, null, 1));
await browser.close();
console.log("done", LABEL, Object.keys(result).length);
