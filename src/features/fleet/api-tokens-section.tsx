/** Settings → API tokens: create a token, show its secret once, list and revoke. */
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { LoaderCircle, Plus } from "lucide-react";
import { useReducer, useRef, useState, type ReactNode } from "react";
import { toast } from "sonner";

import { confirmAction } from "~/components/confirm-dialog";
import { CopyButton } from "~/components/copy-button";
import { Callout, InlineError, LoadingRows } from "~/components/page";
import { SettingsRow, SettingsSection, Switch, UnitField } from "~/components/settings-ui";
import { Badge } from "~/components/ui/badge";
import { Button } from "~/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "~/components/ui/dialog";
import { Input } from "~/components/ui/input";
import { cn, formatDateTime, formatRelativeTime } from "~/lib/utils";
import { createApiToken, listApiTokens, revokeApiToken } from "./admin-api";
import { API_TOKEN_MAX_DAYS, apiTokenScopes, type ApiToken, type ApiTokenScope } from "./admin-contracts";
import {
  apiTokenState, attemptFor, createFlowReducer, emptyTokenForm, initialCreateFlow, validateTokenForm,
  type RevealedToken, type TokenFormErrors, type TokenFormField, type TokenFormValues,
} from "./api-token-form";
import { fleetErrorMessage, newIdempotencyKey } from "./fleet-ui";

export const apiTokensQueryKey = ["fleet", "api-tokens"] as const;

const scopeDetails: Record<ApiTokenScope, string> = {
  "fleet:read": "Read hosts, backends, grants, events, targets, queues and operations.",
  "fleet:enroll": "Issue, check and revoke enrollment codes.",
  "fleet:approve": "Approve a pending host.",
  "fleet:operate": "Turn runtimes on or off, set lifecycle intent and grants, and save a pool’s fleet profile.",
  "fleet:retire": "Retire a machine that is out of service, silent and idle. Not included in fleet:operate.",
  "pools:read": "Read runner pools, their events and capacity, and the pool form’s options.",
  "pools:manage": "Create, edit and remove runner pools, with the form’s validation. Add pools:read to list them.",
};

