import fs from "node:fs";
const D = process.argv[2];
const slider = (id, before, after, w) => `
<div class="cmp" style="max-width:${Math.min(w, 900)}px"><div class="lbl"><span>Before the fix (left of the handle)</span><span>After the fix (right)</span></div>
<div class="stack" id="${id}"><img src="screenshots/${after}" alt="After the fix"><img class="top" src="screenshots/${before}" alt="Before the fix"></div>
<input type="range" min="0" max="100" value="50" aria-label="Wipe between before and after" oninput="document.getElementById('${id}').querySelector('.top').style.clipPath='inset(0 '+(100-this.value)+'% 0 0)'">
</div>`;
const block = (w, who, ae, pct, note) => `
<section class="vp" id="vp${w}${who}"><h3>${w}px wide, ${who === "admin" ? "administrator" : "read-only member"}: <span class="mono">${ae.toLocaleString("en-US")}</span> pixels different (${pct}%)</h3>
<p>${note}</p>
${slider(`cmp${w}${who}`, `s5-connections-populated-${who}-${w}-before.png`, `s5-connections-populated-${who}-${w}-after.png`, w)}
<h4>Difference map (red = changed pixels, picture dimmed)</h4>
<img class="full" style="max-width:${Math.min(w, 700)}px" src="screenshots/s5-connections-populated-${who}-${w}-diff.png" alt="Difference map at ${w}">
<h4>Side by side (before on the left, after on the right)</h4>
<img class="full" style="max-width:100%" src="screenshots/s5-connections-populated-${who}-${w}-composite.png" alt="Side by side at ${w}">
</section>`;
const surfaces = [
  ["Connections, empty (admin / member)", "398", "398", "398", "footer text and the port in the Endpoints URLs"],
  ["Settings: General (fleet profile Save header)", "359", "359", "359", "footer text"],
  ["Settings: AI", "359", "359", "359", "footer text"],
  ["Settings: Members", "359", "359", "500", "footer text; one 'last signed in 1m / 2m ago' label at 1280"],
  ["Settings: Preferences", "359", "359", "359", "footer text"],
  ["Settings: Runner host", "359", "359", "359", "footer text"],
  ["Settings: API tokens", "359", "359", "359", "footer text"],
  ["New pool", "359", "359", "359", "footer text"],
  ["Pool settings, repository pool", "359", "359", "359", "footer text"],
  ["Pool settings, organization pool", "359", "359", "359", "footer text"],
];
const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>#145 spec audit (round 5): visual report</title>
<style>
:root{color-scheme:dark}body{margin:0;background:#0f1013;color:#e7e7ea;font:15px/1.55 system-ui,sans-serif}
header.banner{padding:28px 32px;background:#14301d;border-bottom:1px solid #2e7a4a}
header.banner h1{margin:0 0 6px;font-size:44px;letter-spacing:.5px;color:#7ee0a0}
nav{position:sticky;top:0;background:#16171b;padding:10px 32px;border-bottom:1px solid #2a2c33;z-index:5;display:flex;gap:16px;flex-wrap:wrap}
nav a{color:#9db4ff;text-decoration:none}main{max-width:1500px;margin:0 auto;padding:8px 32px 80px}
h2{margin-top:44px;border-bottom:1px solid #2a2c33;padding-bottom:6px}h4{margin:18px 0 6px;color:#b9bbc6;font-weight:600}
table{border-collapse:collapse;margin:12px 0}td,th{border:1px solid #2e3038;padding:6px 10px;text-align:left;vertical-align:top}
th{background:#1b1d23}.pill{font-size:12px;padding:2px 8px;border-radius:99px;margin-left:8px;vertical-align:middle}.ok{background:#16351f;color:#7ee0a0}.warn{background:#3b3216;color:#f0c969}.bad{background:#451c1c;color:#ff8f8f}
.mono,code{font-family:ui-monospace,monospace}code{background:#1b1d23;padding:1px 5px;border-radius:4px}
.cmp{max-width:900px}.stack{position:relative;border:1px solid #2e3038}.stack img{display:block;width:100%;height:auto}.stack img.top{position:absolute;inset:0;clip-path:inset(0 50% 0 0)}
.cmp input[type=range]{width:100%}.lbl{display:flex;justify-content:space-between;font-size:12px;color:#9a9ca8}
img.full{display:block;border:1px solid #2e3038;height:auto}
.crop{display:flex;gap:14px;flex-wrap:wrap}.crop img{max-width:100%;border:1px solid #2e3038}
.row{display:flex;gap:18px;flex-wrap:wrap;align-items:flex-start}.row img{border:1px solid #2e3038;max-width:100%;height:auto}figure{margin:0}figcaption{font-size:12px;color:#9a9ca8;margin-bottom:4px}
</style></head><body>
<header class="banner"><h1>PASS</h1>
<div>Issue #145, spec audit, round 5 (after the 360 px rework). The operator waived the pixel diff against the wireframe (brief, round-4 decision), so the comparison below is <b>before the fix against after the fix</b>, rendered headless in Chromium from the real app and API.</div>
<table><tr><th>Viewport</th><th>Pixels different (administrator / member)</th><th>Whole-page delta</th><th>What the difference is</th></tr>
<tr><td>360 x 2300</td><td>27,992 / 25,131</td><td>3.38% / 3.04%</td><td>The intended fix: connection names, row controls and the GitHub App header are laid out differently (all of it on the Connections page).</td></tr>
<tr><td>768 x 1500</td><td>398 / 398</td><td>0.035%</td><td>Only the build text in the footer and the port number inside the two Endpoints URLs.</td></tr>
<tr><td>1280 x 1500</td><td>398 / 398</td><td>0.021%</td><td>Same: footer build text and the port in the URLs.</td></tr></table>
<div>No horizontal overflow, no console error and no off-screen button at any width (the one "off-screen" item is the hidden Skip to content link).</div></header>
<nav><a href="#diff">What I see different</a><a href="#vp360admin">360</a><a href="#vp768admin">768</a><a href="#vp1280admin">1280</a><a href="#table">Section table</a><a href="#other">Other pages</a><a href="#after">Other rendered checks</a><a href="#follow">Follow-ups</a></nav>
<main>
<h2 id="diff">What I see different</h2>
<ul>
<li><b>360, Connections list:</b> every connection name is now readable ("acme-labs", "ada-admin", "Acme Bitbucket", "Platform Team"). Before the fix the first was invisible and the others were cut to "a...", "Acm...", "Platf...". The Configure, Edit and Remove buttons moved to their own right-aligned line under each row's text.</li>
<li><b>360, read-only member:</b> "Owner approval required" now sits on its own line under "Organization" instead of being drawn on top of it.</li>
<li><b>360, GitHub App section:</b> the title and its paragraph use the full width instead of a column about 70 px wide, and the three buttons (Refresh access, Install, Manage App) sit on lines below it. "Manage App" is fully on screen (before it ran off the right edge).</li>
<li><b>360, Connections header:</b> the "Add connection" button now sits under the description instead of squeezing it into a 178 px column.</li>
<li><b>360, a small side effect:</b> for a read-only member, each Bitbucket row has about 8 px of blank space under its text (the "Verified ... ago" label is hidden on phones but its container still takes a line gap). Cosmetic, left as is by F.</li>
<li><b>768 and 1280:</b> no visible differences from the pre-fix build. The only changed pixels are the footer build text and the port inside the Endpoints URLs (different test servers).</li>
<li><b>Every other settings page and both pool forms</b> (General, AI, Members, Preferences, Runner host, API tokens, new pool, pool settings) are pixel-identical to the pre-fix build at 360, 768 and 1280 apart from the footer text. The shared header change did not move anything on them.</li>
<li><b>One thing I only saw with test data I added (an 80-character connection name):</b> at 1280, a name that long is cut with an ellipsis and its "Bitbucket" badge wraps under it instead of sitting beside it. Realistic names are unaffected. Not blocking.</li>
<li><b>Still there, not from this story:</b> the Targets dropdown is cut off by its card in the new-pool form and pool settings (see Follow-ups).</li>
</ul>
<h2>Per-viewport comparators (Connections page, populated)</h2>
${block(360, "admin", 27992, "3.381", "The intended changes: rows, App header and Add connection button reflow. Drag the handle to wipe between the broken layout (left) and the fixed one (right).")}
${block(360, "member", 25131, "3.035", "Read-only member: the GitHub row's name is back and the status line no longer overlaps.")}
${block(768, "admin", 398, "0.035", "Identical apart from the footer text and the URL port.")}
${block(1280, "admin", 398, "0.021", "Identical apart from the footer text and the URL port.")}
<section class="vp"><h3>768 and 1280 as a member</h3><p>Also 398 pixels at both widths (same footer and port). Files: <a href="screenshots/s5-connections-populated-member-768-composite.png">768 composite</a>, <a href="screenshots/s5-connections-populated-member-1280-composite.png">1280 composite</a>.</p></section>
<h3>Close-ups of the three 360 fixes (before on the left, after on the right)</h3>
<div class="crop"><div><p>Rows, administrator</p><img src="screenshots/s5-closeup-rows-admin-360-before-after.png" alt="Rows before and after"></div>
<div><p>Rows, read-only member</p><img src="screenshots/s5-closeup-rows-member-360-before-after.png" alt="Member rows before and after"></div>
<div><p>GitHub App header, administrator</p><img src="screenshots/s5-closeup-app-header-admin-360-before-after.png" alt="App header before and after"></div></div>
<h2 id="table">Per-section delta table</h2>
<table><tr><th>Section</th><th>Viewport</th><th>Status</th><th>Description</th><th>Crop of the difference map</th></tr>
<tr><td>Connections rows (administrator)</td><td>360</td><td><span class="pill ok">MATCH to the brief, intended DELTA from before</span></td><td>Names readable; row controls on their own line.</td><td><img src="screenshots/t3-connections-rows-admin-360-crop-20261120.png" width="300" alt=""></td></tr>
<tr><td>Connections rows (member)</td><td>360</td><td><span class="pill ok">MATCH, intended DELTA</span></td><td>No overlap; name visible.</td><td><img src="screenshots/t3-connections-rows-member-360-crop-20261120.png" width="300" alt=""></td></tr>
<tr><td>GitHub App header</td><td>360</td><td><span class="pill ok">MATCH, intended DELTA</span></td><td>Title full width; all three buttons on screen.</td><td><img src="screenshots/t3-connections-app-header-admin-360-crop-20261120.png" width="300" alt=""></td></tr>
<tr><td>Top of the page and rows</td><td>768</td><td><span class="pill ok">MATCH</span></td><td>Only the footer text differs.</td><td><img src="screenshots/t3-connections-top-admin-768-crop-20261120.png" width="300" alt=""></td></tr>
<tr><td>Top of the page and rows</td><td>1280</td><td><span class="pill ok">MATCH</span></td><td>Only the footer text differs.</td><td><img src="screenshots/t3-connections-top-admin-1280-crop-20261120.png" width="300" alt=""></td></tr>
<tr><td>Targets dropdown (new pool, pool settings)</td><td>360, 768, 1280</td><td><span class="pill warn">DELTA, pre-existing</span></td><td>Cut off by its card; options cannot be hit with a mouse at rest. Same shared widget as the old form. See Follow-ups.</td><td><img src="screenshots/u-targets-dropdown-open-360.png" width="150" alt=""></td></tr>
<tr><td>Member Bitbucket rows (extra blank space)</td><td>360</td><td><span class="pill warn">DELTA, cosmetic</span></td><td>About 8 px of empty space under each row's text.</td><td></td></tr></table>
<h2 id="other">Every other surface, pre-fix build against fixed build (pixels different)</h2>
<table><tr><th>Surface</th><th>360 x 4200</th><th>768 x 3400</th><th>1280 x 3000</th><th>Cause of the difference</th></tr>
${surfaces.map(([n, a, b, c, why]) => `<tr><td>${n}</td><td>${a}</td><td>${b}</td><td>${c}</td><td>${why}</td></tr>`).join("\n")}
</table>
<p>Whole-page numbers over the full captured page (tall viewports, because the app scrolls inside its own container). Each is under 0.03%. For the empty Connections page the 398 px also holds at 360, which shows its layout did not change.</p>
<h2 id="after">Other rendered checks on the fixed build</h2>
<div class="row">
<figure><figcaption>Empty, administrator, 360</figcaption><img src="screenshots/s5-connections-empty-admin-360-after.png" width="240" alt=""></figure>
<figure><figcaption>Empty, member, 360</figcaption><img src="screenshots/s5-connections-empty-member-360-after.png" width="240" alt=""></figure>
<figure><figcaption>Editing a Bitbucket row, 360</figcaption><img src="screenshots/s5-connections-edit-row-360-after.png" width="240" alt=""></figure>
<figure><figcaption>Populated, 480 (below the sm breakpoint)</figcaption><img src="screenshots/s5-connections-populated-admin-480-after.png" width="300" alt=""></figure>
<figure><figcaption>Populated, 640 (at the sm breakpoint)</figcaption><img src="screenshots/s5-connections-populated-admin-640-after.png" width="360" alt=""></figure></div>
<p>Widths 320, 360, 480, 639, 640, 768, 1024 and 1280 were measured for both roles (<a href="screenshots/s5-geometry-after.json">s5-geometry-after.json</a>): no horizontal overflow, no overlap, names 66, 67, 98 and 90 px and unclipped at every width, "Manage App" inside the screen at every width. The full set of round-4 screenshots for the other surfaces (Add connection steps, new-pool form, pool settings, Platform options) is still valid for them, because their pixels did not change (table above); they are in <code>screenshots/s-*.png</code>.</p>
<h2 id="follow">Follow-ups (non-blocking; this verdict is PASS)</h2>
<ol>
<li><b>Targets dropdown clipped by its card (pre-existing, all widths).</b> Proposed branch: <code>fix/searchable-multi-select-clipped-listbox</code>. In the new-pool form and pool settings only the search box and one group heading show when it opens, so a mouse user cannot reach an option until the card is scrolled. The old repository picker had the same fault. Fix is in the shared <code>SearchableMultiSelect</code> / <code>SearchableSelect</code> (portal the list, or stop the card clipping). Operator decision needed: file it as its own issue (recommended) or accept as is.</li>
<li><b>Tidy-ups, optional:</b> the empty trailing wrapper on member Bitbucket rows at 360 (about 8 px); and the <code>3rem</code> in the row's text basis is a stand-in for the width of the leading avatar or dot.</li>
</ol>
</main></body></html>`;
fs.writeFileSync(`${D}/phase-9-platform-neutral-ui-report.html`, html);
console.log("report written", html.length);
