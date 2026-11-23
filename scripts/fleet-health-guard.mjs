#!/usr/bin/env node
// The `Fleet health guard` (#197 section 6): keeps each wave-1 fleet health pull request inside
// its own files, so that the issues of epic #198 can be done in parallel without touching one
// another's files.
//
// A pull request is checked against the wave-1 issue its description closes (`Closes`, `Fixes`
// or `Resolves #N`, any case; see `issuesInBody`), using docs/design/fleet-health-owners.json:
// - every changed file is one the issue owns (globs allowed), or a shared file where it may only
//   add lines (`addOnly`);
// - a contract test file (`contractTests`) stays #197's whatever glob it matches: the only change
//   allowed is removing the issue's own `contract: #N` markers (`#[ignore = "contract: #N"]`, or
//   `it.skip("[contract: #N] name"` becoming `it("name"`).
// A pull request that closes no wave-1 issue is out of scope and passes; one that closes two or
// more fails. Wave-2 issues (`wave2`, #204) are not checked.
//
// The checks are pure functions, tested in scripts/fleet-health-guard.test.mjs. Run as a script
// in CI, it reads the pull request from the GitHub Actions event and its changed files from the
// GitHub API (`GITHUB_TOKEN`), and exits 1 with one line per problem. No dependencies.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const CLOSING = /\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s*:?\s+#(\d+)\b/gi;

/** The parsed owners file at `path`. */
export function loadOwners(path) {
  const owners = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(owners.contractTests) || typeof owners.issues !== "object" || owners.issues === null) {
    throw new Error(`${path}: expected { contractTests: [...], issues: {...}, wave2: {...} }`);
  }
  return owners;
}

/** The wave-1 issues a `Closes`/`Fixes`/`Resolves #N` in `body` names, in order, each once. */
export function issuesInBody(body, owners) {
  const found = [];
  for (const match of String(body ?? "").matchAll(CLOSING)) {
    const issue = Number(match[1]);
    if (Object.hasOwn(owners.issues, String(issue)) && !found.includes(issue)) found.push(issue);
  }
  return found;
}

const globCache = new Map();

/** `glob` as a regular expression: `**` crosses directories, `*` and `?` stay inside one. */
function globRegExp(glob) {
  let cached = globCache.get(glob);
  if (cached) return cached;
  let source = "";
  for (let index = 0; index < glob.length; index += 1) {
    const character = glob[index];
    if (character === "*" && glob[index + 1] === "*") {
      const slash = glob[index + 2] === "/";
      source += slash ? "(?:.*/)?" : ".*";
      index += slash ? 2 : 1;
    } else if (character === "*") {
      source += "[^/]*";
    } else if (character === "?") {
      source += "[^/]";
    } else {
      source += character.replace(/[.+^${}()|[\]\\]/g, "\\$&");
    }
  }
  cached = new RegExp(`^${source}$`);
  globCache.set(glob, cached);
  return cached;
}

const isGlob = (pattern) => /[*?]/.test(pattern);
const matchesAny = (path, globs = []) => globs.some((glob) => globRegExp(glob).test(path));

/** The `-` and `+` lines of a unified diff, grouped into change blocks between context lines. */
function changeBlocks(patch) {
  const blocks = [];
  let block = null;
  for (const line of patch.split("\n")) {
    if (line.startsWith("-") || line.startsWith("+")) {
      block ??= { removed: [], added: [] };
      (line.startsWith("-") ? block.removed : block.added).push(line.slice(1));
    } else {
      if (block) blocks.push(block);
      block = null;
    }
  }
  if (block) blocks.push(block);
  return blocks;
}

/**
 * Whether `patch` only removes issue `issue`'s contract markers: a Rust `#[ignore = "contract: #N"]`
 * line taken out, or a Vitest `it.skip("[contract: #N] name"` turned into `it("name"` with the
 * rest of the line unchanged.
 */
function onlyRemovesMarkers(patch, issue) {
  const rust = new RegExp(`^\\s*#\\[ignore\\s*=\\s*"contract: #${issue}"\\]\\s*$`);
  const vitest = new RegExp(`^(.*\\b(?:it|test|describe))\\.skip\\((["'\`])\\[contract: #${issue}\\] `);
  for (const { removed, added } of changeBlocks(patch)) {
    const expected = [];
    for (const line of removed) {
      if (rust.test(line)) continue;
      const marked = vitest.exec(line);
      if (!marked) return false;
      expected.push(line.replace(vitest, (_, call, quote) => `${call}(${quote}`));
    }
    if (expected.length !== added.length || expected.some((line, index) => line !== added[index])) return false;
  }
  return true;
}

/** Whether `patch` only adds lines. */
const onlyAdds = (patch) => changeBlocks(patch).every((block) => block.removed.length === 0);

