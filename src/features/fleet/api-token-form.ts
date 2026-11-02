/**
 * State for the Settings → API tokens create flow, kept free of React so the
 * retry and one-time-secret rules are testable on their own.
 *
 * - One `Idempotency-Key` per submission: a retry of the same request reuses
 *   it, so a double click or a retry after a lost response replays the first
 *   token instead of minting a second; a changed request gets a fresh key.
 * - The plaintext lives only in `revealed`, which `closeSecret` drops.
 */
import { FleetRequestError } from "./registry-api";
import { fleetErrorMessage } from "./fleet-ui";
import {
  API_TOKEN_DEFAULT_DAYS, API_TOKEN_MAX_DAYS, API_TOKEN_MAX_NAME_CHARS, apiTokenScopes,
  type ApiToken, type ApiTokenScope, type CreateApiTokenResult,
} from "./admin-contracts";

export type TokenFormValues = { name: string; scopes: readonly ApiTokenScope[]; days: string };
export type TokenFormField = "name" | "scopes" | "days";
export type TokenFormErrors = Partial<Record<TokenFormField, string>>;
export type ValidTokenRequest = { name: string; scopes: ApiTokenScope[]; expiresInDays: number };

export const emptyTokenForm: TokenFormValues = { name: "", scopes: [], days: String(API_TOKEN_DEFAULT_DAYS) };

/** Mirrors the server's rules so most mistakes never reach the network. */
export function validateTokenForm(values: TokenFormValues):
  | { ok: true; request: ValidTokenRequest }
  | { ok: false; errors: TokenFormErrors } {
  const errors: TokenFormErrors = {};
  const name = values.name.trim();
  if (!name) errors.name = "Enter a name.";
  else if ([...name].length > API_TOKEN_MAX_NAME_CHARS) errors.name = `Use at most ${API_TOKEN_MAX_NAME_CHARS} characters.`;
  else if (/\p{Cc}/u.test(name)) errors.name = "Remove control characters from the name.";

  // Canonical order and no duplicates, so the same choice always fingerprints alike.
  const scopes = apiTokenScopes.filter((scope) => values.scopes.includes(scope));
  if (scopes.length === 0) errors.scopes = "Choose at least one scope.";

  const daysText = values.days.trim();
  const days = /^\d{1,3}$/.test(daysText) ? Number(daysText) : Number.NaN;
  if (!Number.isInteger(days) || days < 1 || days > API_TOKEN_MAX_DAYS) {
    errors.days = `Enter a whole number of days from 1 to ${API_TOKEN_MAX_DAYS}.`;
  }

  if (errors.name || errors.scopes || errors.days) return { ok: false, errors };
  return { ok: true, request: { name, scopes, expiresInDays: days } };
}

/** The key sent with one submission and the request it was minted for. */
export type CreateAttempt = { key: string; fingerprint: string };

const fingerprint = (request: ValidTokenRequest) =>
  JSON.stringify([request.name, request.scopes, request.expiresInDays]);

/** Reuses the previous key only for the identical request; anything else is a new attempt. */
export function attemptFor(previous: CreateAttempt | null, request: ValidTokenRequest, newKey: () => string): CreateAttempt {
  const print = fingerprint(request);
  return previous?.fingerprint === print ? previous : { key: newKey(), fingerprint: print };
}

export type RevealedToken = { token: ApiToken; secret: string };

export type CreateFlowState = {
  /** Kept after a failure so retrying the same request replays rather than duplicates. */
  attempt: CreateAttempt | null;
  pending: boolean;
  error: string | null;
  /** The one-time plaintext, only while its dialog is open. */
  revealed: RevealedToken | null;
};

export type CreateFlowAction =
  | { type: "submit"; attempt: CreateAttempt }
  | { type: "settled"; result: CreateApiTokenResult }
  | { type: "failed"; error: unknown }
  | { type: "closeSecret" }
  | { type: "dismissError" };

export const initialCreateFlow: CreateFlowState = { attempt: null, pending: false, error: null, revealed: null };

export const REPLAYED_MESSAGE =
  "This token was already created by an earlier attempt, so its secret can’t be shown again. Revoke it below and create a new one.";

export function createFlowReducer(state: CreateFlowState, action: CreateFlowAction): CreateFlowState {
  switch (action.type) {
    case "submit":
      // A second submit while one is in flight is ignored, never sent.
      if (state.pending) return state;
      return { ...state, attempt: action.attempt, pending: true, error: null };
    case "settled":
      if (action.result.status === "created") {
        return { attempt: null, pending: false, error: null, revealed: { token: action.result.token, secret: action.result.secret } };
      }
      return { ...state, attempt: null, pending: false, error: REPLAYED_MESSAGE };
    case "failed":
      return {
        ...state,
        // The key is only abandoned when the server says it belongs to a different request.
        attempt: isIdempotencyConflict(action.error) ? null : state.attempt,
        pending: false,
        error: createErrorMessage(action.error),
      };
    case "closeSecret":
      return { ...state, revealed: null };
    case "dismissError":
      return { ...state, error: null };
  }
}

const isIdempotencyConflict = (error: unknown) => error instanceof FleetRequestError && error.code === "idempotency_conflict";

/** Turns a create failure into a sentence; never echoes a response body. */
export function createErrorMessage(error: unknown): string {
  if (error instanceof FleetRequestError) {
    const details = error.details;
    if (details?.kind === "limit") return `You already hold ${details.maximum} unrevoked tokens, the most allowed. Revoke one first.`;
    if (details?.kind === "invalid_field") {
      if (details.field === "name") return "You already have an unrevoked token with this name. Choose another name or revoke that token first.";
      if (details.field === "scopes") return "Choose at least one valid scope.";
      if (details.field === "expiresInDays") return `Choose a lifetime from 1 to ${API_TOKEN_MAX_DAYS} days.`;
    }
    if (error.code === "idempotency_conflict") return "This submission clashed with an earlier one. Create the token again.";
    if (error.code === "unauthenticated") return "Your session has ended. Sign in again, then retry.";
    if (error.status === 403) return "Only system administrators can create API tokens.";
    return fleetErrorMessage(error, "The token could not be created.");
  }
  // fetch rejects with a TypeError when the network fails; the key is kept for the retry.
  if (error instanceof TypeError) return "Couldn’t reach GridOps. Try again: retrying won’t create a second token.";
  return "The token could not be created. Try again.";
}

export type ApiTokenState = "active" | "expired" | "revoked";

export function apiTokenState(token: Pick<ApiToken, "expiresAt" | "revokedAt">, now = Date.now()): ApiTokenState {
  if (token.revokedAt !== null) return "revoked";
  return new Date(token.expiresAt).getTime() <= now ? "expired" : "active";
}
