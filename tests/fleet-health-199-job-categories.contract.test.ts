// @vitest-environment jsdom
/**
 * Contract tests for #199's Settings section (job category rules), written by #197.
 *
 * #199 turns each `it.skip("[contract: #199] …")` into `it("…")` and makes it pass; it changes
 * nothing else here (`Fleet health guard`). `JobCategoriesSection` takes no props; it loads the
 * rules from `GET /api/v1/settings/job-categories` and saves through `src/lib/api.ts`. It marks:
 * - one row per rule, `data-rule-index` = its position, with an `input[name="pattern"]` and
 *   "Move up", "Move down" and "Delete rule" buttons (text or `aria-label`); "Add rule" adds an
 *   empty row at the end; "Save" PUTs `{rules: [{category, field, pattern}]}` in row order;
 * - a 422's per-rule `details` (`ApiError.details`) shown inside the row at that position;
 * - the preview (`POST …/preview` with the rules as edited) refreshed 500 ms after the last edit.
 */
import { createElement } from "react";
import { describe, expect, it, vi } from "vitest";

import { JobCategoriesSection } from "~/features/settings/job-categories-section";

import { all, button, click, one, render, settle, type, mountEachTest } from "./fleet-health-dom";

vi.mock("@tanstack/react-router", async () => {
  const { createElement: element } = await import("react");
  return {
    Link: ({ children, to, ...props }: { children?: unknown; to: string } & Record<string, unknown>) =>
      element("a", { ...props, href: to }, children as never),
  };
});

mountEachTest();

const RULES = "/api/v1/settings/job-categories";
const defaults = [
  { position: 0, category: "test", field: "job_name", pattern: "test|spec|e2e|playwright|vitest|jest|pytest|cargo test|shard" },
  { position: 1, category: "build", field: "job_name", pattern: "build|compile|bundle|image|docker|package" },
  { position: 2, category: "check", field: "job_name", pattern: "lint|clippy|fmt|format|typecheck|tsc|check|audit" },
  { position: 3, category: "deploy", field: "job_name", pattern: "deploy|release|publish|pages" },
];

type Call = { path: string; method: string; body: unknown };

/** A server for the section: the rules, the preview, and a PUT answered by `onSave`. */
function serve(onSave: (body: unknown) => { status: number; body: unknown } = (body) => ({ status: 200, body })) {
  const calls: Call[] = [];
  const fetcher = vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const path = typeof input === "string" ? input : input instanceof URL ? input.pathname : input.url;
    const method = init.method ?? "GET";
    const body = typeof init.body === "string" ? JSON.parse(init.body) : undefined;
    calls.push({ path, method, body });
    if (path === RULES && method === "GET") return Response.json({ rules: defaults });
    if (path === `${RULES}/preview`) {
      const rules = (body as { rules: unknown[] } | undefined)?.rules ?? [];
      return Response.json({ rules: rules.map((_, position) => ({ position, matches: 0 })), other: [] });
    }
    if (path === RULES && method === "PUT") {
      const answer = onSave(body);
      return Response.json(answer.body, { status: answer.status });
    }
    return Response.json({ error: "not found" }, { status: 404 });
  });
  vi.stubGlobal("fetch", fetcher);
  return calls;
}

const rows = () => all("[data-rule-index]");
const pattern = (row: HTMLElement) => one<HTMLInputElement>('input[name="pattern"]', row);
const patterns = () => rows().map((row) => pattern(row).value);
const saved = (calls: Call[]) => calls.filter((call) => call.path === RULES && call.method === "PUT").map((call) => call.body);
const previews = (calls: Call[]) => calls.filter((call) => call.path === `${RULES}/preview`);

async function mount() {
  await render(createElement(JobCategoriesSection));
  await settle();
  expect(rows().map((row) => row.getAttribute("data-rule-index"))).toEqual(["0", "1", "2", "3"]);
}

describe("job category rules", () => {
  it("moves a rule up and down and saves the new order", async () => {
    const calls = serve();
    await mount();
    const second = rows()[1];
    if (!second) throw new Error("no second rule");
    await click(button("Move up", second));
    expect(patterns()).toEqual([defaults[1]?.pattern, defaults[0]?.pattern, defaults[2]?.pattern, defaults[3]?.pattern]);
    const third = rows()[2];
    if (!third) throw new Error("no third rule");
    await click(button("Move down", third));
    await click(button("Save"));
    await settle();
    expect(saved(calls)).toEqual([{
      rules: [defaults[1], defaults[0], defaults[3], defaults[2]].map((rule) => ({
        category: rule?.category, field: rule?.field, pattern: rule?.pattern,
      })),
    }]);
  });

  it("adds an empty rule at the end and deletes one", async () => {
    const calls = serve();
    await mount();
    await click(button("Add rule"));
    expect(rows()).toHaveLength(5);
    expect(patterns()[4]).toBe("");
    const added = rows()[4];
    if (!added) throw new Error("no added rule");
    type(pattern(added), "notify");
    const check = rows()[2];
    if (!check) throw new Error("no third rule");
    await click(button("Delete rule", check));
    expect(patterns()).toEqual([defaults[0]?.pattern, defaults[1]?.pattern, defaults[3]?.pattern, "notify"]);
    await click(button("Save"));
    await settle();
    const body = saved(calls)[0] as { rules: Array<{ pattern: string }> } | undefined;
    expect(body?.rules.map((rule) => rule.pattern)).toEqual([defaults[0]?.pattern, defaults[1]?.pattern, defaults[3]?.pattern, "notify"]);
  });

  it("shows a 422's message on the rule it names", async () => {
    const message = "regex parse error: unclosed group";
    serve(() => ({
      status: 422,
      body: { error: "1 rule has an invalid pattern.", code: "invalid_job_category_rules", details: [{ position: 1, message }] },
    }));
    await mount();
    const second = rows()[1];
    if (!second) throw new Error("no second rule");
    type(pattern(second), "(unclosed");
    await click(button("Save"));
    await settle();
    expect(rows()[1]?.textContent).toContain(message);
    for (const index of [0, 2, 3]) expect(rows()[index]?.textContent ?? "").not.toContain(message);
  });

  it("refreshes the preview once, 500 ms after the last edit", async () => {
    const calls = serve();
    await mount();
    await new Promise((resolve) => setTimeout(resolve, 600));
    await settle();
    const before = previews(calls).length;
    const first = rows()[0];
    if (!first) throw new Error("no first rule");
    type(pattern(first), "te");
    type(pattern(first), "tes");
    type(pattern(first), "test");
    await new Promise((resolve) => setTimeout(resolve, 300));
    await settle();
    expect(previews(calls)).toHaveLength(before);
    await new Promise((resolve) => setTimeout(resolve, 400));
    await settle();
    const after = previews(calls).slice(before);
    expect(after).toHaveLength(1);
    expect((after[0]?.body as { rules: Array<{ pattern: string }> }).rules[0]?.pattern).toBe("test");
  });
});
