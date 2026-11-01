/** Verifies the API token transport, its contracts and the create-flow rules. */
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApiToken, listApiTokens, revokeApiToken } from "../src/features/fleet/admin-api";
import {
  apiTokenListSchema, apiTokenResultSchema, createApiTokenRequestSchema, createApiTokenResultSchema,
} from "../src/features/fleet/admin-contracts";
import {
  REPLAYED_MESSAGE, apiTokenState, attemptFor, createErrorMessage, createFlowReducer, emptyTokenForm,
  initialCreateFlow, validateTokenForm, type CreateFlowState, type ValidTokenRequest,
} from "../src/features/fleet/api-token-form";
import type { FleetError } from "../src/features/fleet/contracts";
import { FleetRequestError } from "../src/features/fleet/registry-api";

const tokenId = "00000009-0000-4000-8000-000000000001";
const requestId = "0000000a-0000-4000-8000-000000000001";
const secret = `gat_AbCdEfGh${"x".repeat(34)}A`;
const token = {
  id: tokenId, name: "ci-bot", displayPrefix: "AbCdEfGh", scopes: ["fleet:read", "fleet:enroll"],
  createdAt: "2026-11-01T20:00:00.000Z", expiresAt: "2027-01-30T20:00:00.000Z", lastUsedAt: null, revokedAt: null,
};
const request: ValidTokenRequest = { name: "ci-bot", scopes: ["fleet:read", "fleet:enroll"], expiresInDays: 90 };

const calls = (fetcher: ReturnType<typeof vi.fn>) => fetcher.mock.calls as Array<[string, RequestInit]>;
const fleetError = (status: number, code: string, details: unknown = null) =>
  Response.json({ code, message: "Invalid fleet request", requestId, details }, { status });

afterEach(() => vi.unstubAllGlobals());

