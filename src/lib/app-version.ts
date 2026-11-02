import { formatAppVersion } from "~/lib/build-info";

// Baked in by vite (`define` in vite.config.ts): the Cargo workspace version and the short git commit of the build.
// Not defined under vitest or a bare tsc, hence the typeof guards.
declare const __APP_VERSION__: string | undefined;
declare const __APP_COMMIT__: string | undefined;

export const APP_VERSION_LABEL = formatAppVersion(
  typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "",
  typeof __APP_COMMIT__ === "string" ? __APP_COMMIT__ : "",
);