function checkFile(file, issue, entry, owners) {
  const { filename, status, previous_filename: previous, patch } = file;
  const paths = status === "renamed" && previous ? [previous, filename] : [filename];
  if (paths.some((path) => matchesAny(path, owners.contractTests))) {
    if (status !== "modified") return `${filename} is a contract test (#197's): it may not be ${status}`;
    if (typeof patch !== "string") return `${filename} is a contract test (#197's) and its diff could not be read`;
    if (!onlyRemovesMarkers(patch, issue)) {
      return `${filename} is a contract test (#197's): #${issue} may only remove its own "contract: #${issue}" markers`;
    }
    return null;
  }
  const outside = paths.filter((path) => !matchesAny(path, entry.owns));
  if (outside.length === 0) return null;
  if (outside.length === 1 && outside[0] === filename && status === "modified" && matchesAny(filename, entry.addOnly)) {
    if (typeof patch !== "string") return `${filename} is shared: #${issue} may only add lines, and its diff could not be read`;
    return onlyAdds(patch) ? null : `${filename} is shared: #${issue} may only add lines to it`;
  }
  return outside.map((path) => `${path} is outside #${issue}'s files (docs/design/fleet-health-owners.json)`).join("\n");
}

/** Checks a pull request's changed `files` against the wave-1 issue its `body` closes. */
export function checkPullRequest({ body, files, owners }) {
  const issues = issuesInBody(body, owners);
  if (issues.length === 0) return { ok: true, issue: null, errors: [] };
  if (issues.length > 1) {
    return {
      ok: false,
      issue: null,
      errors: [`the pull request closes ${issues.map((issue) => `#${issue}`).join(", ")}: one wave-1 issue per pull request`],
    };
  }
  const [issue] = issues;
  const entry = owners.issues[String(issue)];
  const errors = files.map((file) => checkFile(file, issue, entry, owners)).filter(Boolean).flatMap((error) => error.split("\n"));
  return { ok: errors.length === 0, issue, errors };
}

/**
 * The problems with the owners file against `repoFiles` (every file in the repository): a path a
 * wave-1 issue owns or shares that is not a glob and doesn't exist, and a file two wave-1 issues
 * own. A glob that matches nothing yet is fine, contract tests are #197's whatever globs match
 * them, and wave-2 entries are not checked (the guard runs for wave 1 only).
 */
export function checkOwners(owners, repoFiles) {
  const errors = [];
  const existing = new Set(repoFiles);
  const issues = Object.entries(owners.issues);
  for (const [issue, entry] of issues) {
    for (const path of [...(entry.owns ?? []), ...(entry.addOnly ?? [])]) {
      if (!isGlob(path) && !existing.has(path)) errors.push(`#${issue} lists ${path}, which does not exist`);
    }
  }
  for (const file of repoFiles) {
    if (matchesAny(file, owners.contractTests)) continue;
    const owning = issues.filter(([, entry]) => matchesAny(file, entry.owns)).map(([issue]) => `#${issue}`);
    if (owning.length > 1) errors.push(`${file} is owned by ${owning.join(" and ")}`);
  }
  return errors;
}

async function changedFiles(repository, number, token) {
  const files = [];
  for (let page = 1; ; page += 1) {
    const response = await fetch(`https://api.github.com/repos/${repository}/pulls/${number}/files?per_page=100&page=${page}`, {
      headers: { Accept: "application/vnd.github+json", Authorization: `Bearer ${token}`, "X-GitHub-Api-Version": "2022-11-28" },
    });
    if (!response.ok) throw new Error(`GitHub answered ${response.status} listing the pull request's files`);
    const batch = await response.json();
    files.push(...batch);
    if (batch.length < 100) return files;
  }
}

async function main() {
  const { GITHUB_EVENT_PATH: eventPath, GITHUB_REPOSITORY: repository, GITHUB_TOKEN: token } = process.env;
  if (!eventPath || !repository || !token) throw new Error("GITHUB_EVENT_PATH, GITHUB_REPOSITORY and GITHUB_TOKEN are required");
  const pullRequest = JSON.parse(readFileSync(eventPath, "utf8")).pull_request;
  if (!pullRequest) {
    console.log("Not a pull request: nothing to check.");
    return;
  }
  const owners = loadOwners(fileURLToPath(new URL("../docs/design/fleet-health-owners.json", import.meta.url)));
  if (issuesInBody(pullRequest.body, owners).length === 0) {
    console.log("The pull request closes no wave-1 fleet health issue: nothing to check.");
    return;
  }
  const result = checkPullRequest({ body: pullRequest.body, files: await changedFiles(repository, pullRequest.number, token), owners });
  if (result.ok) {
    console.log(`Every change stays inside #${result.issue}'s files.`);
    return;
  }
  for (const error of result.errors) console.error(`::error::${error}`);
  process.exitCode = 1;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main().catch((error) => {
    console.error(`::error::${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  });
}