describe("API token transport", () => {
  it("creates with POST, a JSON body and the caller's Idempotency-Key, and returns the secret once", async () => {
    const fetcher = vi.fn().mockResolvedValue(Response.json({ ...token, tokenSecret: secret }, { status: 201 }));
    vi.stubGlobal("fetch", fetcher);

    await expect(createApiToken(request, "key-1")).resolves.toEqual({ status: "created", token, secret });
    const [[path, init]] = calls(fetcher) as [[string, RequestInit]];
    expect(path).toBe("/api/v1/fleet/api-tokens");
    expect(init).toMatchObject({ method: "POST", credentials: "same-origin", cache: "no-store", redirect: "error" });
    const headers = new Headers(init.headers);
    expect(headers.get("Idempotency-Key")).toBe("key-1");
    expect(headers.get("Content-Type")).toBe("application/json");
    expect(headers.get("Accept")).toBe("application/json");
    expect(JSON.parse(String(init.body))).toEqual(request);
  });

  it("reports a replay as metadata only, never as a second secret", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ token })));
    await expect(createApiToken(request, "key-1")).resolves.toEqual({ status: "replay", token });
  });

  it("refuses bad input and keys before any request is sent", async () => {
    const fetcher = vi.fn();
    vi.stubGlobal("fetch", fetcher);
    expect(() => createApiToken({ ...request, scopes: [] }, "key")).toThrow();
    expect(() => createApiToken({ ...request, scopes: ["fleet:admin" as never] }, "key")).toThrow();
    expect(() => createApiToken({ ...request, expiresInDays: 366 }, "key")).toThrow();
    expect(() => createApiToken({ ...request, expiresInDays: 0 }, "key")).toThrow();
    expect(() => createApiToken({ ...request, name: "" }, "key")).toThrow();
    expect(() => createApiToken({ ...request, name: "x".repeat(65) }, "key")).toThrow();
    await expect(createApiToken(request, "")).rejects.toThrow();
    await expect(createApiToken(request, "has space")).rejects.toThrow();
    expect(() => revokeApiToken("../hosts")).toThrow();
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("lists with a cookie GET and revokes with an empty POST", async () => {
    const revoked = { ...token, revokedAt: "2026-11-02T20:00:00.000Z" };
    const fetcher = vi.fn()
      .mockResolvedValueOnce(Response.json({ items: [token] }))
      .mockResolvedValueOnce(Response.json({ token: revoked }));
    vi.stubGlobal("fetch", fetcher);

    await expect(listApiTokens()).resolves.toEqual({ items: [token] });
    await expect(revokeApiToken(tokenId.toUpperCase())).resolves.toEqual({ token: revoked });
    const [list, revoke] = calls(fetcher);
    expect(list?.[0]).toBe("/api/v1/fleet/api-tokens");
    expect(list?.[1]).toMatchObject({ method: "GET", credentials: "same-origin" });
    expect(list?.[1].body).toBeUndefined();
    expect(revoke?.[0]).toBe(`/api/v1/fleet/api-tokens/${tokenId}/revoke`);
    expect(revoke?.[1]).toMatchObject({ method: "POST", credentials: "same-origin", redirect: "error" });
    expect(revoke?.[1].body).toBeUndefined();
    for (const [, init] of calls(fetcher)) {
      expect(new Headers(init.headers).get("Idempotency-Key")).toBeNull();
      expect(new Headers(init.headers).get("Authorization")).toBeNull();
    }
  });

  it("maps fleet errors and never retries a failed mutation", async () => {
    const fetcher = vi.fn()
      .mockResolvedValueOnce(fleetError(400, "invalid_request", { kind: "invalid_field", field: "name" }))
      .mockResolvedValueOnce(fleetError(403, "forbidden"))
      .mockResolvedValueOnce(fleetError(409, "idempotency_conflict"))
      .mockResolvedValueOnce(new Response("<html>proxy error</html>", { status: 502 }));
    vi.stubGlobal("fetch", fetcher);

    await expect(createApiToken(request, "k")).rejects.toMatchObject({
      status: 400, code: "invalid_request", requestId, details: { kind: "invalid_field", field: "name" },
    });
    await expect(createApiToken(request, "k")).rejects.toMatchObject({ status: 403, code: "forbidden" });
    await expect(createApiToken(request, "k")).rejects.toMatchObject({ status: 409, code: "idempotency_conflict" });
    await expect(revokeApiToken(tokenId)).rejects.toMatchObject({ status: 502, code: "invalid_response", requestId: undefined });
    expect(fetcher).toHaveBeenCalledTimes(4);
  });

  it("fails closed on a list that carries a secret, without echoing it", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ items: [{ ...token, tokenSecret: secret }] })));
    const failure = await listApiTokens().catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(FleetRequestError);
    expect(failure).toMatchObject({ code: "invalid_response" });
    expect(JSON.stringify(failure)).not.toContain(secret);
    expect(String((failure as Error).message)).not.toContain(secret);
  });
});