export function CreateTokenSection() {
  const queryClient = useQueryClient();
  const [values, setValues] = useState<TokenFormValues>(emptyTokenForm);
  const [errors, setErrors] = useState<TokenFormErrors>({});
  const [flow, dispatch] = useReducer(createFlowReducer, initialCreateFlow);
  // State updates land after the click that caused them; this blocks a second
  // submit fired in the same tick from sending a second request.
  const inFlight = useRef(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const daysRef = useRef<HTMLInputElement>(null);
  const firstScopeRef = useRef<HTMLButtonElement>(null);

  function update(patch: Partial<TokenFormValues>, field: TokenFormField) {
    setValues((current) => ({ ...current, ...patch }));
    if (errors[field]) setErrors((current) => ({ ...current, [field]: undefined }));
  }

  async function submit() {
    if (inFlight.current) return;
    const validation = validateTokenForm(values);
    if (!validation.ok) {
      setErrors(validation.errors);
      // Focus lands on the first problem; its message is the field's description.
      (validation.errors.name ? nameRef : validation.errors.scopes ? firstScopeRef : daysRef).current?.focus();
      return;
    }
    setErrors({});
    const attempt = attemptFor(flow.attempt, validation.request, newIdempotencyKey);
    inFlight.current = true;
    dispatch({ type: "submit", attempt });
    try {
      // Deliberately not a react-query mutation: its cache would keep the plaintext.
      const result = await createApiToken(validation.request, attempt.key);
      dispatch({ type: "settled", result });
      if (result.status === "created") setValues(emptyTokenForm);
    } catch (error) {
      dispatch({ type: "failed", error });
    } finally {
      inFlight.current = false;
      void queryClient.invalidateQueries({ queryKey: apiTokensQueryKey });
    }
  }

  const describedBy = (field: TokenFormField) => (errors[field] ? `api-token-${field}-error` : undefined);

  return (
    <SettingsSection
      description="The secret is shown once, right after you create the token. GridOps keeps only a hash of it."
      title="Create a token"
    >
      <form
        aria-label="Create an API token"
        className="divide-y divide-border"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <SettingsRow description="Say what will use it, for example “ci-onboarding-bot”." htmlFor="api-token-name" label="Name">
          <Input
            aria-describedby={describedBy("name")}
            aria-invalid={errors.name ? true : undefined}
            autoComplete="off"
            id="api-token-name"
            onChange={(event) => update({ name: event.target.value }, "name")}
            ref={nameRef}
            value={values.name}
          />
          {errors.name ? <FieldError id="api-token-name-error">{errors.name}</FieldError> : null}
        </SettingsRow>

        <fieldset aria-describedby={describedBy("scopes")} className="divide-y divide-border">
          <legend className="sr-only">Scopes</legend>
          <div className="px-4 pb-1 pt-3.5">
            <div aria-hidden="true" className="text-sm font-medium text-foreground">Scopes</div>
            <div className="mt-0.5 text-xs leading-5 text-muted-foreground">Grant only what the script needs. Host onboarding needs all four.</div>
            {errors.scopes ? <FieldError id="api-token-scopes-error">{errors.scopes}</FieldError> : null}
          </div>
          {apiTokenScopes.map((scope, index) => (
            <SettingsRow description={scopeDetails[scope]} htmlFor={`api-token-scope-${scope}`} inline key={scope} label={<span className="font-mono text-xs">{scope}</span>}>
              <Switch
                checked={values.scopes.includes(scope)}
                id={`api-token-scope-${scope}`}
                onCheckedChange={(checked) => update({
                  scopes: checked ? [...values.scopes, scope] : values.scopes.filter((item) => item !== scope),
                }, "scopes")}
                ref={index === 0 ? firstScopeRef : undefined}
              />
            </SettingsRow>
          ))}
        </fieldset>

        <SettingsRow description={`Up to ${API_TOKEN_MAX_DAYS} days. Revoke it sooner if you no longer need it.`} htmlFor="api-token-days" label="Expires after">
          <UnitField unit="days">
            <Input
              aria-describedby={describedBy("days")}
              aria-invalid={errors.days ? true : undefined}
              className="pr-12"
              id="api-token-days"
              inputMode="numeric"
              max={API_TOKEN_MAX_DAYS}
              min={1}
              onChange={(event) => update({ days: event.target.value }, "days")}
              ref={daysRef}
              type="number"
              value={values.days}
            />
          </UnitField>
          {errors.days ? <FieldError id="api-token-days-error">{errors.days}</FieldError> : null}
        </SettingsRow>

        <div className="flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-end">
          {flow.error ? (
            <Callout
              action={<Button onClick={() => dispatch({ type: "dismissError" })} size="sm" variant="ghost">Dismiss</Button>}
              className="sm:flex-1"
              title="No token created"
              tone="danger"
            >
              {flow.error}
            </Callout>
          ) : null}
          {/* aria-disabled rather than disabled keeps focus on the button while the request runs. */}
          <Button aria-disabled={flow.pending || undefined} className="self-end aria-disabled:cursor-progress aria-disabled:opacity-60 sm:self-auto" type="submit">
            {flow.pending ? <LoaderCircle className="animate-spin" /> : <Plus />}
            {flow.pending ? "Creating…" : "Create token"}
          </Button>
        </div>
      </form>

      <SecretDialog
        onClose={() => {
          dispatch({ type: "closeSecret" });
        }}
        onClosed={() => nameRef.current?.focus()}
        revealed={flow.revealed}
      />
    </SettingsSection>
  );
}

function FieldError({ id, children }: { id: string; children: ReactNode }) {
  return <p className="mt-1.5 text-left text-xs leading-5 text-danger" id={id}>{children}</p>;
}

/**
 * Shows the plaintext once. Escape and clicks outside are ignored so the only
 * way out is the explicit button, which also drops the secret from state.
 */
function SecretDialog({ revealed, onClose, onClosed }: { revealed: RevealedToken | null; onClose: () => void; onClosed: () => void }) {
  return (
    <Dialog open={revealed !== null}>
      <DialogContent
        // No exit animation: the secret leaves the DOM the moment it leaves state.
        className="top-[16vh] max-w-[520px]"
        instant
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          onClosed();
        }}
        onEscapeKeyDown={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
      >
        {revealed ? (
          <div className="space-y-4 p-5">
            <div>
              <DialogTitle className="text-base font-medium leading-6 text-foreground">Copy your new token</DialogTitle>
              <DialogDescription className="mt-1.5 text-sm leading-5 text-muted-foreground">
                This is the only time GridOps shows the secret for “{revealed.token.name}”. Store it now, for example as a masked CI secret. Once you close this dialog it can’t be shown again.
              </DialogDescription>
            </div>
            <div className="flex items-center gap-2 rounded-lg border border-border-strong bg-panel-subtle px-3 py-2">
              <code aria-label="API token secret" className="min-w-0 flex-1 break-all font-mono text-xs text-foreground">{revealed.secret}</code>
              <CopyButton label="API token" size="sm" value={revealed.secret} variant="outline">Copy</CopyButton>
            </div>
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
              <dt className="text-muted-foreground">Scopes</dt>
              <dd className="font-mono text-foreground">{revealed.token.scopes.join(", ")}</dd>
              <dt className="text-muted-foreground">Expires</dt>
              <dd className="text-foreground">{formatDateTime(revealed.token.expiresAt)}</dd>
            </dl>
            <div className="flex justify-end">
              <Button onClick={onClose}>I’ve stored it, close</Button>
            </div>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

export function TokenListSection() {
  const tokens = useQuery({ queryKey: apiTokensQueryKey, queryFn: ({ signal }) => listApiTokens(signal) });
  const items = tokens.data?.items ?? [];
  const active = items.filter((token) => apiTokenState(token) === "active").length;

  return (
    <SettingsSection
      description="Your tokens, newest first. Revoking takes effect immediately."
      title={tokens.data ? `${active} active ${active === 1 ? "token" : "tokens"}` : "Your tokens"}
    >
      {tokens.isPending ? <LoadingRows rows={3} /> : tokens.isError ? (
        <div className="px-4 py-3">
          <InlineError className="mt-0" onRetry={() => void tokens.refetch()} title="Couldn’t load tokens">{fleetErrorMessage(tokens.error, "")}</InlineError>
        </div>
      ) : items.length ? items.map((token) => <TokenRow key={token.id} token={token} />) : (
        <div className="px-4 py-6 text-center text-sm text-muted-foreground">No API tokens yet.</div>
      )}
    </SettingsSection>
  );
}

const stateBadge = {
  active: <Badge variant="success">Active</Badge>,
  expired: <Badge variant="warning">Expired</Badge>,
  revoked: <Badge variant="outline">Revoked</Badge>,
} as const;

function TokenRow({ token }: { token: ApiToken }) {
  const queryClient = useQueryClient();
  const [pending, setPending] = useState(false);
  const rowRef = useRef<HTMLDivElement>(null);
  const state = apiTokenState(token);

  async function revoke() {
    if (pending) return;
    const confirmed = await confirmAction(
      `Revoke “${token.name}”? Anything using this token loses access immediately. This can’t be undone.`,
      { confirmLabel: "Revoke", destructive: true },
    );
    if (!confirmed) return;
    setPending(true);
    try {
      const { token: revoked } = await revokeApiToken(token.id);
      queryClient.setQueryData(apiTokensQueryKey, (current: { items: ApiToken[] } | undefined) => current
        ? { items: current.items.map((item) => (item.id === revoked.id ? revoked : item)) }
        : current);
      toast.success(`Revoked “${token.name}”.`);
      // The button disappears with the active state; keep focus on the row.
      rowRef.current?.focus();
    } catch (error) {
      toast.error(fleetErrorMessage(error, "The token could not be revoked."));
    } finally {
      setPending(false);
      void queryClient.invalidateQueries({ queryKey: apiTokensQueryKey });
    }
  }

  return (
    <div aria-label={`${token.name}, ${state}`} className="flex items-start gap-3 px-4 py-3 outline-none focus-visible:bg-hover" ref={rowRef} role="group" tabIndex={-1}>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className={cn("truncate text-sm font-medium", state === "revoked" ? "text-muted-foreground" : "text-foreground")}>{token.name}</span>
          {stateBadge[state]}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-1.5">
          <code className="font-mono text-2xs text-muted-foreground" title="The first characters of the token, for recognising it">gat_{token.displayPrefix}…</code>
          {token.scopes.map((scope) => <Badge className="font-mono" key={scope} variant="outline">{scope}</Badge>)}
        </div>
        <div className="mt-1 text-xs text-muted-foreground">
          Created {formatDateTime(token.createdAt)}
          {" · "}
          {token.lastUsedAt ? `last used ${formatRelativeTime(token.lastUsedAt)}` : "never used"}
          {" · "}
          {token.revokedAt ? `revoked ${formatDateTime(token.revokedAt)}` : `${state === "expired" ? "expired" : "expires"} ${formatDateTime(token.expiresAt)}`}
        </div>
      </div>
      {state === "revoked" ? null : (
        <Button aria-label={`Revoke ${token.name}`} disabled={pending} onClick={() => void revoke()} size="sm" variant="outline">
          {pending ? <LoaderCircle className="animate-spin" /> : null}
          Revoke
        </Button>
      )}
    </div>
  );
}
