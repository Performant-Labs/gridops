import fs from "node:fs";
const ROOT = "/Users/andreangelantoni/Projects/gridops/.claude/worktrees/gitlab-159-pool-pages-fe8fd1/docs/handoffs/159";
const D = "20261121";
const tsv = fs.readFileSync("/private/tmp/s159-diff-table2.tsv", "utf8").trim().split("\n").slice(1).map((l) => l.split("\t"));
const metrics = {};
for (const [name, vp, ref, live, ae, pct] of tsv) {
  const stem = `t3-${name}-${vp}`;
  if (fs.existsSync(`${ROOT}/screenshots/cycle-1/${stem}-diff-${D}.png`)) metrics[stem] = { vp: Number(vp), ref, ae: Number(ae), pct: Number(pct) };
}
const STATES = [
  ["list", "Pool list, all nine pools", "mixed"],
  ["gh-org", "Pool detail (top of page): GitHub organization pool", "match"],
  ["gl-two", "Pool detail (top of page): GitLab pool, 2 connections", "delta"],
  ["gh-many-dest", "Destination group: GitHub repository pool, 3 repositories", "match"],
  ["gh-nine-dest", "Destination group: GitHub repository pool, 9 repositories (collapsed)", "match"],
  ["gh-nine-expanded-dest", "Destination group: GitHub repository pool, 9 repositories (expanded)", "match"],
  ["gh-org-dest", "Destination group: GitHub organization pool", "match"],
  ["bb-dest", "Destination group: Bitbucket (GitHub-scoped) pool", "match"],
  ["gl-one-dest", "Destination group: GitLab pool, 1 connection", "delta"],
  ["gl-two-dest", "Destination group: GitLab pool, 2 connections", "delta"],
  ["gl-zero-dest", "Destination group: GitLab pool, no connections", "delta"],
  ["unknown-dest", "Destination group: unknown scope", "delta"],
];
const VPS = [[360, 800], [768, 1024], [1280, 800]];
const img = (stem, kind) => `screenshots/cycle-1/${stem}-${kind}-${D}.png`;
const fmt = (m) => `${m.ae < 1 ? (m.ae > 0 ? "<1" : "0") : Math.round(m.ae).toLocaleString("en-US")} px (${m.pct.toFixed(3)}%)`;
const worst = (vp, kind) => Math.max(0, ...STATES.filter((s) => s[2] === kind).map((s) => metrics[`t3-${s[0]}-${vp}`]?.pct ?? 0));
const summaryRow = (vp) => {
  const gh = STATES.filter((s) => s[2] === "match").map((s) => metrics[`t3-${s[0]}-${vp}`]).filter(Boolean);
  const ghMax = Math.max(...gh.map((m) => m.ae));
  const dl = Math.max(...STATES.filter((s) => s[2] !== "match").map((s) => metrics[`t3-${s[0]}-${vp}`]?.pct ?? 0));
  return { vp, ghMax, dl };
};
const sums = VPS.map(([w]) => summaryRow(w));
const crops = [
  ["Pool list: destination cells of the five GitHub and Bitbucket pools", "1280", "MATCH", "No pixel differs from the build before the change: same avatar, same text, same spacing.", "t3-list-github-rows-1280-crop"],
  ["Pool list: destination cells of the GitLab pools and the unknown-scope pool", "1280", "DELTA (intended)", "Before: each showed the installation's GitHub avatar and \"acme-labs\". Now: the GitLab icon with \"Acme / web\", \"2 connections\" or \"GitLab\"; a dash with an empty square for the unknown scope.", "t3-list-gitlab-and-unknown-rows-1280-crop"],
  ["Pool list at 360 and 768", "360, 768", "MATCH", "The destination column is hidden below 1280 by an existing rule that this change does not touch, so the page is identical.", null],
  ["Pool detail, Destination group: GitHub organization pool", "1280", "MATCH", "Scope, Account with avatar and Runner group #id are unchanged.", "t3-gh-org-destination-1280-crop"],
  ["Pool detail, Destination group: GitHub organization pool", "360", "MATCH", "Same at mobile width.", "t3-gh-org-destination-360-crop"],
  ["Pool detail, Destination group: Bitbucket pool (GitHub-scoped)", "1280", "MATCH", "Looks exactly like a GitHub organization pool, as before.", "t3-bb-destination-1280-crop"],
  ["Pool detail, Destination group: GitHub repository pool, 9 repositories, expanded", "1280", "MATCH", "The list of nine, the \"Show fewer\" toggle and the spacing are unchanged. The collapsed state (\"Show 3 more\") also matches at all three widths.", "t3-gh-nine-expanded-destination-1280-crop"],
  ["Pool detail, Destination group: GitLab pool, 1 connection", "1280", "DELTA (intended)", "Scope reads \"GitLab\" (was \"Gitlab\"); one two-line connection item; the Account row and Runner group row are gone.", "t3-gl-one-destination-1280-crop"],
  ["Pool detail, Destination group: GitLab pool, 2 connections", "1280", "DELTA (intended)", "Scope \"GitLab\" and two connection items in the server's order. The group is a little taller, so the Configuration group below it moves down.", "t3-gl-two-destination-1280-crop"],
  ["Pool detail, Destination group: GitLab pool, 2 connections", "360", "DELTA (intended)", "Same change on a phone; both items fit without clipping or sideways scrolling.", "t3-gl-two-destination-360-crop"],
  ["Pool detail, Destination group: GitLab pool, no connections", "1280", "DELTA (intended)", "Scope \"GitLab\" then a muted \"No connections listed\"; the installation's GitHub account no longer shows.", "t3-gl-zero-destination-1280-crop"],
  ["Pool detail, Destination group: unknown scope", "1280", "DELTA (intended)", "Only \"Scope: Unknown\"; the raw scope text, the GitHub account and the Runner group row are not shown.", "t3-unknown-destination-1280-crop"],
  ["Everything else on every page (sidebar, header, status, runtime, configuration)", "360, 768, 1280", "MATCH", "Every element above the Destination group has the same position and size in both builds (measured). Only the content below the group moves with its height, and some text edges look slightly different at 1280 for the shorter groups (GitLab one or no connection, unknown scope) because the page no longer scrolls.", null],
];
const rows = crops.map(([s, v, st, d, c]) => `<tr><td>${s}</td><td>${v}</td><td class="st ${st.startsWith("MATCH") ? "m" : "d"}">${st}</td><td>${d}</td><td>${c ? `<a href="screenshots/cycle-1/${c}-${D}.png"><img class="crop" loading="lazy" src="screenshots/cycle-1/${c}-${D}.png" alt=""></a>` : "-"}</td></tr>`).join("\n");
const blocks = VPS.map(([w, h]) => {
  const first = STATES[0][0];
  const tbl = STATES.map(([k, label, kind]) => { const m = metrics[`t3-${k}-${w}`]; return `<tr><td>${label}</td><td>${m ? fmt(m) : "-"}</td><td class="st ${kind === "match" ? "m" : "d"}">${kind === "match" ? "MATCH" : kind === "delta" ? "DELTA (intended)" : "MATCH + DELTA (intended)"}</td></tr>`; }).join("");
  return `
<section class="vp" id="vp-${w}" data-vp="${w}">
  <h2>${w} x ${h}</h2>
  <label>Show: <select class="pick">${STATES.map(([k, label]) => `<option value="t3-${k}-${w}">${label}</option>`).join("")}</select></label>
  <p class="hint">Drag the slider. Left of the line is the build after this change; right of the line is the build before it.</p>
  <div class="wipe" style="--aspect:${w}/${h}">
    <img class="size" alt="" src="${img(`t3-${first}-${w}`, "reference")}">
    <img class="bottom" alt="Before this change" src="${img(`t3-${first}-${w}`, "reference")}">
    <img class="top" alt="After this change" src="${img(`t3-${first}-${w}`, "live")}">
    <div class="line"></div>
  </div>
  <input class="range" type="range" min="0" max="100" value="50" aria-label="Wipe between before and after at ${w} pixels wide">
  <h3>Pixel difference (red = changed)</h3>
  <img class="diff full" alt="Difference image" src="${img(`t3-${first}-${w}`, "diff")}">
  <h3>Side by side (before, then after)</h3>
  <img class="comp full" alt="Side-by-side" src="${img(`t3-${first}-${w}`, "composite")}">
  <h3>Numbers at ${w} px</h3>
  <table><thead><tr><th>Page state</th><th>Pixels different</th><th>Status</th></tr></thead><tbody>${tbl}</tbody></table>
</section>`;
}).join("\n");
const pctOf = (x) => x.toFixed(3) + "%";
const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>#159 spec audit: pool destination from the registry</title>
<style>
:root{color-scheme:light dark;--bg:#fafafa;--fg:#161616;--mut:#5b5b5b;--card:#fff;--bd:#d9d9d9;--ok:#14803c;--warn:#b45309}
@media(prefers-color-scheme:dark){:root{--bg:#121214;--fg:#ececec;--mut:#a0a0a0;--card:#1a1a1d;--bd:#33333a;--ok:#4ade80;--warn:#fbbf24}}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,-apple-system,Segoe UI,sans-serif}
header.top{position:sticky;top:0;background:var(--card);border-bottom:1px solid var(--bd);padding:8px 24px;z-index:5;display:flex;gap:16px;flex-wrap:wrap;align-items:center}
header.top a{color:var(--fg);text-decoration:none;font-size:13px;padding:2px 8px;border:1px solid var(--bd);border-radius:6px}
main{max-width:1180px;margin:0 auto;padding:24px}
.banner{border:2px solid var(--ok);border-radius:12px;padding:20px 24px;background:var(--card)}
.banner .v{font-size:44px;font-weight:800;color:var(--ok);letter-spacing:.02em}
.banner .sub{color:var(--mut)}
.nums{display:grid;grid-template-columns:repeat(3,1fr);gap:12px;margin-top:14px}
.nums div{border:1px solid var(--bd);border-radius:8px;padding:10px 12px;font-size:13px}
.nums b{display:block;font-size:15px}
h1{margin:0 0 6px;font-size:22px}h2{margin-top:36px;border-bottom:1px solid var(--bd);padding-bottom:6px}h3{margin:18px 0 6px;font-size:14px;color:var(--mut)}
table{border-collapse:collapse;width:100%;background:var(--card);font-size:13px}th,td{border:1px solid var(--bd);padding:6px 8px;text-align:left;vertical-align:top}
td.st{font-weight:700;white-space:nowrap}td.m{color:var(--ok)}td.d{color:var(--warn)}
.crop{width:300px;max-width:100%;border:1px solid var(--bd);display:block}
.full{width:100%;border:1px solid var(--bd);display:block;background:#888}
.wipe{position:relative;width:100%;max-width:1180px;border:1px solid var(--bd);overflow:hidden;background:#000}
.wipe img{display:block;width:100%}
.wipe .size{visibility:hidden}
.wipe .bottom,.wipe .top{position:absolute;inset:0 auto auto 0;height:100%;object-fit:fill}
.wipe .top{clip-path:inset(0 50% 0 0)}
.wipe .line{position:absolute;top:0;bottom:0;left:50%;width:2px;background:#f0f;pointer-events:none}
.range{width:100%;max-width:1180px;margin:8px 0}
.hint{color:var(--mut);font-size:13px;margin:4px 0 8px}
select{font:inherit;padding:4px 6px;max-width:100%}
ul li{margin:4px 0}code{background:var(--card);border:1px solid var(--bd);border-radius:4px;padding:0 4px}
@media(max-width:800px){.nums{grid-template-columns:1fr}}
</style></head><body>
<header class="top"><strong>#159 audit</strong><a href="#verdict">Verdict</a><a href="#different">What I see different</a><a href="#vp-360">360</a><a href="#vp-768">768</a><a href="#vp-1280">1280</a><a href="#sections">Per-section table</a><a href="#follow">Follow-ups</a><a href="#method">Method</a></header>
<main>
<section id="verdict" class="banner">
  <h1>Pool list and pool detail: destination from the platform registry (#159)</h1>
  <div class="v">PASS</div>
  <div class="sub">Compared the build after this change (branch head) with the build before it (master tip), same data, same fixed clock, headless Chromium, 2026-11-21.</div>
  <div class="nums">
${sums.map((s) => `    <div><b>${s.vp} px</b>GitHub and Bitbucket pools: ${s.ghMax < 1 ? (s.ghMax > 0 ? "no whole pixel differs" : "0 px different") : s.ghMax + " px different"}<br>GitLab and unknown-scope pools: up to ${pctOf(s.dl)} of the screen differs (the destination area, plus the side effects listed below)</div>`).join("\n")}
  </div>
</section>

<h2 id="different">What I see different</h2>
<ul>
<li><b>GitHub and Bitbucket pools: nothing.</b> At 360, 768 and 1280 wide, the pool list rows, the detail page's Destination group, the nine-repository list and its "Show 3 more" / "Show fewer" toggle are identical to the build before the change. Two of the 1280 captures differ by one shade on a 2 by 2 pixel rounded corner, which is rendering noise.</li>
<li><b>Pool list, GitLab pools (1280):</b> the destination column no longer shows the installation's GitHub avatar and "acme-labs". It shows the GitLab icon with "Acme / web" (one connection), "2 connections" (hover shows both names) or "GitLab" (no connections).</li>
<li><b>Pool list, unknown scope (1280):</b> an empty dashed square and a dash instead of the GitHub avatar and account name; the hover text reads "Unknown destination".</li>
<li><b>Pool detail, GitLab pools (360, 768, 1280):</b> Scope now reads "GitLab" (it read "Gitlab"). The Account row with the GitHub avatar and the "Runner group #1" row are gone. Each connection appears as a small two-line item: its name, then its address. With no connections the group says "No connections listed".</li>
<li><b>Pool detail, unknown scope:</b> a single row, "Scope: Unknown". The raw scope text is no longer echoed.</li>
<li><b>Side effects of the above:</b> where the Destination group gets taller or shorter, the Configuration group below it moves with it. Where it gets shorter (GitLab with one or no connection, unknown scope) the page no longer needs to scroll by 6 px, so the browser draws the text on the rest of the page with slightly different edge smoothing at 1280 wide; no element outside the Destination group moves or changes size (measured, element by element). In the detail captures taken with the group scrolled into view, the page also sits at a slightly different scroll position for the same reason.</li>
<li>The pool list's destination column is still hidden below 1280 wide. That is an existing rule and is not changed here.</li>
</ul>

${blocks}

<h2 id="sections">Per-section delta table</h2>
<table><thead><tr><th>Section</th><th>Viewport</th><th>Status</th><th>Description</th><th>Crop of the difference image</th></tr></thead><tbody>
${rows}
</tbody></table>

<h2 id="follow">Sub-issue recommendations</h2>
<p>None required: the verdict is PASS. One carried item for the operator (not part of this audit's verdict): the runner rows and the dashboard pool row still show a GitLab pool under the installation's GitHub login or as "Gitlab". They are outside #159's scope and this change leaves them as they were; a follow-up in epic #136 should take them through the same registry (<code>runner-row.tsx</code> lines 46, 109, 127 and <code>routes/_app/index.tsx</code> line 279).</p>

<h2 id="method">Method</h2>
<ul>
<li>Both builds are <code>vite build</code> outputs of committed trees (<code>git archive</code>): before = <code>e022a5d</code> (identical source to master tip), after = the branch head. Both served by the same API binary against identically seeded throwaway databases, with the build number and commit pinned so the version bar matches.</li>
<li>Captured with headless Playwright Chromium at 360x800, 768x1024 and 1280x800; clock fixed; GitHub avatar requests answered with one fixed image so avatars are comparable. The app scrolls inside its own container, so detail pages were captured with the Destination group scrolled into view.</li>
<li>Differences counted with ImageMagick <code>compare -metric AE</code>, diff images from the same command, composites with <code>+append</code>. The reference for GitHub and Bitbucket is the build before the change; for the new GitLab and unknown-scope states the wireframe (<code>wireframe.html</code>, low fidelity) cannot be a pixel reference, so copy and layout were checked against it by eye. The pixel diff against the before build shows the destination area changing, and an element-by-element geometry comparison shows nothing above the Destination group changing position or size.</li>
</ul>
</main>
<script>
document.querySelectorAll("section.vp").forEach(function(sec){
  var pick=sec.querySelector(".pick"),range=sec.querySelector(".range"),top=sec.querySelector(".top");
  function wipe(){top.style.clipPath="inset(0 "+(100-range.value)+"% 0 0)";sec.querySelector(".line").style.left=range.value+"%";}
  range.addEventListener("input",wipe);wipe();
  pick.addEventListener("change",function(){
    var s=pick.value,base="screenshots/cycle-1/"+s+"-";
    sec.querySelector(".size").src=base+"reference-${D}.png";
    sec.querySelector(".bottom").src=base+"reference-${D}.png";
    top.src=base+"live-${D}.png";
    sec.querySelector(".diff").src=base+"diff-${D}.png";
    sec.querySelector(".comp").src=base+"composite-${D}.png";
  });
});
</script>
</body></html>`;
fs.writeFileSync(`${ROOT}/phase-9-pool-destination-report.html`, html);
console.log("report written", html.length, JSON.stringify(sums));
