// #197: the tests of the `Fleet health guard` (scripts/fleet-health-guard.mjs). Run with
// `node --test scripts/fleet-health-guard.test.mjs` (the CI job runs them before the guard).
//
// The guard is pure over its inputs:
// - `checkPullRequest({ body, files, owners })` -> `{ ok, issue, errors }`. `body` is the PR
//   description; `files` are the PR's changed files in the GitHub "list pull request files"
//   shape (`{ filename, status, previous_filename?, patch? }`, status one of added, modified,
//   removed, renamed); `owners` is docs/design/fleet-health-owners.json. `issue` is the wave-1
//   issue the PR closes (a number), or null when it names none.
// - `issuesInBody(body, owners)` -> the wave-1 issues a `Closes`/`Fixes`/`Resolves #N` names.
// - `checkOwners(owners, repoFiles)` -> a list of errors (empty when the owners file is sound).
// - `loadOwners(path)` -> the parsed owners file.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";

import { checkOwners, checkPullRequest, issuesInBody, loadOwners } from "./fleet-health-guard.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** A small owners file in the real one's shape. */
const owners = {
  contractTests: ["crates/**/*_contract_tests.rs", "tests/fleet-health-*.contract.test.ts"],
  issues: {
    191: { owns: ["crates/gridops-reconciler/src/fleet_health_rollup.rs"], addOnly: [] },
    194: { owns: ["crates/gridops-api/src/fleet/health.rs", "crates/gridops-api/src/fleet/health_tests.rs"], addOnly: [] },
    202: {
      owns: ["src/features/fleet-health/machine-*.tsx", "tests/fleet-health-202-*.test.ts"],
      addOnly: ["src/routes/_app/fleet.machines.$hostId.tsx"],
    },
    203: { owns: ["src/features/fleet-health/live-*.tsx"], addOnly: ["src/features/dashboard/metric-tiles.ts"] },
  },
  wave2: { 204: { owns: ["crates/gridops-api/src/fleet/health_source.rs"], addOnly: [] } },
};

const modified = (filename, patch = "@@ -1 +1,2 @@\n line\n+added") => ({ filename, status: "modified", patch });
const check = (body, files) => checkPullRequest({ body, files, owners });

