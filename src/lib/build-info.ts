/**
 * Build facts for the version bar. Pure helpers: the vite config calls them with what it read from the
 * Cargo manifest and git, and the app calls formatAppVersion with what vite baked in.
 */

/** The `version` of the `[workspace.package]` table of a Cargo manifest, or "" when the table or the key is missing. */
export function parseWorkspaceVersion(manifest: string): string {
  const table = /\[workspace\.package\]([^[]*)/.exec(manifest);
  const line = /^version\s*=\s*"([^"]+)"/m.exec(table?.[1] ?? "");
  return line?.[1] ?? "";
}

/** A build number: a positive integer without leading zeros, as `git rev-list --count HEAD` prints it. Anything else is "" (missing). */
export function normalizeBuildNumber(raw: string | undefined): string {
  const value = (raw ?? "").trim();
  return /^[1-9]\d*$/.test(value) ? value : "";
}

/** A commit id of 7 to 40 hex digits cut to the 7-character short form, in lower case. Anything else is "" (missing). */
export function normalizeCommit(raw: string | undefined): string {
  const value = (raw ?? "").trim();
  return /^[0-9a-fA-F]{7,40}$/.test(value) ? value.slice(0, 7).toLowerCase() : "";
}

/**
 * The text of the version bar: "v0.1.3 · build 412 · 9d92a33". A part that is missing says so ("build unknown",
 * "commit unknown", "version unknown") rather than being left out, so a build that lost its number is visible.
 */
export function formatAppVersion(version: string, buildNumber: string, commit: string): string {
  const v = version.trim();
  const b = buildNumber.trim();
  const c = commit.trim();
  return [v === "" ? "version unknown" : `v${v}`, b === "" ? "build unknown" : `build ${b}`, c === "" ? "commit unknown" : c].join(" · ");
}
