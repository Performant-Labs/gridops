/**
 * Source-level acceptance checks of #145, run as tests so they hold after the story:
 * - no string in src/ targets a redirecting settings route (only the two redirect-only route
 *   declarations may name their own path; the gitignored routeTree.gen.ts is excluded);
 * - only settings.index.tsx reads `appError`;
 * - the pool forms' shared rows and the shared components they render do not branch on a
 *   platform; the picker, the Platform options section and the Connections list contain no
 *   platform-specific code; the runner row has no platform ternary;
 * - the Bitbucket label rule exists once, in schemas.ts.
 * Extended by #141 (GitLab): GitLab ids and pool fields join the branching patterns; the delivery
 * list takes its platform from the registry; and GitLab code lives only in its descriptor, its
 * API functions, the wire-field files and its one registry entry.
 */
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const SRC = join(ROOT, "src");

function sourceFiles(directory = SRC): string[] {
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return name === "routeTree.gen.ts" ? [] : [path];
  });
}

/** `grep -rn -e PATTERN... src --exclude=routeTree.gen.ts` as "src/…:line:text" hits. */
function grep(patterns: RegExp[]) {
  return sourceFiles().flatMap((path) => readFileSync(path, "utf8").split("\n").flatMap((line, index) =>
    patterns.some((pattern) => pattern.test(line)) ? [`${relative(ROOT, path)}:${index + 1}:${line}`] : []));
}

const read = (path: string) => readFileSync(join(ROOT, path), "utf8");

describe("no link or redirect targets a redirecting route", () => {
  it("names /settings/github or /settings/integrations only in the two redirect-only route declarations", () => {
    const allowed = /^src\/routes\/settings\.(github|integrations)\.tsx:[0-9]+:export const Route = createFileRoute\("\/settings\/(github|integrations)"\)\(\{$/;
    const hits = grep([/settings\/github/, /settings\/integrations/]).filter((hit) => !allowed.test(hit));
    expect(hits).toEqual([]);
    // The exclusion only applies while each file still holds its own one-line declaration.
    expect(read("src/routes/settings.github.tsx").split("\n")).toContain('export const Route = createFileRoute("/settings/github")({');
    expect(read("src/routes/settings.integrations.tsx").split("\n")).toContain('export const Route = createFileRoute("/settings/integrations")({');
  });

  it("reads appError only in settings.index.tsx (the Connections page does not consume it)", () => {
    expect(new Set(grep([/appError/]).map((hit) => hit.split(":")[0]))).toEqual(new Set(["src/routes/settings.index.tsx"]));
  });
});

describe("shared rows do not branch on a platform", () => {
  const SHARED = [
    "src/routes/_app/runner-pools.new.tsx",
    "src/routes/_app/runner-pools.$poolId.settings.tsx",
  ];
  const BRANCHING = [
    /["'](github|bitbucket|gitlab)(\.[a-z-]+)?["']/, // a platform or kind id
    /\bpool\.scope\b/,
    /\bscope\s*[!=]==/,
    /bitbucketConnectionIds\.length/,
    /runUntagged|gitlabConnectionIds/, // GitLab pool fields travel inside the descriptor's partials only
  ];

  it.each(SHARED)("%s compares against no platform id, kind id, scope or platform-named field", (path) => {
    const lines = read(path).split("\n").flatMap((line, index) => BRANCHING.some((pattern) => pattern.test(line)) ? [`${index + 1}: ${line.trim()}`] : []);
    expect(lines).toEqual([]);
  });

  it("the Tart base VM row and the plan preview carry no platform text or prop", () => {
    expect(read("src/features/runner-pools/tart-base-vm-row.tsx")).not.toMatch(/bitbucket/i);
    expect(read("src/components/pool-plan-preview.tsx")).not.toMatch(/github|bitbucket|\bscope\b/i);
  });

  it("the Targets row, Platform options section and Connections list contain no platform-specific code", () => {
    for (const path of [
      "src/features/runner-pools/targets-row.tsx",
      "src/features/runner-pools/platform-options-section.tsx",
      "src/features/platform-connections/connections-section.tsx",
    ]) {
      expect(existsSync(join(ROOT, path)), `${path} exists`).toBe(true);
      if (existsSync(join(ROOT, path))) expect(read(path)).not.toMatch(/github|bitbucket|gitlab/i);
    }
  });

  it("the runner row takes platform names from the registry, with no platform ternary", () => {
    const row = read("src/features/runners/runner-row.tsx");
    expect(row).not.toMatch(/runner\.platform\s*[!=]==/);
    expect(row).not.toMatch(/["'](github|bitbucket|gitlab)["']/);
    expect(row).toMatch(/from "~\/features\/platform-connections\/registry"/);
  });

  it("the deliveries list takes each delivery's platform and fallback destination from the registry, with no platform comparison", () => {
    const page = read("src/routes/_app/webhooks.tsx");
    expect(page).not.toMatch(/["'](github|bitbucket|gitlab)["']/);
    expect(page).not.toMatch(/\.(platform|id)\s*[!=]==\s*["']/);
    expect(page).not.toMatch(/\.startsWith\(\s*["']/);
    // The fallback destination is GitHub's descriptor value now, not page copy.
    expect(page).not.toMatch(/["']GitHub App["']/);
    expect(page).toMatch(/from "~\/features\/platform-connections\/registry"/);
  });
});

describe("GitLab code has one home (#141)", () => {
  /** The files that may name GitLab: the descriptor, its API functions, and the wire-field files. */
  const ALLOWED = [
    "src/features/platform-connections/gitlab.tsx",
    "src/features/platform-connections/platform-connections.functions.ts",
    "src/features/runner-pools/schemas.ts",
    "src/features/runner-pools/runner-pools.functions.ts",
    "src/features/operations/operations.functions.ts",
  ];
  const REGISTRY = "src/features/platform-connections/registry.tsx";

  it("lives in gitlab.tsx, and outside it only in the API functions, the wire-field files and the registry", () => {
    expect(existsSync(join(ROOT, ALLOWED[0] ?? "")), "gitlab.tsx exists").toBe(true);
    const files = new Set(grep([/gitlab/i]).map((hit) => hit.split(":")[0]));
    expect(files.has(ALLOWED[0] ?? "")).toBe(true);
    expect([...files].filter((path) => path !== REGISTRY && !ALLOWED.includes(path ?? "")).sort()).toEqual([]);
  });

  it("the registry names GitLab only in its import, its array entry and comments", () => {
    const lines = read(REGISTRY).split("\n").filter((line) => /gitlab/i.test(line));
    expect(lines.some((line) => /^import \{ gitlabPlatform \} from "\.\/gitlab";$/.test(line.trim()))).toBe(true);
    const other = lines.filter((line) => {
      const text = line.trim();
      return !/^import \{ gitlabPlatform \} from "\.\/gitlab";$/.test(text)
        && !/^(\/\/|\*|\/\*)/.test(text)
        && !/^export const platforms\b.*\bgitlabPlatform\b/.test(text)
        && !/^gitlabPlatform,?$/.test(text);
    });
    expect(other).toEqual([]);
  });
});

describe("the Bitbucket label rule has one home", () => {
  it("appears only in schemas.ts", () => {
    expect(grep([/\^\[a-z0-9\.\]\+\$/]).map((hit) => hit.split(":")[0])).toEqual(["src/features/runner-pools/schemas.ts"]);
  });
});
