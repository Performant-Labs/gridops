import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { formatAppVersion, normalizeBuildNumber, normalizeCommit, parseWorkspaceVersion } from "~/lib/build-info";

const read = (path: string) => readFileSync(fileURLToPath(new URL(path, import.meta.url)), "utf8");

describe("parseWorkspaceVersion", () => {
  it("reads the version of the workspace package table", () => {
    expect(parseWorkspaceVersion('[workspace]\nmembers = ["crates/a"]\n\n[workspace.package]\nversion = "0.1.3"\nedition = "2024"\n')).toBe("0.1.3");
  });

  it("ignores a version outside that table", () => {
    expect(parseWorkspaceVersion('[package]\nversion = "9.9.9"\n\n[workspace.package]\nedition = "2024"\n')).toBe("");
    expect(parseWorkspaceVersion('[workspace.package]\nedition = "2024"\n\n[dependencies]\nversion = "9.9.9"\n')).toBe("");
  });

  it("is empty without the table or for an empty manifest", () => {
    expect(parseWorkspaceVersion('[package]\nversion = "1.0.0"\n')).toBe("");
    expect(parseWorkspaceVersion("")).toBe("");
  });

  it("finds a version in the repository's own manifest", () => {
    expect(parseWorkspaceVersion(read("../Cargo.toml"))).toMatch(/^\d+\.\d+\.\d+/);
  });
});

describe("normalizeBuildNumber", () => {
  it("accepts a positive integer, as `git rev-list --count` prints it", () => {
    expect(normalizeBuildNumber("412")).toBe("412");
    expect(normalizeBuildNumber(" 7\n")).toBe("7");
  });

  it("rejects anything else, so a bad value reads as missing instead of being shown", () => {
    for (const bad of ["", "  ", "0", "012", "-3", "4.1", "dev", "12a", undefined]) {
      expect(normalizeBuildNumber(bad), String(bad)).toBe("");
    }
  });
});

describe("normalizeCommit", () => {
  it("cuts a commit to seven lower-case hex digits", () => {
    expect(normalizeCommit("9D92A33ffe0123")).toBe("9d92a33");
    expect(normalizeCommit(" 9d92a33 ")).toBe("9d92a33");
  });

  it("rejects what is not a commit id", () => {
    for (const bad of ["", "9d92a3", "xyz1234", "a".repeat(41), undefined]) {
      expect(normalizeCommit(bad), String(bad)).toBe("");
    }
  });
});

describe("formatAppVersion", () => {
  it("shows the version, the build number and the commit", () => {
    expect(formatAppVersion("0.1.3", "412", "9d92a33")).toBe("v0.1.3 · build 412 · 9d92a33");
  });

  it("says what is missing instead of leaving it out", () => {
    expect(formatAppVersion("0.1.3", "", "9d92a33")).toBe("v0.1.3 · build unknown · 9d92a33");
    expect(formatAppVersion("0.1.3", "412", "")).toBe("v0.1.3 · build 412 · commit unknown");
    expect(formatAppVersion("0.1.3", "", "")).toBe("v0.1.3 · build unknown · commit unknown");
    expect(formatAppVersion("", "412", "9d92a33")).toBe("version unknown · build 412 · 9d92a33");
    expect(formatAppVersion("", "", "")).toBe("version unknown · build unknown · commit unknown");
  });

  it("ignores stray whitespace", () => {
    expect(formatAppVersion(" 0.1.3 ", " 412 ", "  9d92a33")).toBe("v0.1.3 · build 412 · 9d92a33");
    expect(formatAppVersion("0.1.3", "  ", "  ")).toBe("v0.1.3 · build unknown · commit unknown");
  });
});

describe("the version bar is on every page", () => {
  it("is rendered by the root route, above every route including login and not-found", () => {
    expect(read("../src/routes/__root.tsx")).toMatch(/<VersionBar \/>/);
  });

  it("leaves room for it: the fixed app shell starts below the bar, and the root pads the page", () => {
    expect(read("../src/components/app-shell.tsx")).toContain("top-[var(--version-bar-height)]");
    expect(read("../src/routes/__root.tsx")).toContain("pt-[var(--version-bar-height)]");
    expect(read("../src/styles/app.css")).toMatch(/--version-bar-height:\s*1\.25rem/);
  });
});
