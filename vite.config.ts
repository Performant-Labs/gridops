import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import viteReact from "@vitejs/plugin-react";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

import { defineConfig } from "vite";

import { normalizeBuildNumber, normalizeCommit, parseWorkspaceVersion } from "./src/lib/build-info";

// The version bar names the build: the Cargo workspace version (one version for the server, the agent and this client), the
// build number (commits in the history of the commit built; scripts/build-info.sh prints it) and the short git commit. The
// Docker build has no .git, so it passes GRIDOPS_BUILD_NUMBER and GRIDOPS_COMMIT; any may be empty and the bar says what it lacks.
function buildVersion(): string {
  try {
    return parseWorkspaceVersion(readFileSync(new URL("./Cargo.toml", import.meta.url), "utf8"));
  } catch {
    return "";
  }
}

function git(...args: string[]): string {
  try {
    return execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return "";
  }
}

function buildNumber(): string {
  const fromEnv = process.env.GRIDOPS_BUILD_NUMBER?.trim();
  if (fromEnv) {
    const number = normalizeBuildNumber(fromEnv);
    if (number === "") console.warn(`GRIDOPS_BUILD_NUMBER must be a positive integer, got "${fromEnv}"; the build number will read unknown.`);
    return number;
  }
  // A shallow clone counts only the commits it fetched: a real-looking but wrong number, so leave it unknown.
  if (git("rev-parse", "--is-shallow-repository") !== "false") return "";
  return normalizeBuildNumber(git("rev-list", "--count", "HEAD"));
}

function buildCommit(): string {
  const fromEnv = process.env.GRIDOPS_COMMIT?.trim();
  return normalizeCommit(fromEnv || git("rev-parse", "--short=7", "HEAD"));
}

export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(buildVersion()),
    __APP_BUILD_NUMBER__: JSON.stringify(buildNumber()),
    __APP_COMMIT__: JSON.stringify(buildCommit()),
  },
  server: {
    port: 3000,
    strictPort: false,
    proxy: {
      "/api": "http://127.0.0.1:8080",
      "/auth": "http://127.0.0.1:8080",
    },
  },
  resolve: {
    tsconfigPaths: true,
  },
  plugins: [
    tanstackRouter({ target: "react", autoCodeSplitting: true }),
    viteReact(),
    tailwindcss(),
  ],
});
