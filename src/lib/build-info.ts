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

/** The text of the version bar: "v0.1.3 · 9d92a33", "v0.1.3" without a commit, or "version unknown" without a version. */
export function formatAppVersion(version: string, commit: string): string {
  const v = version.trim();
  const c = commit.trim();
  if (v === "") return "version unknown";
  return c === "" ? `v${v}` : `v${v} · ${c}`;
}
