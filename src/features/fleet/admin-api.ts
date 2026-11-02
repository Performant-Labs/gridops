/**
 * Same-origin browser calls for fleet administration. Like the registry calls,
 * mutations never retry automatically and failures keep only the safe envelope.
 */
import { z } from "zod";
import { fleetErrorSchema, fleetIdSchema } from "./contracts";
import { FleetRequestError } from "./registry-api";
import { fleetIdempotencyKeySchema } from "./registry-contracts";
import {
  backendEnabledRequestSchema, budgetRequestSchema, deletedHostSchema, budgetResultSchema, type BudgetRequest, fleetPoolIdSchema, grantPolicySchema, hostGrantListSchema, fleetTargetListSchema, lifecycleRequestSchema,
  profileInputSchema, profileResultSchema, savedProfileResultSchema,
  type GrantPolicy, type HostLifecycle, type ProfileInput, type WorkloadWindow, fleetWorkloadSchema, poolQueueSchema, agentLogListSchema, repairResultSchema, unconfirmedListSchema,
  apiTokenListSchema, apiTokenResultSchema, createApiTokenRequestSchema, createApiTokenResultSchema, type CreateApiTokenRequest } from "./admin-contracts";

type Options = { signal?: AbortSignal; body?: unknown; key?: string; method?: "GET" | "POST" | "PUT" | "DELETE" };

async function request<S extends z.ZodType>(path: string, schema: S, options: Options = {}): Promise<z.output<S>> {
  const headers = new Headers({ Accept: "application/json" });
  if (options.body !== undefined) headers.set("Content-Type", "application/json");
  if (options.key !== undefined) headers.set("Idempotency-Key", fleetIdempotencyKeySchema.parse(options.key));
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
/** Sets how much of the machine GridOps may fill; a figure left out is kept. Returns the budget now in force. */
export const setHostBudget = (hostId: string, budget: BudgetRequest, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/budget`, budgetResultSchema, {
    method: "PUT", body: budgetRequestSchema.parse(budget), signal,
  });
export const listAgentLogs = (hostId: string, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/logs`, agentLogListSchema, { signal });
export const retireHost = (hostId: string, expectedRevision: number, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/retire`, emptyResponse, {
    method: "POST", body: { expectedRevision }, signal,
  });
/**
 * Permanently deletes a machine that is retired or never finished enrolling.
 * A 409 `host_not_deletable` carries every reason in `details.reasons`.
 */
export const deleteHost = (hostId: string, signal?: AbortSignal) =>
  request(hostPath(hostId), deletedHostSchema, { method: "DELETE", signal });
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
export const listUnconfirmed = (hostId: string, signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/unconfirmed`, unconfirmedListSchema, { signal });
/** Settles unconfirmed placements the way the agent would; at most 64 per call. */
export const repairUnconfirmed = (hostId: string, placementIds: string[], signal?: AbortSignal) =>
  request(`${hostPath(hostId)}/unconfirmed/repair`, repairResultSchema, {
    method: "POST", body: { placementIds: placementIds.map((id) => fleetIdSchema.parse(id)) }, signal,
  });
export const listTargets = (signal?: AbortSignal) =>
  request("/api/v1/fleet/targets", fleetTargetListSchema, { signal });
export const getFleetWorkload = (window: WorkloadWindow, signal?: AbortSignal) =>
  request(`/api/v1/fleet/workload?window=${window}`, fleetWorkloadSchema, { signal });
export const getPoolQueue = (poolId: string, signal?: AbortSignal) =>
  request(`${profilePath(poolId).replace(/\/profile$/, "")}/queue`, poolQueueSchema, { signal });
export const getPoolProfile = (poolId: string, signal?: AbortSignal) =>
  request(profilePath(poolId), profileResultSchema, { signal });
export const savePoolProfile = (poolId: string, input: ProfileInput, signal?: AbortSignal) =>
  request(profilePath(poolId), savedProfileResultSchema, {
    method: "PUT", body: profileInputSchema.parse(input), signal,
  });

const apiTokensPath = "/api/v1/fleet/api-tokens";
/** Cookie-only, the caller's own tokens, newest first; never a secret. */
export const listApiTokens = (signal?: AbortSignal) =>
  request(apiTokensPath, apiTokenListSchema, { signal });
/**
 * Admin only. Reuse `key` when retrying the same submission: the server then
 * replays the original token's metadata instead of minting a second one.
 * The plaintext comes back once, in the `created` result; never retain it.
 */
export const createApiToken = (input: CreateApiTokenRequest, key: string, signal?: AbortSignal) =>
  request(apiTokensPath, createApiTokenResultSchema, {
    method: "POST", body: createApiTokenRequestSchema.parse(input), key, signal,
  });
/** Idempotent by state: revoking an already revoked token returns it again. */
export const revokeApiToken = (tokenId: string, signal?: AbortSignal) =>
  request(`${apiTokensPath}/${fleetIdSchema.parse(tokenId)}/revoke`, apiTokenResultSchema, { method: "POST", signal });
