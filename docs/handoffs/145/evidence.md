# Evidence (#145): unchanged source the change relies on

- **Fact:** `api()` passes the caller's `RequestInit` options, including `signal`, through to `fetch`, so the new optional `signal` on `getBitbucketConnections` and the kinds' `load(context, signal)` abort real requests.
  **Source:** `src/lib/api.ts:29-30`
  **Verbatim excerpt:**
  > const response = await fetch(path, {
  >     ...options,

- **Fact:** a non-2xx response throws an `ApiError` carrying the server's `error` text, which the Connections page shows inline ("Could not connect") and in a per-platform load error with Retry.
  **Source:** `src/lib/api.ts:35-38`
  **Verbatim excerpt:**
  > if (!response.ok) {
  >     const error = await response.json().catch(() => null) as { error?: string } | null;
  >     throw new ApiError(error?.error ?? `GridOps request failed (${response.status}).`, response.status);
  >   }

- **Fact:** `createRunnerPoolAction` parses `data` with the zod schema before calling `api()`, so a selection with only Bitbucket workspaces (installationId 0) fails at submit with no POST, as the brief requires; the Targets picker adds no second validator.
  **Source:** `src/features/runner-pools/runner-pools.functions.ts:127-130`
  **Verbatim excerpt:**
  > export function createRunnerPoolAction({ data }: { data: CreateRunnerPoolInput }) {
  >   const input = parseCreateRunnerPoolInput(data);
  >   return api<{ id: string; cannotRun?: CannotRun }>("/api/v1/runner-pools", { method: "POST", body: input });
  > }

- **Fact:** the GitHub repository kind loads from the same endpoint both old forms used (all installations' repositories in one list), and the runner-group kind uses the same per-installation endpoint.
  **Source:** `src/features/runner-pools/runner-pools.functions.ts:120-125`
  **Verbatim excerpt:**
  > export const getInstallationRunnerGroups = (installationId: number, signal?: AbortSignal) =>
  >   api<{ items: RunnerGroupOption[] }>(`/api/v1/installations/${installationId}/runner-groups`, { signal });
  > export const getInstallationRepositories = (installationId: number, signal?: AbortSignal) =>
  >   api<{ items: RepositoryOption[] }>(`/api/v1/installations/${installationId}/repositories`, { signal });
  > export const getRunnerPoolRepositories = (signal?: AbortSignal) =>
  >   api<{ items: RepositoryOption[] }>("/api/v1/runner-pools/repositories", { signal });

- **Fact:** without a mounted `ConfirmHost`, `confirmAction` (used by `useAction` for Remove) falls back to `window.confirm` with the same message.
  **Source:** `src/components/confirm-dialog.tsx:26-29`
  **Verbatim excerpt:**
  > if (!showRequest) {
  >       resolve(window.confirm(message));
  >       return;
  >     }

- **Fact:** `useAction` refreshes the router after every successful action, so Remove on the Connections page also refreshes the `/settings` data, in addition to the platform query its `after` invalidates.
  **Source:** `src/lib/use-action.ts:24`
  **Verbatim excerpt:**
  > await router.invalidate();

- **Fact:** the app wraps every page in a `TooltipProvider`, which the pool forms and the Connections page rely on (the tests mirror it).
  **Source:** `src/routes/__root.tsx:49`
  **Verbatim excerpt:**
  > <TooltipProvider delayDuration={450} skipDelayDuration={400}>

## Round 4 (F, 2026-11-20)

- **Fact:** the `/settings` page data's `githubApp` is null while the GitHub App is not configured; GitHub's nullable `emptyText` and its `connectLink` (`href: null` plus `unavailableText`) branch on it.
  **Source:** `src/features/operations/operations.functions.ts:144`
  **Verbatim excerpt:**
  > githubApp: null | { slug: string; appUrl: string; installUrl: string };

- **Fact:** a failed fetching list renders `InlineError`, which carries `role="alert"`; the round-4 tests count alerts to tell a failed list apart from the generic empty state.
  **Source:** `src/components/page.tsx:222-224`
  **Verbatim excerpt:**
  > export function InlineError({ title, children, onRetry, className }: { title: string; children?: React.ReactNode; onRetry?: () => void; className?: string }) {
  >   return (
  >     <div className={cn("mt-1.5 flex items-start gap-1.5 text-xs leading-5", className)} role="alert">

- **Fact:** a `warning` `Callout` (used for the connect link's unavailable text) has no `role="alert"`; only `danger` does, so it does not add an alert.
  **Source:** `src/components/page.tsx:210`
  **Verbatim excerpt:**
  > <div className={cn("flex items-start gap-3 rounded-lg border border-border-strong bg-panel-subtle px-3 py-2.5 text-sm", className)} role={tone === "danger" ? "alert" : undefined}>

- **Fact:** `EmptyState` defaults to a 60vh minimum height; the generic Connections empty state overrides it through `className` (`min-h-0 py-10`), which `cn` merges after the defaults.
  **Source:** `src/components/page.tsx:162-163`
  **Verbatim excerpt:**
  > return (
  >     <div className={cn("grid min-h-[60vh] place-items-center px-6 py-16 text-center", className)}>


## F round 5 (S rework: D1, D2 at 360 px)

- **Fact:** besides the GitHub App and Connections headers, the only other `SettingsSection` with `actions` is the fleet-profile section's single Save button, so it is the only other header the wrap change could move.
  **Source:** `src/features/fleet/fleet-profile-section.tsx:231-232`
  **Verbatim excerpt:**
  > <SettingsSection
  >       actions={<Button disabled={saving} form={FORM_ID} size="sm" type="submit">{saving ? <LoaderCircle className="animate-spin" /> : null}Save</Button>}

- **Fact:** settings pages are padded 20 px each side below `sm`, so at 360 px a `SettingsSection` header is 320 px wide (12rem title + 12 px gap leaves about 116 px for actions before they wrap).
  **Source:** `src/components/settings-ui.tsx:11`
  **Verbatim excerpt:**
  > <div className={cn("mx-auto w-full max-w-[720px] px-5 pb-20 pt-10 sm:px-8 sm:pt-14", className)}>

- **Fact:** the Bitbucket row's aside is hidden below `sm`, so on a phone the row's trailing wrapper is present but shows nothing for a member (no Edit/Remove).
  **Source:** `src/features/platform-connections/bitbucket.tsx:70`
  **Verbatim excerpt:**
  > aside: <span className="hidden text-xs text-faint sm:inline">Verified {formatRelativeTime(connection.updatedAt)}</span>,
