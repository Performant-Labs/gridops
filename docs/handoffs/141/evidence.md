# Evidence - #141 GitLab in the shared connections and targets UI

Source facts in unchanged code that the F diff (and T's tests) rely on.

## F (Feature Implementor) - 2026-11-20

- **Fact:** a runner records its GitLab connection and GitLab runner id, and the connection is set to null when the GitLab connection is deleted, so the runners list's `LEFT JOIN gitlab_connections gc ON gc.id=r.gitlab_connection_id` yields null address and path for a deleted connection or a non-GitLab runner.
  **Source:** `migrations/0047_gitlab.sql:144-145`
  **Verbatim excerpt:**
  > CHECK (runtime IN ('vm','native')), `gitlab_connection_id` text
  > REFERENCES `gitlab_connections`(`id`) ON UPDATE no action ON DELETE set null, `gitlab_runner_id` integer,

- **Fact:** `gitlab_connections` has the `base_url` and `scope_path` columns the runners list selects as `gitlab_base_url` / `gitlab_scope_path`.
  **Source:** `migrations/0047_gitlab.sql:28-31`
  **Verbatim excerpt:**
  > `base_url` text NOT NULL,
  > `scope_kind` text NOT NULL CHECK (`scope_kind` IN ('project','group')),
  > `scope_id` integer NOT NULL CHECK (`scope_id` > 0),
  > `scope_path` text NOT NULL,

- **Fact:** a row action runs through `useAction`, which asks `confirmAction` first when `confirm` is set, reports through `toast.promise` (the error toast is the thrown error's message), and returns without running `after` when the action rejects.
  **Source:** `src/lib/use-action.ts:10-21`
  **Verbatim excerpt:**
  > return useCallback(async (options: { action: () => Promise<unknown>; success: string; confirm?: string; confirmOptions?: ConfirmOptions; after?: () => void | Promise<void> }) => {
  >     if (options.confirm && !(await confirmAction(options.confirm, options.confirmOptions))) return false;
  >     const pending = options.action();
  >     toast.promise(pending, {
  >       loading: "Working…",
  >       success: options.success,
  >       error: (error: unknown) => error instanceof Error ? error.message : "Action failed.",
  >     });
  >     try {
  >       await pending;
  >     } catch {
  >       return false;
  >     }

- **Fact:** with no mounted `ConfirmHost`, `confirmAction` falls back to `window.confirm` (the connection tests stub it).
  **Source:** `src/components/confirm-dialog.tsx:24-28`
  **Verbatim excerpt:**
  > export function confirmAction(message: string, options: ConfirmOptions = {}) {
  >   return new Promise<boolean>((resolve) => {
  >     if (!showRequest) {
  >       resolve(window.confirm(message));
  >       return;

- **Fact:** `api()` rejects with the server's `error` text (so Verify, Rotate and create failures show it verbatim) and resolves to undefined on a 204 (Verify's success).
  **Source:** `src/lib/api.ts:36-39`
  **Verbatim excerpt:**
  > const error = await response.json().catch(() => null) as { error?: string } | null;
  >     throw new ApiError(error?.error ?? `GridOps request failed (${response.status}).`, response.status);
  >   }
  >   if (response.status === 204) return undefined as T;

- **Fact:** `SecretRevealDialog` renders its body, including `details` (where the GitLab webhook URL and its Copy button sit), only while open, so the URL leaves the DOM with the secret.
  **Source:** `src/components/secret-reveal-dialog.tsx:54`
  **Verbatim excerpt:**
  > {open ? (

- **Fact:** exclusion between target kinds is symmetric and the reason shown is the selected kind's `exclusionReason`, so the GitLab kinds' `excludes` list disables GitHub and Bitbucket options with "Can’t combine with GitLab targets" and the reverse direction uses GitHub's own reasons.
  **Source:** `src/features/runner-pools/targets.ts:41-46`
  **Verbatim excerpt:**
  > for (const { kind: other } of targetKinds(descriptors)) {
  >     if (other.id === kind.id || selectedCount(selection, other.id) === 0) continue;
  >     if (other.excludes?.includes(kind.id) || kind.excludes?.includes(other.id)) {
  >       return other.exclusionReason ?? `Can’t combine with ${other.label.toLowerCase()} targets`;
  >     }
  >   }

- **Fact:** the create action sends the zod-parsed input, so the GitLab pool fields reach the server only because they are in `createRunnerPoolSchema`.
  **Source:** `src/features/runner-pools/runner-pools.functions.ts:137-139`
  **Verbatim excerpt:**
  > export function createRunnerPoolAction({ data }: { data: CreateRunnerPoolInput }) {
  >   const input = parseCreateRunnerPoolInput(data);
  >   return api<{ id: string; cannotRun?: CannotRun }>("/api/v1/runner-pools", { method: "POST", body: input });

- **Fact:** Add connection is offered for every platform whose list reports `canManage`; this is why the two `tests/connections-page.test.ts` member tests fail once GitLab is registered, since their GitLab stub always answers `canManage: true`.
  **Source:** `src/features/platform-connections/connections-section.tsx:70-73` and `tests/connections-page.test.ts:83`
  **Verbatim excerpt:**
  > const addable = descriptors.filter((descriptor, index) => {
  >     const state = states[index];
  >     const bindings = descriptor.connections;
  >     return Boolean(bindings.create || bindings.connectLink) && state?.status === "ready" && state.list.canManage;

  > if (method === "GET" && url === "/api/v1/platform-connections/gitlab") return Response.json({ canManage: true, items: [] });
