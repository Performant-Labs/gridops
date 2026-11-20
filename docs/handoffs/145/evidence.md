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
