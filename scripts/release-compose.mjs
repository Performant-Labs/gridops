// Prints compose.yaml with each locally built service switched to its
// published image, for the release's compose.release.yaml:
//
//   node scripts/release-compose.mjs 0.1.0 > compose.release.yaml
//
// Images come from this fork's GHCR namespace (Performant-Labs), matching
// the repository that serves release binaries; override with GRIDOPS_IMAGE_OWNER.
import { readFileSync } from "node:fs";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+([-+].+)?$/.test(version ?? "")) {
  throw new Error("usage: release-compose.mjs <version>");
}

const owner = process.env.GRIDOPS_IMAGE_OWNER ?? "performant-labs";
const source = readFileSync(new URL("../compose.yaml", import.meta.url), "utf8");
let replaced = 0;
// The published images are already built (and numbered), so the build-argument block at the top has no use in the release file.
const text = source.replace(/^# Every image is built with a build number\.[\s\S]*?\n\n(?=services:)/m, "").replace(/^ {4}build:\n {6}context: \.\n {6}target: ([a-z-]+)\n(?: {6}args: \*build-args\n)?/gm, (_, target) => {
  replaced += 1;
  return `    image: ghcr.io/${owner}/gridops-${target}:\${GRIDOPS_VERSION:-${version}}\n`;
});
if (replaced === 0 || /^\s+build:/m.test(text)) {
  throw new Error("compose.yaml has a build section this script doesn't recognise");
}

process.stdout.write(`# GridOps ${version}: the control plane from published images.
#
#   cp env.example .env   # then fill in the required values
#   docker compose -f compose.release.yaml up -d
#
# Set GRIDOPS_VERSION to run a different release.
${text}`);
