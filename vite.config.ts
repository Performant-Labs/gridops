import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import viteReact from "@vitejs/plugin-react";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

import { defineConfig } from "vite";

import { parseWorkspaceVersion } from "./src/lib/build-info";

// The version bar names the build: the Cargo workspace version (one version for the server, the agent and this client) and the
// short git commit. The Docker build has no .git, so it passes GRIDOPS_COMMIT; either may be empty and the bar says what it lacks.
function buildVersion(): string {
  try {
    return parseWorkspaceVersion(readFileSync(new URL("./Cargo.toml", import.meta.url), "utf8"));
  } catch {
    return "";
  }
}

function buildCommit(): string {
  const fromEnv = process.env.GRIDOPS_COMMIT?.trim();
  if (fromEnv) return fromEnv.slice(0, 7);
  try {
    return execFileSync("git", ["rev-parse", "--short=7", "HEAD"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return "";
  }
}

export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(buildVersion()),
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
