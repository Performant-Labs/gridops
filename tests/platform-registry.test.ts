/**
 * The frontend platform registry (#145): one descriptor per CI platform, looked up by id with a
 * safe fallback, each well-formed enough for the Connections page, the Targets row and the
 * Platform options section to render it without platform-specific code.
 */
import { readFileSync } from "node:fs";
import type { ReactNode } from "react";
import { describe, expect, expectTypeOf, it } from "vitest";

import type { SettingsPage } from "../src/features/operations/operations.functions";
import { getPlatform, platforms } from "../src/features/platform-connections/registry";
import type { ConnectionBindings, PlatformDescriptor, SecretReveal } from "../src/features/platform-connections/types";

describe("platform registry lookups", () => {
  it("registers GitHub, Bitbucket, then GitLab last (create partials merge in this order), and nothing else", () => {
    expect(platforms.map((platform) => platform.id)).toEqual(["github", "bitbucket", "gitlab"]);
  });

  it("finds each platform by id with its display name", () => {
    expect(getPlatform("github").name).toBe("GitHub");
    expect(getPlatform("bitbucket").name).toBe("Bitbucket");
    expect(getPlatform("bitbucket")).toBe(platforms[1]);
    expect(getPlatform("gitlab").name).toBe("GitLab");
    expect(getPlatform("gitlab")).toBe(platforms[2]);
  });

  it("falls back for an unknown id: the raw id as its name, no targets, no throw", () => {
    const unknown = getPlatform("azure-devops");
    expect(unknown.id).toBe("azure-devops");
    expect(unknown.name).toBe("azure-devops");
    expect(unknown.icon).toBeTruthy();
    expect(unknown.targetKinds).toEqual([]);
    expect(platforms.map((platform) => platform.id)).not.toContain("azure-devops");
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

describe("one-time reveal contract", () => {
  it("SecretReveal (renamed from PostCreateReveal in #141: it also serves rotation) keeps the #145 shape (checked by npm run typecheck)", () => {
    expectTypeOf<SecretReveal>().toEqualTypeOf<{ title: string; description: string; secretLabel: string; value: string; details?: ReactNode; closeButtonText?: string }>();
  });
});

describe("Add connection contract (round-4 W1/W2)", () => {
  it.each(platforms.map((platform) => [platform.id, platform] as const))("%s has a chooser description and no externalLinks field", (_id, platform) => {
    expect(platform.connections.chooserDescription).toEqual(expect.stringMatching(/\S/));
    expect(platform).not.toHaveProperty("externalLinks");
  });

  it("the descriptor types carry chooserDescription, connectLink and a nullable emptyText, and no externalLinks (checked by npm run typecheck)", () => {
    expectTypeOf<PlatformDescriptor>().not.toHaveProperty("externalLinks");
    expectTypeOf<ConnectionBindings["chooserDescription"]>().toEqualTypeOf<string>();
    expectTypeOf<ConnectionBindings["emptyText"]>().toEqualTypeOf<(page: SettingsPage) => string | null>();
    expectTypeOf<NonNullable<ConnectionBindings["connectLink"]>>()
      .toEqualTypeOf<(page: SettingsPage) => { label: string; description: ReactNode; href: string | null; unavailableText?: string }>();
  });

  it("the forward-compat survey names the chooser bindings and no longer names externalLinks", () => {
    const survey = readFileSync(new URL("../docs/handoffs/145/survey.md", import.meta.url), "utf8");
    expect(survey).toContain("chooserDescription");
    expect(survey).toContain("connectLink");
    expect(survey).not.toContain("externalLinks");
  });
});
