/**
 * Same-origin browser calls for fleet administration. Like the registry calls,
 * mutations never retry automatically and failures keep only the safe envelope.
 */
import { z } from "zod";
import { fleetErrorSchema, fleetIdSchema } from "./contracts";
import { FleetRequestError } from "./registry-api";
import {
  backendEnabledRequestSchema, fleetPoolIdSchema, grantPolicySchema, hostGrantListSchema, fleetTargetListSchema, lifecycleRequestSchema,
  profileInputSchema, profileResultSchema, savedProfileResultSchema,
  type GrantPolicy, type HostLifecycle, type ProfileInput, poolQueueSchema, agentLogListSchema } from "./admin-contracts";

type Options = { signal?: AbortSignal; body?: unknown; method?: "GET" | "POST" | "PUT" };

async function request<S extends z.ZodType>(path: string, schema: S, options: Options = {}): Promise<z.output<S>> {
  const headers = new Headers({ Accept: "application/json" });
  if (options.body !== undefined) headers.set("Content-Type", "application/json");
  const response = await fetch(path, {
    method: options.method ?? "GET", credentials: "same-origin", cache: "no-store",
    redirect: "error", headers, signal: options.signal,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });
  if (!response.ok) {
    const parsed = fleetErrorSchema.safeParse(await response.json().catch(() => null));
    throw new FleetRequestError(response.status, parsed.success ? parsed.data : undefined);
  }
  const body = schema instanceof z.ZodUndefined ? undefined : await response.json().catch(() => null);
  const parsed = schema.safeParse(body);
  if (!parsed.success) throw new FleetRequestError(response.status);
  return parsed.data;
}

const emptyResponse = z.undefined();
const hostPath = (id: string) => `/api/v1/fleet/hosts/${fleetIdSchema.parse(id)}`;
const profilePath = (poolId: string) => `/api/v1/fleet/pools/${encodeURIComponent(fleetPoolIdSchema.parse(poolId))}/profile`;

export const setHostLifecycle = (hostId: string, expectedRevision: number, lifecycle: HostLifecycle, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/lifecycle`, emptyResponse, {
    method: "POST", body: lifecycleRequestSchema.parse({ expectedRevision, lifecycle }), signal,
  });
export const listAgentLogs = (hostId: string, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/logs`, agentLogListSchema, { signal });
export const retireHost = (hostId: string, expectedRevision: number, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/retire`, emptyResponse, {
    method: "POST", body: { expectedRevision }, signal,
  });
export const setBackendEnabled = (hostId: string, backendId: string, enabled: boolean, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/backends/${fleetIdSchema.parse(backendId)}/enabled`, emptyResponse, {
    method: "POST", body: backendEnabledRequestSchema.parse({ enabled }), signal,
  });
export const listHostGrants = (hostId: string, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/grants`, hostGrantListSchema, { signal });
export const setHostGrant = (hostId: string, targetId: string, policy: GrantPolicy, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/grants/${fleetIdSchema.parse(targetId)}`, emptyResponse, {
    method: "POST", body: grantPolicySchema.parse(policy), signal,
  });
export const listTargets = (signal?: AbortSignal) =>
  request("/api/v1/fleet/targets", fleetTargetListSchema, { signal });
export const getPoolQueue = (poolId: string, signal?: AbortSignal) =>
  request(`${profilePath(poolId).replace(/\/profile$/, "")}/queue`, poolQueueSchema, { signal });
export const getPoolProfile = (poolId: string, signal?: AbortSignal) =>
  request(profilePath(poolId), profileResultSchema, { signal });
export const savePoolProfile = (poolId: string, input: ProfileInput, signal?: AbortSignal) =>
  request(profilePath(poolId), savedProfileResultSchema, {
    method: "PUT", body: profileInputSchema.parse(input), signal,
  });
