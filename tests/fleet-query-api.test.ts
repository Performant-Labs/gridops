/** Verifies fleet query paths, transport policy, encoding and safe failures. */
import { afterEach, describe, expect, it, vi } from "vitest";
import fixture from "./fixtures/fleet-query-contracts.json";
import {
  FleetRequestError,
} from "../src/features/fleet/registry-api";
import {
  getBackend,
  getDomain,
  getHost,
  listBackends,
  listDomains,
  listHostEvents,
  listHosts,
  listSamples,
} from "../src/features/fleet/query-api";

const hostId = "00000001-0000-4000-8000-000000000001";
const backendId = "00000002-0000-4000-8000-000000000001";
const domainId = "00000003-0000-4000-8000-000000000001";
const targetId = "00000008-0000-4000-8000-000000000001";

const mockFetch = (body: unknown, status = 200) => {
  const fetcher = vi.fn().mockResolvedValue(Response.json(body, { status }));
  vi.stubGlobal("fetch", fetcher);
  return fetcher;
};

const firstCall = (fetcher: ReturnType<typeof vi.fn>) => {
  const call = fetcher.mock.calls[0];
  if (!call) throw new Error("Expected a fleet request");
  return call as [string, RequestInit];
};

afterEach(() => vi.unstubAllGlobals());

