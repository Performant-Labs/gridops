import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { formatAppVersion, parseWorkspaceVersion } from "~/lib/build-info";

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

describe("formatAppVersion", () => {
  it("shows the version and the commit", () => {
    expect(formatAppVersion("0.1.3", "9d92a33")).toBe("v0.1.3 · 9d92a33");
  });

  it("shows the version alone when the commit is unknown", () => {
    expect(formatAppVersion("0.1.3", "")).toBe("v0.1.3");
    expect(formatAppVersion(" 0.1.3 ", "  ")).toBe("v0.1.3");
  });

  it("says so when the version is unknown, whatever the commit", () => {
    expect(formatAppVersion("", "")).toBe("version unknown");
    expect(formatAppVersion("", "9d92a33")).toBe("version unknown");
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
