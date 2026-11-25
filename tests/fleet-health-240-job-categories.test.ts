// @vitest-environment jsdom
/**
 * #240 for the job category rules in Settings (`JobCategoriesSection`):
 * - item 7: the page checks the server's limits itself, at most 200 rules and at most 1 KiB (UTF-8
 *   bytes) per pattern, naming each problem where it is and sending nothing over them;
 * - item 9: only a 422 pauses the preview as an invalid rule, with each message on its row (a blank
 *   new row is "the pattern is empty", not a compile failure); a server error or a network failure
 *   says the preview is unavailable; and a newer edit aborts the preview in flight.
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
const PREVIEW = `${RULES}/preview`;
const COMPILE_FAILURE = "does not compile";

type Rule = { category: string; field: string; pattern: string };
type Call = { path: string; method: string; body: unknown; signal: AbortSignal | null | undefined };
type Answer = Response | Promise<Response> | "network";

const rulesOf = (count: number): Array<Rule & { position: number }> =>
  Array.from({ length: count }, (_, position) => ({ position, category: "test", field: "job_name", pattern: `job${position}` }));

/** A server with `listed` saved rules whose preview answers `onPreview(rules, signal)`. */
function serve(listed: Array<Rule & { position: number }>, onPreview?: (rules: Rule[], signal: AbortSignal | null | undefined) => Answer) {
  const calls: Call[] = [];
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const path = typeof input === "string" ? input : input instanceof URL ? input.pathname : input.url;
    const method = init.method ?? "GET";
    const body = typeof init.body === "string" ? JSON.parse(init.body) : undefined;
    calls.push({ path, method, body, signal: init.signal });
    if (path === RULES && method === "GET") return Response.json({ rules: listed });
    if (path === PREVIEW) {
      const rules = (body as { rules: Rule[] }).rules;
      const answer = onPreview?.(rules, init.signal) ?? Response.json({ rules: rules.map((_, position) => ({ position, matches: 1 })), other: [] });
      if (answer === "network") throw new TypeError("Failed to fetch");
      return answer;
    }
    if (path === RULES && method === "PUT") return Response.json(body);
    return Response.json({ error: "not found" }, { status: 404 });
  }));
  return calls;
}

const rows = () => all("[data-rule-index]");
const pattern = (row: HTMLElement) => one<HTMLInputElement>('input[name="pattern"]', row);
const previews = (calls: Call[]) => calls.filter((call) => call.path === PREVIEW);
const puts = (calls: Call[]) => calls.filter((call) => call.path === RULES && call.method === "PUT");
const text = () => document.body.textContent ?? "";
const debounce = async () => {
  await new Promise((resolve) => setTimeout(resolve, 600));
  await settle();
};

async function mount() {
  await render(createElement(JobCategoriesSection));
  await settle();
}

/** The 422 `parse_rule` answers for empty patterns, as the server words it. */
function emptyPatterns(rules: Rule[]): Response | null {
  const details = rules.flatMap((rule, position) => (rule.pattern === "" ? [{ position, message: "the pattern is empty" }] : []));
  if (details.length === 0) return null;
  return Response.json({ error: "1 rule is invalid.", code: "invalid_job_category_rules", details }, { status: 422 });
}

