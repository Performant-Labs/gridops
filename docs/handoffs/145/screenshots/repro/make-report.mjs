import fs from "node:fs";
const D = process.argv[2]; // docs/handoffs/145
const S = `${D}/screenshots`;
const exists = (f) => fs.existsSync(`${S}/${f}`);
const W = [360, 768, 1280];
const surfaces = [
  ["Connections, nothing connected (administrator, GitHub App not configured)", "connections-empty-admin-full", "MATCH", "One 'No connections yet' state with a single Add connection button. No per-platform empty rows underneath."],
  ["Connections, nothing connected (read-only member)", "connections-empty-member", "MATCH", "Read-only note, 'No connections yet', administrator copy, no button."],
  ["Connections, populated (administrator)", "connections-populated-admin-full", "REWORK at 360", "At 768 and 1280 the rows and App section are fine. At 360 the connection names collapse and 'Manage App' is pushed off-screen."],
  ["Connections, populated (read-only member)", "connections-populated-member-full", "REWORK at 360", "At 360 the GitHub row has no visible name and 'Owner approval required' overlaps 'Organization'."],
  ["Add connection: chooser (nothing connected)", "add-chooser-empty", "MATCH", "GitHub and Bitbucket listed, each with its one-line description."],
  ["Add connection: chooser (populated)", "add-chooser-populated", "MATCH", "Same chooser."],
  ["Add connection: GitHub step, App not configured", "add-github-linkout-unconfigured", "MATCH", "Install button disabled, warning says to create the App first, Back works."],
  ["Add connection: GitHub step, App configured", "add-github-linkout-configured", "MATCH", "Primary 'Install GitHub App' link with external-link icon, Back."],
  ["Add connection: Bitbucket form", "add-bitbucket-form", "MATCH", "Three fields, scopes help, Back and Connect workspace."],
  ["New pool: full form", "new-pool-form-full", "MATCH", "One Targets row; no scope, installation or Bitbucket rows."],
  ["New pool: Targets picker open", "new-pool-targets-open", "DELTA (pre-existing)", "The dropdown is cut off by its card: only the search box shows. The old repository picker was cut off in the same way (see the notes)."],
  ["New pool: picker open after picking a repository", "new-pool-targets-repo-picked-open", "DELTA (pre-existing)", "The card scrolled inside itself to reveal the option; the heading and trigger moved out of view."],
  ["New pool: repository and Bitbucket picked, Platform options open", "new-pool-mixed-platform-options-full", "MATCH", "Collapsed section opens to the Bitbucket notes; labels row help is Bitbucket-aware."],
  ["Pool settings: repository pool with a Bitbucket workspace, Platform options open", "pool-settings-mixed-full", "MATCH", "Removable GitHub chips, locked Bitbucket chip, Platform options section."],
  ["Pool settings: Targets picker open", "pool-settings-targets-open", "DELTA (pre-existing)", "Same cut-off dropdown as the new-pool form."],
  ["Pool settings: organization pool", "pool-settings-org-full", "MATCH", "Single runner-group target."],
];
const slider = (id, a, b, la, lb, w) => `
<div class="cmp" style="max-width:${Math.min(w, 1000)}px"><div class="lbl"><span>${la}</span><span>${lb}</span></div>
<div class="stack" id="${id}"><img src="screenshots/${b}" alt="${lb}"><img class="top" src="screenshots/${a}" alt="${la}"></div>
<input type="range" min="0" max="100" value="50" aria-label="Wipe between ${la} and ${lb}" oninput="document.getElementById('${id}').querySelector('.top').style.clipPath='inset(0 '+(100-this.value)+'% 0 0)'">
</div>`;
const gallery = surfaces.map(([title, stem, status, note], i) => `
<section class="surf" id="s${i}"><h3>${title} <span class="pill ${status.startsWith("MATCH") ? "ok" : status.startsWith("DELTA") ? "warn" : "bad"}">${status}</span></h3><p>${note}</p>
<div class="row">${W.filter((w) => exists(`s-${stem}-${w}.png`)).map((w) => `<figure><figcaption>${w}px wide</figcaption><a href="screenshots/s-${stem}-${w}.png"><img loading="lazy" src="screenshots/s-${stem}-${w}.png" alt="${title} at ${w}"></a></figure>`).join("")}</div></section>`).join("\n");
const cmpBlocks = W.map((w) => `
<section class="vp" id="vp${w}"><h3>${w}px: old GitHub settings page (left side of the slider) against the new Connections page</h3>
<p>The two pages are different layouts by design, so this is for eyeballing, not a pixel match. Drag the slider to wipe from the old page (before this story) to the new one.</p>
${slider("cmp" + w, `base-github-page-full-${w}.png`, `s-connections-populated-admin-full-${w}.png`, "Before (old GitHub page)", "After (new Connections page)", w)}
</section>`).join("\n");
const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>#145 spec audit: visual report</title>
<style>
:root{color-scheme:dark}body{margin:0;background:#0f1013;color:#e7e7ea;font:15px/1.55 system-ui,sans-serif}
header.banner{padding:28px 32px;background:#3b1d1d;border-bottom:1px solid #7a2e2e}
header.banner h1{margin:0 0 6px;font-size:40px;letter-spacing:.5px}
nav{position:sticky;top:0;background:#16171b;padding:10px 32px;border-bottom:1px solid #2a2c33;z-index:5;display:flex;gap:16px;flex-wrap:wrap}
nav a{color:#9db4ff;text-decoration:none}main{max-width:1500px;margin:0 auto;padding:8px 32px 80px}
h2{margin-top:44px;border-bottom:1px solid #2a2c33;padding-bottom:6px}table{border-collapse:collapse;margin:12px 0}td,th{border:1px solid #2e3038;padding:6px 10px;text-align:left;vertical-align:top}
th{background:#1b1d23}.pill{font-size:12px;padding:2px 8px;border-radius:99px;margin-left:8px;vertical-align:middle}.ok{background:#16351f;color:#7ee0a0}.warn{background:#3b3216;color:#f0c969}.bad{background:#451c1c;color:#ff8f8f}
.row{display:flex;gap:18px;align-items:flex-start;flex-wrap:wrap}figure{margin:0}figure img{display:block;border:1px solid #2e3038;max-width:100%;height:auto}
.row figure:nth-child(1) img{width:240px}.row figure:nth-child(2) img{width:420px}.row figure:nth-child(3) img{width:560px}
figcaption{font-size:12px;color:#9a9ca8;margin-bottom:4px}code{background:#1b1d23;padding:1px 5px;border-radius:4px}
.cmp{max-width:900px}.stack{position:relative;border:1px solid #2e3038}.stack img{display:block;width:100%;height:auto}.stack img.top{position:absolute;inset:0;clip-path:inset(0 50% 0 0)}
.cmp input[type=range]{width:100%}.lbl{display:flex;justify-content:space-between;font-size:12px;color:#9a9ca8}
.crop{display:flex;gap:14px;flex-wrap:wrap}.crop img{max-width:100%;border:1px solid #2e3038}
</style></head><body>
<header class="banner"><h1>REWORK</h1>
<div>Issue #145, spec audit. Pixel diff waived by the operator (brief, round-4 decision): there is no comparable reference. What I measured instead is below.</div>
<table><tr><th>Viewport</th><th>Pixel diff</th><th>Horizontal overflow</th><th>Defects introduced by this story</th><th>Console errors</th></tr>
<tr><td>360 x 800</td><td>waived</td><td>none (page 360 / 360)</td><td><b>2</b> (connection names collapse, "Manage App" off-screen)</td><td>none</td></tr>
<tr><td>768 x 1024</td><td>waived</td><td>none</td><td>0</td><td>none</td></tr>
<tr><td>1280 x 800</td><td>waived</td><td>none</td><td>0</td><td>none</td></tr></table></header>
<nav><a href="#diff">What I see different</a><a href="#vp360">360</a><a href="#vp768">768</a><a href="#vp1280">1280</a><a href="#table">Section table</a><a href="#gallery">All screenshots</a><a href="#recs">Rework items</a></nav>
<main>
<h2 id="diff">What I see different</h2>
<ul>
<li><b>On a phone-width screen (360), the Connections list no longer shows connection names.</b> The GitHub row for "acme-labs" shows only its avatar, a "GitHub" badge and the word "Organization": the name has shrunk to zero width. "ada-admin" shows as "a...", "Acme Bitbucket" as "Acm...", "Platform Team" as "Platf...". Before this story the same names were readable on the old pages.</li>
<li><b>At 360 a read-only member sees text on top of text</b> in the GitHub row: "Owner approval required" is drawn over "Organization".</li>
<li><b>At 360 the "GitHub App" section header is squeezed.</b> The title and its paragraph sit in a column about 70 px wide (the title wraps to two lines, the paragraph runs about twenty lines), because the three buttons beside it (Refresh access, Install, Manage App) take the rest. "Manage App" runs off the right edge of the screen (it spans 327 to 442 px on a 360 px screen), so an administrator cannot reach it without a sideways scroll that the page does not offer.</li>
<li><b>The Targets dropdown is cut off by its card</b> in the new-pool form and in pool settings, at every width: you see the search box, not the options or the group headings. The old repository dropdown was cut off in the same way (one option showed at 768 and 1280, none at 360, none in the old settings form), so this is not new, but it hides the grouped options and disabled reasons that are the point of this story.</li>
<li>Everything else I looked at matches the brief at all three widths: the empty states for an administrator and a member, the Add connection chooser, the GitHub step (enabled and disabled), the Bitbucket form, Back, the new-pool form, pool settings and the Platform options section. No horizontal overflow and no console error at any width.</li>
</ul>
<h2>Per-viewport comparators</h2>
${cmpBlocks}
<h3>Close-ups of the 360 problems (before on the left, after on the right)</h3>
<div class="crop"><div><p>Rows: old Bitbucket page, old GitHub installations, new Connections list</p><img src="screenshots/s-composite-D1-baseline-vs-head-360.png" alt="Rows before and after at 360"></div>
<div><p>Header actions: old Installations section, new GitHub App section</p><img src="screenshots/s-composite-D2-baseline-vs-head-360.png" alt="GitHub App header before and after at 360"></div>
<div><p>Read-only member at 360</p><img src="screenshots/s-crop-D1-rows-member-head-360.png" alt="Member rows at 360"></div></div>
<h3>Targets dropdown, before (old repository picker, 1280)</h3><div class="crop"><img src="screenshots/base-new-pool-repo-picker-open-1280.png" alt="Old repository picker open at 1280" style="width:640px"></div>
<h2 id="table">Per-section delta table</h2>
<table><tr><th>Section</th><th>Viewport</th><th>Status</th><th>Description</th><th>Crop</th></tr>
<tr><td>Connections list rows</td><td>360</td><td><span class="pill bad">REWORK</span></td><td>Names collapse to 0 to 45 px; the organization name is invisible.</td><td><img src="screenshots/s-crop-D1-rows-head-360.png" width="300" alt=""></td></tr>
<tr><td>Connections list rows (member)</td><td>360</td><td><span class="pill bad">REWORK</span></td><td>Status text overlaps the kind label.</td><td><img src="screenshots/s-crop-D1-rows-member-head-360.png" width="300" alt=""></td></tr>
<tr><td>GitHub App section header</td><td>360</td><td><span class="pill bad">REWORK</span></td><td>Heading squeezed to 73 px; Manage App off-screen.</td><td><img src="screenshots/s-crop-D2-github-app-header-head-360.png" width="300" alt=""></td></tr>
<tr><td>Targets dropdown</td><td>360, 768, 1280</td><td><span class="pill warn">DELTA</span></td><td>Cut off by its card; pre-existing, not introduced here.</td><td><img src="screenshots/s-new-pool-targets-open-1280.png" width="300" alt=""></td></tr>
<tr><td>All other sections</td><td>360, 768, 1280</td><td><span class="pill ok">MATCH</span></td><td>See the gallery.</td><td></td></tr></table>
<h2 id="gallery">All screenshots</h2>
${gallery}
<h2 id="recs">Rework items for F (one pass)</h2>
<ol>
<li><b>Branch:</b> the current branch (<code>claude/ui-145-platform-neutral-cf5832</code>). <b>Problem:</b> at 360 the Connections list rows lose the connection name and the member's status text overlaps the kind label. <b>Needs from the operator:</b> nothing; the old pages showed the name, so the new row must too.</li>
<li><b>Branch:</b> same. <b>Problem:</b> at 360 the GitHub App section's header squeezes its title and paragraph and pushes "Manage App" off-screen. <b>Needs from the operator:</b> nothing.</li>
<li><b>Optional, operator's call:</b> the Targets dropdown clipping (pre-existing). Fold into this pass or raise a separate issue.</li>
</ol>
</main></body></html>`;
fs.writeFileSync(`${D}/phase-9-platform-neutral-ui-report.html`, html);
console.log("report written", html.length);