describe("which issue a pull request is for", () => {
  it("finds Closes, Fixes and Resolves in any case, and only wave-1 issues", () => {
    assert.deepEqual(issuesInBody("Closes #191", owners), [191]);
    assert.deepEqual(issuesInBody("fixes #194 and more", owners), [194]);
    assert.deepEqual(issuesInBody("RESOLVES #202", owners), [202]);
    assert.deepEqual(issuesInBody("Resolved #203", owners), [203]);
    assert.deepEqual(issuesInBody("See #191; closes #999", owners), []);
    assert.deepEqual(issuesInBody("Closes #204", owners), [], "#204 is wave 2");
  });

  it("passes a pull request that names no wave-1 issue, whatever it changes", () => {
    const result = check("Closes #999\n\nAn unrelated fix.", [modified("crates/gridops-api/src/main.rs")]);
    assert.equal(result.ok, true);
    assert.equal(result.issue, null);
    assert.deepEqual(result.errors, []);
    assert.equal(check("Closes #204", [modified("crates/gridops-api/src/fleet/health.rs")]).ok, true);
  });

  it("fails a pull request that names two wave-1 issues", () => {
    const result = check("Closes #191\nCloses #194", [modified("crates/gridops-reconciler/src/fleet_health_rollup.rs")]);
    assert.equal(result.ok, false);
    assert.match(result.errors.join("\n"), /#191/);
    assert.match(result.errors.join("\n"), /#194/);
  });
});

describe("the files a wave-1 pull request may change", () => {
  it("passes changes to the issue's own files, globs included", () => {
    const result = check("Closes #202", [
      modified("src/features/fleet-health/machine-score-table.tsx"),
      { filename: "src/features/fleet-health/machine-sparkline.tsx", status: "added", patch: "@@ -0,0 +1 @@\n+x" },
      { filename: "tests/fleet-health-202-sorting.test.ts", status: "added", patch: "@@ -0,0 +1 @@\n+x" },
    ]);
    assert.deepEqual(result, { ok: true, issue: 202, errors: [] });
  });

  it("fails a change to a file outside the issue's list, naming it", () => {
    const result = check("Closes #191", [
      modified("crates/gridops-reconciler/src/fleet_health_rollup.rs"),
      modified("crates/gridops-reconciler/src/main.rs"),
    ]);
    assert.equal(result.ok, false);
    assert.equal(result.issue, 191);
    assert.equal(result.errors.length, 1);
    assert.match(result.errors[0], /crates\/gridops-reconciler\/src\/main\.rs/);
  });

  it("fails another issue's file, and a file deleted or renamed out of the list", () => {
    assert.equal(check("Closes #191", [modified("crates/gridops-api/src/fleet/health.rs")]).ok, false);
    assert.equal(check("Closes #191", [{ filename: "src/lib/api.ts", status: "removed", patch: "@@ -1 +0,0 @@\n-x" }]).ok, false);
    const renamedAway = {
      filename: "crates/gridops-reconciler/src/fleet_health_rollup.rs",
      previous_filename: "crates/gridops-reconciler/src/fleet.rs",
      status: "renamed",
    };
    assert.equal(check("Closes #191", [renamedAway]).ok, false, "the old path is not #191's");
  });

  it("lets a shared file take added lines only", () => {
    const added = modified("src/features/dashboard/metric-tiles.ts", "@@ -10,2 +10,4 @@\n context\n+  tile,\n+  other,\n context");
    assert.equal(check("Closes #203", [added]).ok, true);
    const edited = modified("src/features/dashboard/metric-tiles.ts", "@@ -10,2 +10,2 @@\n context\n-  old,\n+  new,");
    const result = check("Closes #203", [edited]);
    assert.equal(result.ok, false);
    assert.match(result.errors.join("\n"), /metric-tiles\.ts/);
    assert.equal(check("Closes #202", [added]).ok, false, "#203's shared file is not #202's");
  });
});

describe("contract tests", () => {
  const rust = "crates/gridops-reconciler/src/fleet_health_rollup_contract_tests.rs";
  const vitest = "tests/fleet-health-202-machines.contract.test.ts";

  it("lets an issue remove its own markers and nothing else", () => {
    const unmarkRust = modified(rust, '@@ -40,3 +40,2 @@\n #[tokio::test]\n-#[ignore = "contract: #191"]\n async fn rows() -> Result<()> {');
    assert.deepEqual(check("Closes #191", [unmarkRust]), { ok: true, issue: 191, errors: [] });
    const unmarkVitest = modified(
      vitest,
      '@@ -40,1 +40,1 @@\n-  it.skip("[contract: #202] sorts the machines worst first", async () => {\n+  it("sorts the machines worst first", async () => {',
    );
    assert.deepEqual(check("Closes #202", [unmarkVitest]), { ok: true, issue: 202, errors: [] });
  });

  it("fails removing another issue's marker", () => {
    const other = modified(rust, '@@ -40,3 +40,2 @@\n #[tokio::test]\n-#[ignore = "contract: #194"]\n async fn rows() -> Result<()> {');
    assert.equal(check("Closes #191", [other]).ok, false);
  });

  it("fails any other edit to a contract test, even next to an allowed one", () => {
    const edited = modified(rust, '@@ -40,3 +40,3 @@\n-    assert_eq!(rows, expected);\n+    assert!(rows.len() > 0);');
    const result = check("Closes #191", [edited]);
    assert.equal(result.ok, false);
    assert.match(result.errors.join("\n"), /fleet_health_rollup_contract_tests\.rs/);
    const renamedTest = modified(
      vitest,
      '@@ -40,1 +40,1 @@\n-  it.skip("[contract: #202] sorts the machines worst first", async () => {\n+  it("sorts machines", async () => {',
    );
    assert.equal(check("Closes #202", [renamedTest]).ok, false, "the test keeps its name");
    const both = modified(rust, '@@ -40,4 +40,3 @@\n-#[ignore = "contract: #191"]\n async fn rows() -> Result<()> {\n-    let a = 1;\n+    let a = 2;');
    assert.equal(check("Closes #191", [both]).ok, false);
    assert.equal(check("Closes #191", [{ filename: rust, status: "removed", patch: "@@ -1 +0,0 @@\n-x" }]).ok, false);
  });

  it("keeps a contract test #197's even when an issue's glob matches it", () => {
    const insideGlob = "tests/fleet-health-202-detail.contract.test.ts";
    const result = check("Closes #202", [modified(insideGlob, "@@ -1 +1 @@\n-  expect(a).toBe(1);\n+  expect(a).toBe(2);")]);
    assert.equal(result.ok, false, "tests/fleet-health-202-*.test.ts matches it, but it is a contract test");
  });
});

describe("the owners file", () => {
  it("reports an owned file that doesn't exist and a file two issues own", () => {
    const files = ["crates/gridops-reconciler/src/fleet_health_rollup.rs", "src/features/fleet-health/machine-a.tsx"];
    const broken = {
      ...owners,
      issues: {
        191: { owns: ["crates/gridops-reconciler/src/fleet_health_rollup.rs"], addOnly: [] },
        194: { owns: ["crates/gridops-api/src/fleet/health.rs"], addOnly: [] },
        202: { owns: ["src/features/fleet-health/machine-*.tsx"], addOnly: [] },
        203: { owns: ["src/features/fleet-health/machine-a.tsx"], addOnly: [] },
      },
    };
    const errors = checkOwners(broken, files).join("\n");
    assert.match(errors, /crates\/gridops-api\/src\/fleet\/health\.rs/, "a missing file is named");
    assert.match(errors, /machine-a\.tsx/, "a doubly owned file is named");
    assert.match(errors, /#202/);
    assert.match(errors, /#203/);
    assert.deepEqual(checkOwners(owners, [
      "crates/gridops-reconciler/src/fleet_health_rollup.rs",
      "crates/gridops-api/src/fleet/health.rs",
      "crates/gridops-api/src/fleet/health_tests.rs",
      "src/routes/_app/fleet.machines.$hostId.tsx",
      "src/features/dashboard/metric-tiles.ts",
    ]), [], "a glob that matches nothing yet is fine");
  });

  it("is sound for this repository, with every wave-1 issue in it", () => {
    const real = loadOwners(join(root, "docs/design/fleet-health-owners.json"));
    const files = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard"], { cwd: root, encoding: "utf8" })
      .split("\n")
      .filter(Boolean);
    assert.deepEqual(checkOwners(real, files), []);
    assert.deepEqual(
      Object.keys(real.issues).map(Number).sort((a, b) => a - b),
      [190, 191, 192, 193, 194, 195, 199, 200, 201, 202, 203],
    );
    assert.deepEqual(Object.keys(real.wave2).map(Number), [204]);
  });
});