describe("API token contracts", () => {
  it("rejects list and revoke items that carry a secret, a verifier or any extra field", () => {
    expect(apiTokenListSchema.safeParse({ items: [token] }).success).toBe(true);
    for (const extra of [{ tokenSecret: secret }, { verifier: "abc" }, { secret }, { userId: "fleet-user" }]) {
      expect(apiTokenListSchema.safeParse({ items: [{ ...token, ...extra }] }).success).toBe(false);
      expect(apiTokenResultSchema.safeParse({ token: { ...token, ...extra } }).success).toBe(false);
    }
    expect(apiTokenListSchema.safeParse({ items: [token], nextCursor: "x" }).success).toBe(false);
  });

  it("rejects malformed metadata", () => {
    for (const delta of [
      { id: "not-a-uuid" }, { name: "" }, { name: "x".repeat(65) }, { name: "a\u0007b" },
      { displayPrefix: "short" }, { displayPrefix: "AbCdEfG!" }, { scopes: [] }, { scopes: ["fleet:read", "fleet:read"] },
      { scopes: ["tokens"] }, { createdAt: "yesterday" }, { expiresAt: "2026-11-01T20:00:00+01:00" },
      { lastUsedAt: undefined }, { revokedAt: 0 },
    ]) expect(apiTokenListSchema.safeParse({ items: [{ ...token, ...delta }] }).success, JSON.stringify(delta)).toBe(false);
    expect(apiTokenListSchema.safeParse({ items: [{ ...token, name: "ü".repeat(64) }] }).success).toBe(true);
  });

  it("accepts a created receipt only with a well-formed secret that matches its prefix", () => {
    expect(createApiTokenResultSchema.safeParse({ ...token, tokenSecret: secret }).success).toBe(true);
    for (const tokenSecret of [secret.replace("gat_", "ghp_"), `${secret.slice(0, -1)}B`, `gat_ZZZZZZZZ${"x".repeat(34)}A`, null]) {
      expect(createApiTokenResultSchema.safeParse({ ...token, tokenSecret }).success).toBe(false);
    }
    expect(createApiTokenResultSchema.safeParse({ token, tokenSecret: secret }).success).toBe(false);
    expect(createApiTokenResultSchema.safeParse({ ...token, tokenSecret: secret, verifier: "v" }).success).toBe(false);
  });

  it("keeps the request to known scopes and a 1–365 day lifetime", () => {
    expect(createApiTokenRequestSchema.safeParse({ name: "a", scopes: ["fleet:operate"] }).success).toBe(true);
    expect(createApiTokenRequestSchema.safeParse({ ...request, expiresInDays: 365 }).success).toBe(true);
    for (const delta of [{ expiresInDays: 1.5 }, { expiresInDays: -1 }, { tokenSecret: secret }, { scopes: ["fleet:read", "fleet:read"] }]) {
      expect(createApiTokenRequestSchema.safeParse({ ...request, ...delta }).success).toBe(false);
    }
  });
});

describe("API token form validation", () => {
  it("trims the name, orders scopes canonically and defaults to 90 days", () => {
    expect(emptyTokenForm.days).toBe("90");
    expect(validateTokenForm({ name: "  ci-bot ", scopes: ["fleet:operate", "fleet:read"], days: "90" })).toEqual({
      ok: true, request: { name: "ci-bot", scopes: ["fleet:read", "fleet:operate"], expiresInDays: 90 },
    });
    expect(validateTokenForm({ name: "x", scopes: ["fleet:read"], days: "365" }).ok).toBe(true);
    expect(validateTokenForm({ name: "x", scopes: ["fleet:read"], days: "1" }).ok).toBe(true);
  });

  it("reports each invalid field", () => {
    expect(validateTokenForm(emptyTokenForm)).toEqual({ ok: false, errors: {
      name: "Enter a name.", scopes: "Choose at least one scope.",
    } });
    for (const days of ["0", "366", "", "1.5", "-3", "1e2", "abc", "9999"]) {
      const result = validateTokenForm({ name: "x", scopes: ["fleet:read"], days });
      expect(result.ok ? null : result.errors.days, days).toBe("Enter a whole number of days from 1 to 365.");
    }
    const tooLong = validateTokenForm({ name: "x".repeat(65), scopes: ["fleet:read"], days: "30" });
    expect(tooLong.ok ? null : tooLong.errors.name).toBe("Use at most 64 characters.");
    const control = validateTokenForm({ name: "a\tb", scopes: ["fleet:read"], days: "30" });
    expect(control.ok ? null : control.errors.name).toMatch(/control characters/);
  });
});

