import { afterEach, describe, expect, it, vi } from "vitest";

import { setHostBudget } from "../src/features/fleet/admin-api";
import { budgetDraft, budgetRequest } from "../src/features/fleet/machine-budget";

const hostId = "00000001-0000-4000-8000-000000000001";
const GIB = 1024 ** 3;

afterEach(() => vi.unstubAllGlobals());

describe("setHostBudget", () => {
  it("PUTs only the figures given and returns the budget in force", async () => {
    const budget = { cpuMillis: 1500, memoryMib: 3072, diskBytes: 0 };
    const fetcher = vi.fn().mockResolvedValue(Response.json({ budget }));
    vi.stubGlobal("fetch", fetcher);
    await expect(setHostBudget(hostId, { cpuMillis: 1500, memoryMib: 3072 })).resolves.toEqual({ budget });
    const [path, init] = fetcher.mock.calls[0] as [string, RequestInit];
    expect(path).toBe(`/api/v1/fleet/hosts/${hostId}/budget`);
    expect(init).toMatchObject({ method: "PUT", credentials: "same-origin", cache: "no-store", redirect: "error" });
    expect(JSON.parse(String(init.body))).toEqual({ cpuMillis: 1500, memoryMib: 3072 });
  });

  it("refuses an empty, zero or fractional request before sending anything", () => {
    const fetcher = vi.fn();
    vi.stubGlobal("fetch", fetcher);
    expect(() => setHostBudget(hostId, {})).toThrow();
    expect(() => setHostBudget(hostId, { cpuMillis: 0 })).toThrow();
    expect(() => setHostBudget(hostId, { memoryMib: 1.5 })).toThrow();
    expect(() => setHostBudget("../secrets", { cpuMillis: 1000 })).toThrow();
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("surfaces the server's refusal as a fleet error", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json(
      { code: "invalid_request", message: "Invalid fleet request", requestId: hostId, details: null }, { status: 400 })));
    await expect(setHostBudget(hostId, { cpuMillis: 99_000 })).rejects.toMatchObject({ status: 400, code: "invalid_request" });
  });
});

describe("the machine budget form", () => {
  const current = { cpuMillis: 6400, memoryMib: 13_104, diskBytes: 100 * GIB };
  const reported = { cpuMillis: 8000, memoryMib: 16_384, diskBytes: 200 * GIB };

  it("shows the stored budget in cores and GB", () => {
    expect(budgetDraft(current)).toEqual({ cores: "6.4", memoryGb: "12.8", diskGb: "100" });
  });

  it("sends only the figures the operator changed", () => {
    const draft = { ...budgetDraft(current), cores: "4" };
    expect(budgetRequest(current, draft, reported)).toEqual({ ok: true, request: { cpuMillis: 4000 } });
    const both = { ...draft, memoryGb: "8" };
    expect(budgetRequest(current, both, reported)).toEqual({ ok: true, request: { cpuMillis: 4000, memoryMib: 8192 } });
  });

  it("does not turn display rounding into a change", () => {
    const odd = { cpuMillis: 6400, memoryMib: 13_105, diskBytes: 100 * GIB };
    expect(budgetRequest(odd, budgetDraft(odd), reported)).toEqual({ ok: false, error: "Nothing to save." });
  });

  it("allows exactly what the machine reports and no more", () => {
    expect(budgetRequest(current, { ...budgetDraft(current), cores: "8" }, reported)).toEqual({ ok: true, request: { cpuMillis: 8000 } });
    expect(budgetRequest(current, { ...budgetDraft(current), cores: "8.5" }, reported)).toEqual({
      ok: false, error: "CPU can’t exceed the 8 cores this machine reports.",
    });
    expect(budgetRequest(current, { ...budgetDraft(current), memoryGb: "17" }, reported)).toEqual({
      ok: false, error: "Memory can’t exceed the 16 GB this machine reports.",
    });
    expect(budgetRequest(current, { ...budgetDraft(current), diskGb: "201" }, reported)).toEqual({
      ok: false, error: "Disk can’t exceed the 200 GB this machine reports.",
    });
  });

  it("rejects blanks, zero and negatives", () => {
    for (const cores of ["", "0", "-1", "abc"]) {
      expect(budgetRequest(current, { ...budgetDraft(current), cores }, reported)).toMatchObject({ ok: false });
    }
  });

  it("cannot set a figure the machine has not reported", () => {
    const unreported = { ...reported, diskBytes: null };
    expect(budgetRequest(current, { ...budgetDraft(current), diskGb: "50" }, unreported)).toEqual({
      ok: false, error: "This machine hasn’t reported its disk, so its budget can’t be set yet.",
    });
    // An unchanged, unreported figure is fine.
    expect(budgetRequest(current, { ...budgetDraft(current), cores: "4" }, unreported)).toEqual({ ok: true, request: { cpuMillis: 4000 } });
  });
});
