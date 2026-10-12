/**
 * Same-origin browser transport for the version-one fleet read routes.
 *
 * Reads are non-cacheable, never retried automatically, and preserve caller
 * cancellation. Error parsing accepts only the fleet envelope and never puts a
 * raw response body into an exception.
 */
import { z } from "zod";
import { fleetErrorSchema, fleetIdSchema, type FleetError } from "./contracts";
import { FleetRequestError } from "./registry-api";
import {
  backendQueryPageSchema,
  backendQueryResultSchema,
  domainQueryPageSchema,
  domainQueryResultSchema,
  eventQueryPageSchema,
  fleetCatalogQuerySchema,
  fleetDomainCatalogQuerySchema,
  fleetEventQuerySchema,
  fleetSampleQuerySchema,
  hostQueryPageSchema,
  hostQueryResultSchema,
  sampleQueryPageSchema,
  type FleetCatalogQuery,
  type FleetDomainCatalogQuery,
  type FleetEventQuery,
  type FleetSampleQuery,
} from "./query-contracts";

type QuerySchema<S extends z.ZodType> = S;

async function request<S extends z.ZodType>(path: string, schema: QuerySchema<S>, signal?: AbortSignal): Promise<z.output<S>> {
  const headers = new Headers({ Accept: "application/json" });
  const response = await fetch(path, {
    method: "GET",
    credentials: "same-origin",
    cache: "no-store",
    redirect: "error",
    headers,
    signal,
  });

  if (!response.ok) {
    const body = await readJson(response, signal);
    const parsed = fleetErrorSchema.safeParse(body);
    throw new FleetRequestError(response.status, parsed.success ? parsed.data : undefined);
  }

  const body = await readJson(response, signal);
  const parsed = schema.safeParse(body);
  if (!parsed.success) throw new FleetRequestError(response.status);
  return parsed.data;
}

function isAbortError(error: unknown): boolean {
  return typeof error === "object" && error !== null && "name" in error
    && (error as { name?: unknown }).name === "AbortError";
}

async function readJson(response: Response, signal?: AbortSignal): Promise<unknown> {
  signal?.throwIfAborted();
  try {
    const body = await response.json();
    signal?.throwIfAborted();
    return body;
  } catch (error: unknown) {
    signal?.throwIfAborted();
    if (isAbortError(error)) throw error;
    return null;
  }
}

function resourcePath(id: string): string {
  return fleetIdSchema.parse(id);
}

function queryString<T extends z.ZodType>(schema: T, input: unknown): string {
  const parsed = schema.parse(input);
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
    if (value !== undefined) params.set(key, String(value));
  }
  const encoded = params.toString();
  return encoded.length === 0 ? "" : `?${encoded}`;
}

export const listHosts = (input: FleetCatalogQuery = {}, signal?: AbortSignal) =>
  request(`/api/v1/fleet/hosts${queryString(fleetCatalogQuerySchema, input)}`, hostQueryPageSchema, signal);

export const getHost = (hostId: string, signal?: AbortSignal) =>
  request(`/api/v1/fleet/hosts/${resourcePath(hostId)}`, hostQueryResultSchema, signal);

export const listBackends = (hostId: string, input: FleetCatalogQuery = {}, signal?: AbortSignal) =>
  request(`/api/v1/fleet/hosts/${resourcePath(hostId)}/backends${queryString(fleetCatalogQuerySchema, input)}`, backendQueryPageSchema, signal);

export const getBackend = (hostId: string, backendId: string, signal?: AbortSignal) =>
  request(`/api/v1/fleet/hosts/${resourcePath(hostId)}/backends/${resourcePath(backendId)}`, backendQueryResultSchema, signal);

export const listDomains = (hostId: string, input: FleetDomainCatalogQuery = {}, signal?: AbortSignal) =>
  request(`/api/v1/fleet/hosts/${resourcePath(hostId)}/domains${queryString(fleetDomainCatalogQuerySchema, input)}`, domainQueryPageSchema, signal);

export const getDomain = (hostId: string, domainId: string, signal?: AbortSignal) =>
  request(`/api/v1/fleet/hosts/${resourcePath(hostId)}/domains/${resourcePath(domainId)}`, domainQueryResultSchema, signal);

export const listSamples = (hostId: string, input: FleetSampleQuery, signal?: AbortSignal) =>
  request(`/api/v1/fleet/hosts/${resourcePath(hostId)}/samples${queryString(fleetSampleQuerySchema, input)}`, sampleQueryPageSchema, signal);

export const listHostEvents = (hostId: string, input: FleetEventQuery = {}, signal?: AbortSignal) =>
  request(`/api/v1/fleet/hosts/${resourcePath(hostId)}/events${queryString(fleetEventQuerySchema, input)}`, eventQueryPageSchema, signal);

export type FleetQueryError = FleetError;