describe("API token create flow", () => {
  const keys = () => { let next = 0; return vi.fn(() => `key-${++next}`); };

  it("reuses the key for a retried identical request and mints a new one when it changes", () => {
    const newKey = keys();
    const first = attemptFor(null, request, newKey);
    expect(first.key).toBe("key-1");
    expect(attemptFor(first, { ...request, scopes: [...request.scopes] }, newKey)).toBe(first);
    expect(attemptFor(first, { ...request, name: "other" }, newKey).key).toBe("key-2");
    expect(attemptFor(first, { ...request, expiresInDays: 30 }, newKey).key).toBe("key-3");
    expect(attemptFor(first, { ...request, scopes: ["fleet:read"] }, newKey).key).toBe("key-4");
  });

  it("keeps the attempt across a network failure so the retry replays instead of duplicating", () => {
    const newKey = keys();
    let state = createFlowReducer(initialCreateFlow, { type: "submit", attempt: attemptFor(null, request, newKey) });
    expect(state.pending).toBe(true);
    // A second click while the first is in flight changes nothing.
    expect(createFlowReducer(state, { type: "submit", attempt: { key: "other", fingerprint: "x" } })).toBe(state);

    state = createFlowReducer(state, { type: "failed", error: new TypeError("Failed to fetch") });
    expect(state).toMatchObject({ pending: false, attempt: { key: "key-1" } });
    expect(state.error).toMatch(/won’t create a second token/);

    const retry = attemptFor(state.attempt, request, newKey);
    expect(retry.key).toBe("key-1");
    expect(newKey).toHaveBeenCalledTimes(1);

    // The lost first response did mint a token: the retry is a replay without a secret.
    state = createFlowReducer(createFlowReducer(state, { type: "submit", attempt: retry }), {
      type: "settled", result: { status: "replay", token: { ...token, scopes: ["fleet:read", "fleet:enroll"] } },
    });
    expect(state).toEqual({ attempt: null, pending: false, error: REPLAYED_MESSAGE, revealed: null });
  });

  it("drops the key only on an idempotency conflict", () => {
    const pending: CreateFlowState = { ...initialCreateFlow, pending: true, attempt: { key: "key-1", fingerprint: "f" } };
    const conflict = new FleetRequestError(409, { code: "idempotency_conflict", message: "Conflict", requestId, details: null });
    expect(createFlowReducer(pending, { type: "failed", error: conflict }).attempt).toBeNull();
    const invalid = new FleetRequestError(400, { code: "invalid_request", message: "Invalid", requestId, details: { kind: "invalid_field", field: "name" } });
    expect(createFlowReducer(pending, { type: "failed", error: invalid })).toMatchObject({
      attempt: { key: "key-1" }, error: expect.stringMatching(/already have an unrevoked token with this name/) as unknown,
    });
  });

  it("holds the plaintext only until the dialog is closed", () => {
    const pending = createFlowReducer(initialCreateFlow, { type: "submit", attempt: { key: "key-1", fingerprint: "f" } });
    const created = createFlowReducer(pending, { type: "settled", result: { status: "created", token: { ...token, scopes: ["fleet:read"] }, secret } });
    expect(created).toMatchObject({ attempt: null, pending: false, error: null, revealed: { secret } });
    const closed = createFlowReducer(created, { type: "closeSecret" });
    expect(closed.revealed).toBeNull();
    expect(JSON.stringify(closed)).not.toContain(secret);
  });

  it("explains server refusals without echoing bodies", () => {
    const make = (status: number, code: FleetError["code"], details: FleetError["details"] = null) =>
      new FleetRequestError(status, { code, message: "Fleet message", requestId, details });
    expect(createErrorMessage(make(400, "invalid_request", { kind: "limit", maximum: 50 }))).toMatch(/50 unrevoked tokens/);
    expect(createErrorMessage(make(400, "invalid_request", { kind: "invalid_field", field: "expiresInDays" }))).toMatch(/1 to 365 days/);
    expect(createErrorMessage(make(403, "forbidden"))).toBe("Only system administrators can create API tokens.");
    expect(createErrorMessage(make(401, "unauthenticated"))).toMatch(/Sign in again/);
    expect(createErrorMessage(make(503, "dependency_unavailable"))).toBe("Fleet message");
    expect(createErrorMessage(new FleetRequestError(502))).toBe("Fleet returned an invalid response.");
  });

  it("derives active, expired and revoked states", () => {
    const now = new Date("2026-12-01T00:00:00.000Z").getTime();
    expect(apiTokenState(token, now)).toBe("active");
    expect(apiTokenState({ ...token, expiresAt: "2026-11-30T00:00:00.000Z" }, now)).toBe("expired");
    expect(apiTokenState({ ...token, expiresAt: "2026-11-30T00:00:00.000Z", revokedAt: "2026-11-02T00:00:00.000Z" }, now)).toBe("revoked");
  });
});
