/** Verifies fleet administration paths, bodies and response parsing. */
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  getPoolProfile, listHostGrants, listTargets, savePoolProfile, setBackendEnabled, setHostGrant, setHostLifecycle,
} from "../src/features/fleet/admin-api";
import type { ProfileInput } from "../src/features/fleet/admin-contracts";

const hostId = "00000001-0000-4000-8000-000000000001";
const backendId = "00000002-0000-4000-8000-000000000001";
const targetId = "00000008-0000-4000-8000-000000000001";
const profileInput: ProfileInput = {
  enabled: true, placement: "machines", mode: "ephemeral", cpuMillis: 2000, memoryMib: 2048, diskBytes: 0,
  platforms: [{ runtimeKind: "docker", executionOs: "linux", architecture: "arm64", image: "ghcr.io/actions/actions-runner:latest", requiresDockerSocket: false }],
};

const calls = (fetcher: ReturnType<typeof vi.fn>) => fetcher.mock.calls as Array<[string, RequestInit]>;

afterEach(() => vi.unstubAllGlobals());

describe("fleet admin browser transport", () => {
  it("sends empty-response mutations as same-origin JSON without retrying", async () => {
    const fetcher = vi.fn().mockImplementation(() => Promise.resolve(new Response(null, { status: 200 })));
    vi.stubGlobal("fetch", fetcher);
    await expect(setHostLifecycle(hostId, 4, "draining")).resolves.toBeUndefined();
    await expect(setBackendEnabled(hostId, backendId, true)).resolves.toBeUndefined();
    await expect(setHostGrant(hostId, targetId, { allowSchedule: true, allowNative: false, allowDockerSocket: false })).resolves.toBeUndefined();

    const [lifecycle, enabled, grant] = calls(fetcher);
    expect(lifecycle?.[0]).toBe(`/api/v1/fleet/hosts/${hostId}/lifecycle`);
    expect(JSON.parse(String(lifecycle?.[1].body))).toEqual({ expectedRevision: 4, lifecycle: "draining" });
    expect(enabled?.[0]).toBe(`/api/v1/fleet/hosts/${hostId}/backends/${backendId}/enabled`);
    expect(JSON.parse(String(enabled?.[1].body))).toEqual({ enabled: true });
    expect(grant?.[0]).toBe(`/api/v1/fleet/hosts/${hostId}/grants/${targetId}`);
    for (const [, init] of calls(fetcher)) {
      expect(init).toMatchObject({ method: "POST", credentials: "same-origin", cache: "no-store", redirect: "error" });
      expect(new Headers(init.headers).get("Content-Type")).toBe("application/json");
    }

    expect(() => setHostLifecycle(hostId, 0, "active")).toThrow();
    expect(() => setHostLifecycle(hostId, 1, "retired" as never)).toThrow();
    expect(() => setBackendEnabled("../secrets", backendId, true)).toThrow();
    expect(fetcher).toHaveBeenCalledTimes(3);

    fetcher.mockResolvedValue(Response.json({ code: "revision_conflict", message: "Fleet revision has changed",
      requestId: hostId, details: null }, { status: 409 }));
    await expect(setHostLifecycle(hostId, 4, "active")).rejects.toMatchObject({ status: 409, code: "revision_conflict" });
    expect(fetcher).toHaveBeenCalledTimes(4);
  });

  it("reads targets, grants and pool profiles and saves a profile with PUT", async () => {
    const profile = { ...profileInput, platforms: profileInput.platforms.map((platform) => ({ ...platform, id: targetId, revision: 2 })), targetIds: [targetId] };
    const fetcher = vi.fn()
      .mockResolvedValueOnce(Response.json({ items: [{ id: targetId, kind: "github_repository", label: "acme/api" }] }))
      .mockResolvedValueOnce(Response.json({ items: [{ targetId, label: "acme/api", allowSchedule: true, allowNative: false, allowDockerSocket: false }] }))
      .mockResolvedValueOnce(Response.json({ item: null }))
      .mockResolvedValueOnce(Response.json({ item: profile }))
      .mockResolvedValueOnce(Response.json({ item: { ...profile, secret: "x" } }));
    vi.stubGlobal("fetch", fetcher);

    await expect(listTargets()).resolves.toEqual({ items: [{ id: targetId, kind: "github_repository", label: "acme/api" }] });
    await expect(listHostGrants(hostId)).resolves.toMatchObject({ items: [{ targetId, allowSchedule: true }] });
    await expect(getPoolProfile("fleet-pool")).resolves.toEqual({ item: null });
    await expect(savePoolProfile("fleet-pool", profileInput)).resolves.toEqual({ item: profile });
    await expect(getPoolProfile("fleet-pool")).rejects.toMatchObject({ code: "invalid_response" });

    const sent = calls(fetcher);
    expect(sent.map(([path]) => path)).toEqual([
      "/api/v1/fleet/targets",
      `/api/v1/fleet/hosts/${hostId}/grants`,
      "/api/v1/fleet/pools/fleet-pool/profile",
      "/api/v1/fleet/pools/fleet-pool/profile",
      "/api/v1/fleet/pools/fleet-pool/profile",
    ]);
    expect(sent[3]?.[1]).toMatchObject({ method: "PUT" });
    expect(JSON.parse(String(sent[3]?.[1].body))).toEqual(profileInput);
    expect(() => savePoolProfile("fleet-pool", { ...profileInput, cpuMillis: 0 })).toThrow();
    expect(() => getPoolProfile("../pools")).toThrow();
  });
});
