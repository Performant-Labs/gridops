// Prints compose.yaml with each locally built service switched to its
// published image, for the release's compose.release.yaml:
//
//   node scripts/release-compose.mjs 0.1.0 > compose.release.yaml
import { readFileSync } from "node:fs";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+([-+].+)?$/.test(version ?? "")) {
  throw new Error("usage: release-compose.mjs <version>");
}

const source = readFileSync(new URL("../compose.yaml", import.meta.url), "utf8");
let replaced = 0;
const text = source.replace(/^ {4}build:\n {6}context: \.\n {6}target: ([a-z-]+)\n/gm, (_, target) => {
  replaced += 1;
  return `    image: ghcr.io/iamngoni/gridops-${target}:\${GRIDOPS_VERSION:-${version}}\n`;
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
