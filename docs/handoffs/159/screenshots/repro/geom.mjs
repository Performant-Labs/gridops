import { chromium } from "/private/tmp/claude-501/-Users-andreangelantoni-Projects-gridops--claude-worktrees-gitlab-159-pool-pages-fe8fd1/c2780b4c-a68f-4a22-a2f0-a16b556459de/scratchpad/pw/node_modules/playwright/index.mjs";
import fs from "node:fs";
const RUN = process.argv[2]; const cookie = fs.readFileSync(`${RUN}/cookie.txt`, "utf8").trim();
const b = await chromium.launch();
async function geom(port, id, w) {
  const ctx = await b.newContext({ viewport: { width: w, height: 800 } });
  await ctx.addCookies([{ name: "gridops_session", value: cookie, url: `http://127.0.0.1:${port}` }]);
  const p = await ctx.newPage(); await p.goto(`http://127.0.0.1:${port}/runner-pools/${id}`, { waitUntil: "networkidle" }); await p.waitForTimeout(500);
  const r = await p.evaluate(() => {
    const all = [...document.querySelectorAll("*")];
    const h = [...document.querySelectorAll("h3")].find((e) => e.textContent === "Destination"); const g = h.parentElement.parentElement;
    const rect = (e) => { const r = e.getBoundingClientRect(); return [Math.round(r.x * 10) / 10, Math.round(r.y * 10) / 10, Math.round(r.width * 10) / 10, Math.round(r.height * 10) / 10]; };
    const inGroup = (e) => g.contains(e) && e !== g;
    return all.filter((e) => !inGroup(e)).map((e) => ({ k: e.tagName + "." + (e.className + "").slice(0, 30) + "|" + (e.textContent || "").slice(0, 20).replace(/\s+/g, " "), r: rect(e), after: !!(g.compareDocumentPosition(e) & Node.DOCUMENT_POSITION_FOLLOWING) && !g.contains(e) }));
  });
  await ctx.close(); return r;
}
for (const w of [1280, 360]) for (const id of ["pool-gl-one", "pool-gl-zero", "pool-unknown", "pool-gh-org"]) {
  const A = await geom(19411, id, w), B = await geom(19412, id, w);
  const diffs = []; const n = Math.min(A.length, B.length);
  for (let i = 0; i < n; i++) { if (A[i].k !== B[i].k) { diffs.push(`ALIGN-BREAK@${i} ${A[i].k} vs ${B[i].k}`); break; } const same = A[i].r.every((v, j) => Math.abs(v - B[i].r[j]) < 0.6); if (!same) diffs.push(`${A[i].after ? "after-group" : "before/around-group"} ${A[i].k} ${JSON.stringify(A[i].r)} -> ${JSON.stringify(B[i].r)}`); }
  console.log(`== ${id} @${w}: ${A.length} vs ${B.length} elements outside the group; differing: ${diffs.length}`);
  for (const d of diffs.slice(0, 14)) console.log("  ", d);
}
await b.close();