describe("job category rules: preview failures (#240 item 9)", () => {
  it("shows a blank new row's 422 on that row, not as a compile failure", async () => {
    serve(rulesOf(2), (rules) => emptyPatterns(rules) ?? Response.json({ rules: rules.map((_, position) => ({ position, matches: 1 })), other: [] }));
    await mount();
    await debounce();
    await click(button("Add rule"));
    await debounce();
    expect(rows()[2]?.textContent).toContain("the pattern is empty");
    for (const index of [0, 1]) expect(rows()[index]?.textContent).not.toContain("the pattern is empty");
    expect(text()).toContain("Preview paused");
    expect(text()).not.toContain(COMPILE_FAILURE);

    // Filled in, the row's message goes at once, and so does the paused line that points at it
    // (PR-Agent fbc0441e9fc5); then the preview comes back.
    const added = rows()[2];
    if (!added) throw new Error("no added row");
    type(pattern(added), "notify");
    await settle();
    expect(text()).not.toContain("Preview paused");
    await debounce();
    expect(rows()[2]?.textContent).not.toContain("the pattern is empty");
    expect(text()).not.toContain("Preview paused");
  });

  it("shows a 422's compile error on the rule it names", async () => {
    const message = "regex parse error: unclosed group";
    serve(rulesOf(3), (rules) =>
      rules[1]?.pattern === "(unclosed"
        ? Response.json({ error: "1 rule is invalid.", code: "invalid_job_category_rules", details: [{ position: 1, message }] }, { status: 422 })
        : Response.json({ rules: rules.map((_, position) => ({ position, matches: 1 })), other: [] }));
    await mount();
    const second = rows()[1];
    if (!second) throw new Error("no second rule");
    type(pattern(second), "(unclosed");
    await debounce();
    expect(rows()[1]?.textContent).toContain(message);
    for (const index of [0, 2]) expect(rows()[index]?.textContent).not.toContain(message);
    expect(text()).toContain("Preview paused");
  });

  it("does not call a server error or a network failure a compile failure", async () => {
    let answer: "server" | "network" = "server";
    serve(rulesOf(2), () => (answer === "server" ? Response.json({ error: "The database is busy." }, { status: 503 }) : "network"));
    await mount();
    await debounce();
    expect(text()).toContain("Preview unavailable");
    expect(text()).toContain("The database is busy.");
    expect(text()).not.toContain(COMPILE_FAILURE);
    expect(text()).not.toContain("Preview paused");

    answer = "network";
    const first = rows()[0];
    if (!first) throw new Error("no first rule");
    type(pattern(first), "unit");
    await debounce();
    expect(text()).toContain("Preview unavailable");
    expect(text()).not.toContain(COMPILE_FAILURE);
    expect(text()).not.toContain("The database is busy.");
  });

  it("aborts the preview in flight when the rules change", async () => {
    const calls = serve(rulesOf(2), (rules, signal) =>
      rules[0]?.pattern === "slow"
        ? new Promise<Response>((_, reject) => {
            signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
          })
        : Response.json({ rules: rules.map((_, position) => ({ position, matches: 7 })), other: [] }));
    await mount();
    await debounce();
    const first = rows()[0];
    if (!first) throw new Error("no first rule");
    type(pattern(first), "slow");
    await debounce();
    const slow = previews(calls).at(-1);
    expect((slow?.body as { rules: Rule[] }).rules[0]?.pattern).toBe("slow");
    expect(slow?.signal?.aborted).toBe(false);

    type(pattern(first), "fast");
    expect(slow?.signal?.aborted).toBe(true);
    await debounce();
    expect((previews(calls).at(-1)?.body as { rules: Rule[] }).rules[0]?.pattern).toBe("fast");
    expect(text()).not.toContain("Preview unavailable");
    expect(text()).not.toContain("Preview paused");
    expect(rows()[0]?.textContent).toContain("7");
  });
});

describe("job category rules: limits (#240 item 7)", () => {
  it("refuses a pattern over 1 KiB on its row and sends nothing", async () => {
    const calls = serve(rulesOf(3));
    await mount();
    await debounce();
    const before = previews(calls).length;
    const second = rows()[1];
    if (!second) throw new Error("no second rule");
    // 513 two-byte characters: 1,026 bytes, though only 513 characters.
    type(pattern(second), "é".repeat(513));
    await debounce();
    expect(rows()[1]?.textContent).toContain("1024");
    for (const index of [0, 2]) expect(rows()[index]?.textContent).not.toContain("1024");
    expect(previews(calls)).toHaveLength(before);
    await click(button("Save"));
    await settle();
    expect(puts(calls)).toHaveLength(0);

    // 1,024 bytes is allowed.
    type(pattern(second), "a".repeat(1024));
    await debounce();
    expect(rows()[1]?.textContent).not.toContain("1024");
    expect(previews(calls)).toHaveLength(before + 1);
    await click(button("Save"));
    await settle();
    expect(puts(calls)).toHaveLength(1);
  });

  it("refuses more than 200 rules and adds no 201st", async () => {
    const calls = serve(rulesOf(201));
    await mount();
    await debounce();
    expect(text()).toContain("At most 200 rules");
    expect(previews(calls)).toHaveLength(0);
    await click(button("Save"));
    await settle();
    expect(puts(calls)).toHaveLength(0);

    const last = rows()[200];
    if (!last) throw new Error("no 201st rule");
    await click(button("Delete rule 201", last));
    await debounce();
    expect(text()).not.toContain("At most 200 rules");
    expect(previews(calls)).toHaveLength(1);
    expect(button("Add rule").disabled).toBe(true);
    await click(button("Save"));
    await settle();
    expect(puts(calls)).toHaveLength(1);
  });
});