describe("fleet query browser transport", () => {
  it("uses same-origin no-store GETs for all eight routes", async () => {
    const signal = new AbortController().signal;
    const sampleWindow = { receivedFrom: "2026-10-09T00:00:00.000Z", receivedTo: "2026-10-09T12:00:00.000Z" };
    const requests: Array<Promise<unknown>> = [];
    const expectedPaths = [
      "/api/v1/fleet/hosts?limit=2&targetId=00000008-0000-4000-8000-000000000001",
      `/api/v1/fleet/hosts/${hostId}`,
      `/api/v1/fleet/hosts/${hostId}/backends?limit=2&cursor=q1.cGF5bG9hZA`,
      `/api/v1/fleet/hosts/${hostId}/backends/${backendId}`,
      `/api/v1/fleet/hosts/${hostId}/domains?limit=2`,
      `/api/v1/fleet/hosts/${hostId}/domains/${domainId}`,
      `/api/v1/fleet/hosts/${hostId}/samples?domainId=${domainId}&receivedFrom=2026-10-09T00%3A00%3A00.000Z&receivedTo=2026-10-09T12%3A00%3A00.000Z`,
      `/api/v1/fleet/hosts/${hostId}/events?targetId=${targetId}&occurredFrom=2026-10-09T00%3A00%3A00.000Z&occurredTo=2026-10-09T12%3A00%3A00.000Z`,
    ];
    const fetcher = mockFetch(fixture.hostPages[0]);
    requests.push(listHosts({ limit: 2, targetId }, signal));
    fetcher.mockResolvedValue(Response.json(fixture.hostResults[0]));
    requests.push(getHost(hostId, signal));
    fetcher.mockResolvedValue(Response.json(fixture.backendPages[0]));
    requests.push(listBackends(hostId, { cursor: "q1.cGF5bG9hZA", limit: 2 }, signal));
    fetcher.mockResolvedValue(Response.json(fixture.backendResults[0]));
    requests.push(getBackend(hostId, backendId, signal));
    fetcher.mockResolvedValue(Response.json(fixture.domainPages[0]));
    requests.push(listDomains(hostId, { limit: 2 }, signal));
    fetcher.mockResolvedValue(Response.json(fixture.domainResults[0]));
    requests.push(getDomain(hostId, domainId, signal));
    fetcher.mockResolvedValue(Response.json(fixture.samplePages[0]));
    requests.push(listSamples(hostId, { ...sampleWindow, domainId }, signal));
    fetcher.mockResolvedValue(Response.json(fixture.eventPages[0]));
    requests.push(listHostEvents(hostId, {
      occurredFrom: sampleWindow.receivedFrom, occurredTo: sampleWindow.receivedTo, targetId,
    }, signal));
    await Promise.all(requests);

    expect(fetcher).toHaveBeenCalledTimes(8);
    for (const [index, call] of fetcher.mock.calls.entries()) {
      expect(call[0]).toBe(expectedPaths[index]);
      expect(call[1]).toMatchObject({
        method: "GET", credentials: "same-origin", cache: "no-store", redirect: "error", signal,
      });
      expect((call[1] as RequestInit).headers).toBeInstanceOf(Headers);
      expect(new Headers((call[1] as RequestInit).headers).get("Accept")).toBe("application/json");
      expect((call[1] as RequestInit).body).toBeUndefined();
    }
  });

  it("does not retry and preserves AbortSignal cancellation", async () => {
    const controller = new AbortController();
    const fetcher = vi.fn().mockRejectedValue(new DOMException("The operation was aborted", "AbortError"));
    vi.stubGlobal("fetch", fetcher);
    const pending = getHost(hostId, controller.signal);
    controller.abort();
    await expect(pending).rejects.toMatchObject({ name: "AbortError" });
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(firstCall(fetcher)[1].signal).toBe(controller.signal);
  });

  it("rejects malformed IDs and bounded query input before fetching", async () => {
    const fetcher = mockFetch(fixture.hostPages[0]);
    expect(() => getHost("../../secrets")).toThrow();
    expect(() => listHosts({ limit: 0 })).toThrow();
    expect(() => listHosts({ limit: 101 })).toThrow();
    expect(() => listHosts({ cursor: "forged cursor" })).toThrow();
    expect(() => listSamples(hostId, {
      receivedFrom: "2026-10-08T00:00:00.000Z", receivedTo: "2026-10-09T00:00:00.001Z",
    })).toThrow();
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("rejects a target filter on domain lists before fetching", () => {
    const fetcher = mockFetch(fixture.domainPages[0]);
    expect(() => listDomains(hostId, { targetId } as never)).toThrow();
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("parses structured status envelopes and discards raw failure bodies", async () => {
    const failure = {
      code: "not_found", message: "Host was not found", requestId: hostId, details: null,
    };
    const fetcher = mockFetch(failure, 404);
    await expect(getHost(hostId)).rejects.toMatchObject({ status: 404, code: "not_found", requestId: hostId });
    expect(fetcher).toHaveBeenCalledTimes(1);

    const secret = "raw-secret-sentinel";
    mockFetch({ secret, message: secret }, 503);
    const rejected = await getHost(hostId).catch((error: unknown) => error);
    expect(rejected).toBeInstanceOf(FleetRequestError);
    expect(JSON.stringify(rejected)).not.toContain(secret);
    expect(String(rejected)).not.toContain(secret);

    mockFetch({ ...fixture.hostResults[0], secret }, 200);
    const invalid = await getHost(hostId).catch((error: unknown) => error);
    expect(invalid).toBeInstanceOf(FleetRequestError);
    expect(JSON.stringify(invalid)).not.toContain(secret);
  });

  it("preserves AbortError from response-body parsing for success and failure", async () => {
    const abort = () => new DOMException("The operation was aborted", "AbortError");
    const successResponse = new Response(null, { status: 200 });
    successResponse.json = vi.fn().mockRejectedValue(abort());
    const successFetch = vi.fn().mockResolvedValue(successResponse);
    vi.stubGlobal("fetch", successFetch);
    await expect(getHost(hostId)).rejects.toMatchObject({ name: "AbortError" });

    const failureResponse = new Response(null, { status: 404 });
    failureResponse.json = vi.fn().mockRejectedValue(abort());
    const failureFetch = vi.fn().mockResolvedValue(failureResponse);
    vi.stubGlobal("fetch", failureFetch);
    await expect(getHost(hostId)).rejects.toMatchObject({ name: "AbortError" });
  });

  it("preserves custom abort reasons while success and error bodies are pending", async () => {
    const cases = [200, 404];
    for (const status of cases) {
      const controller = new AbortController();
      const reason = { kind: "custom-body-cancel", status };
      let rejectBody: ((error: unknown) => void) | undefined;
      const response = new Response(null, { status });
      response.json = vi.fn(() => new Promise<unknown>((_, reject) => { rejectBody = reject; }));
      const fetcher = vi.fn().mockResolvedValue(response);
      vi.stubGlobal("fetch", fetcher);
      const pending = getHost(hostId, controller.signal);
      await Promise.resolve();
      controller.abort(reason);
      rejectBody?.(new Error("body read cancelled"));
      await expect(pending).rejects.toBe(reason);
    }
  });

  it("rejects with the custom reason when a body fulfills after cancellation", async () => {
    const controller = new AbortController();
    const reason = { kind: "fulfilled-after-cancel" };
    let fulfillBody: ((body: unknown) => void) | undefined;
    const response = new Response(null, { status: 200 });
    response.json = vi.fn(() => new Promise<unknown>((resolve) => { fulfillBody = resolve; }));
    const fetcher = vi.fn().mockResolvedValue(response);
    vi.stubGlobal("fetch", fetcher);
    const pending = getHost(hostId, controller.signal);
    await Promise.resolve();
    controller.abort(reason);
    fulfillBody?.(fixture.hostResults[0]);
    await expect(pending).rejects.toBe(reason);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
});
