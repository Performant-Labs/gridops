/**
 * The frontend platform registry (#145): one descriptor per CI platform, looked up by id with a
 * safe fallback, each well-formed enough for the Connections page, the Targets row and the
 * Platform options section to render it without platform-specific code.
 */
import type { ReactNode } from "react";
import { describe, expect, expectTypeOf, it } from "vitest";

import { getPlatform, platforms } from "../src/features/platform-connections/registry";
import type { PostCreateReveal } from "../src/features/platform-connections/types";

describe("platform registry lookups", () => {
  it("registers GitHub then Bitbucket, and nothing else", () => {
    expect(platforms.map((platform) => platform.id)).toEqual(["github", "bitbucket"]);
  });

  it("finds each platform by id with its display name", () => {
    expect(getPlatform("github").name).toBe("GitHub");
    expect(getPlatform("bitbucket").name).toBe("Bitbucket");
    expect(getPlatform("bitbucket")).toBe(platforms[1]);
  });

  it("falls back for an unknown id: the raw id as its name, no targets, no throw", () => {
    const unknown = getPlatform("gitlab");
    expect(unknown.id).toBe("gitlab");
    expect(unknown.name).toBe("gitlab");
    expect(unknown.icon).toBeTruthy();
    expect(unknown.targetKinds).toEqual([]);
    expect(platforms.map((platform) => platform.id)).not.toContain("gitlab");
  });
});

describe("every registered descriptor is well-formed", () => {
  const kinds = platforms.flatMap((platform) => platform.targetKinds.map((kind) => ({ platform, kind })));

  it.each(platforms.map((platform) => [platform.id, platform] as const))("%s has a name, an icon, target kinds and total payload partials", (_id, platform) => {
    expect(platform.name.trim()).not.toBe("");
    expect(platform.icon).toBeTruthy();
    expect(platform.targetKinds.length).toBeGreaterThan(0);
    expect(typeof platform.toCreatePayload).toBe("function");
    expect(typeof platform.toUpdatePayload).toBe("function");
  });

  it("namespaces every kind id under its platform, uniquely", () => {
    for (const { platform, kind } of kinds) expect(kind.id.startsWith(`${platform.id}.`)).toBe(true);
    const ids = kinds.map(({ kind }) => kind.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("gives every kind a label, a group heading, a loader and an availability check", () => {
    for (const { kind } of kinds) {
      expect(kind.label.trim()).not.toBe("");
      expect(kind.group.trim()).not.toBe("");
      expect(typeof kind.load).toBe("function");
      expect(typeof kind.available).toBe("function");
      expect(typeof kind.standalone).toBe("boolean");
    }
  });

  it("points requires/excludes only at registered kinds, and a non-standalone kind requires a standalone one", () => {
    const byId = new Map(kinds.map(({ kind }) => [kind.id, kind]));
    for (const { kind } of kinds) {
      for (const id of [...(kind.requires ?? []), ...(kind.excludes ?? [])]) expect(byId.has(id)).toBe(true);
      if (!kind.standalone) {
        expect(kind.requires?.length).toBeGreaterThan(0);
        expect(kind.requires?.every((id) => byId.get(id)?.standalone)).toBe(true);
      }
    }
  });
});

describe("GitHub and Bitbucket target kinds keep today's rules", () => {
  const kind = (id: string) => platforms.flatMap((platform) => platform.targetKinds).find((candidate) => candidate.id === id);

  it("declares the three kinds", () => {
    expect(getPlatform("github").targetKinds.map((candidate) => candidate.id)).toEqual(["github.repository", "github.runner-group"]);
    expect(getPlatform("bitbucket").targetKinds.map((candidate) => candidate.id)).toEqual(["bitbucket.workspace"]);
  });

  it("makes repositories and the organization runner group standalone and mutually exclusive", () => {
    expect(kind("github.repository")).toMatchObject({ standalone: true, cardinality: "many" });
    expect(kind("github.runner-group")).toMatchObject({ standalone: true, cardinality: "one" });
    expect(kind("github.repository")?.excludes).toContain("github.runner-group");
    expect(kind("github.runner-group")?.excludes).toContain("github.repository");
  });

  it("makes a Bitbucket workspace need a GitHub target and the Tart provider", () => {
    const workspace = kind("bitbucket.workspace");
    expect(workspace?.standalone).toBe(false);
    expect([...(workspace?.requires ?? [])].sort()).toEqual(["github.repository", "github.runner-group"]);
    expect(workspace?.excludes ?? []).toEqual([]);
    expect(workspace?.requiresProviders).toEqual(["tart"]);
  });

  it("checks Bitbucket labels with the schema's rule, and leaves GitHub labels free-form", () => {
    const bitbucket = getPlatform("bitbucket").labelRules?.validate;
    expect(bitbucket?.(["build", "release.ios"])).toBeNull();
    expect(bitbucket?.(["build", "Release_iOS"])).toEqual(expect.any(String));
    expect(getPlatform("github").labelRules?.validate?.(["Release_iOS"]) ?? null).toBeNull();
  });
});

describe("post-create reveal contract", () => {
  it("PostCreateReveal has exactly the brief's shape (checked by npm run typecheck)", () => {
    expectTypeOf<PostCreateReveal>().toEqualTypeOf<{ title: string; description: string; secretLabel: string; value: string; details?: ReactNode; closeButtonText?: string }>();
  });
});
