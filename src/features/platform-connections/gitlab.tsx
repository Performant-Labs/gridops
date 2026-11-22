/**
 * The GitLab platform descriptor (#141): connected GitLab projects and groups, each one connection
 * with its own webhook. A GitLab pool targets connections only (no GitHub or Bitbucket target).
 */
import { Gitlab } from "lucide-react";
import type { ReactNode } from "react";

import { CopyButton } from "~/components/copy-button";
import { PropertyRow } from "~/components/page";
import { Switch } from "~/components/settings-ui";
import { Badge } from "~/components/ui/badge";
import type { Runner, RunnerPool } from "~/features/operations/operations.functions";
import {
  type GitlabPoolConnection,
  type RunnerPoolDetail,
} from "~/features/runner-pools/runner-pools.functions";
import { gitlabLabelError } from "~/features/runner-pools/schemas";
import { formatRelativeTime } from "~/lib/utils";

import {
  type GitlabConnection,
  type GitlabConnectionCreated,
  type GitlabWebhookSecret,
  createGitlabConnection,
  getGitlabConnections,
  removeGitlabConnection,
  rotateGitlabWebhookSecret,
  updateGitlabConnection,
  verifyGitlabConnection,
} from "./platform-connections.functions";
import type {
  PlatformDescriptor,
  PlatformOptionsProps,
  PoolDestinationView,
  ResolvedSelection,
  RunnerDestinationView,
  SecretReveal,
  TargetCreateValues,
  TargetKind,
  TargetOption,
  TargetUpdateValues,
} from "./types";

const GITLAB = "gitlab";
const PROJECT = "gitlab.project";
const GROUP = "gitlab.group";
const DEFAULT_ADDRESS = "https://gitlab.com";
const SCOPE_KINDS = [{ value: "project", label: "Project" }, { value: "group", label: "Group" }] as const;
/** A GitLab pool registers runners on GitLab only: none of these can share it. */
const OTHER_PLATFORM_KINDS = ["github.repository", "github.runner-group", "bitbucket.workspace"];

type ScopeKind = GitlabPoolConnection["scopeKind"];

// ---- Connection form ---------------------------------------------------------------------------

/** The address as sent: trimmed, with no trailing slash; blank means gitlab.com. */
function normalizeAddress(value: string | undefined) {
  const address = (value ?? "").trim().replace(/\/+$/, "");
  return address || DEFAULT_ADDRESS;
}

/** An http(s) address with no query or fragment; blank is allowed (it sends the default). */
function gitlabAddressError(value: string): string | null {
  const address = value.trim();
  if (!address) return null;
  const problem = "Enter a full address starting with http:// or https://, with no query or fragment.";
  if (address.includes("?") || address.includes("#") || /\s/.test(address)) return problem;
  try {
    const url = new URL(address);
    return (url.protocol === "http:" || url.protocol === "https:") && url.hostname ? null : problem;
  } catch {
    return problem;
  }
}

/** `segment(/segment)*`: no leading or trailing slash, no whitespace, no empty or ".." part. */
function gitlabScopePathError(value: string): string | null {
  const segments = value.split("/");
  return value && segments.every((segment) => segment && segment !== ".." && !/\s/.test(segment))
    ? null
    : "Use a path like group/project: no leading or trailing slash, no spaces, no empty or “..” parts.";
}

/** Where a connection's webhook goes, with the setting GitLab needs, shown once beside its secret. */
function WebhookDetails({ webhookUrl, children }: { webhookUrl: string; children: ReactNode }) {
  return (
    <div className="space-y-3">
      <div>
        <div className="mb-1.5 text-xs font-medium text-foreground">Webhook URL</div>
        <div className="flex items-center gap-2 rounded-lg border border-border-strong bg-panel-subtle px-3 py-2">
          <code aria-label="Webhook URL" className="min-w-0 flex-1 break-all font-mono text-xs text-foreground">{webhookUrl}</code>
          <CopyButton label="Webhook URL" size="sm" value={webhookUrl} variant="outline">Copy</CopyButton>
        </div>
      </div>
      <p className="text-xs leading-5 text-muted-foreground">{children}</p>
    </div>
  );
}

function createdReveal(created: GitlabConnectionCreated): SecretReveal {
  return {
    title: "Set up the GitLab webhook",
    description: `${created.connection.name} is connected. Add a webhook in GitLab so GridOps hears about jobs. The secret is shown only once; store it now.`,
    secretLabel: "Webhook secret",
    value: created.webhookSecret,
    details: (
      <WebhookDetails webhookUrl={created.webhookUrl}>
        In GitLab, open the project’s Settings → Webhooks (for a group: the group’s Settings → Webhooks), choose Add new webhook,
        paste the URL and the secret into Secret token, and tick <strong className="font-medium text-foreground">Job events</strong> under
        Trigger. Leave the other triggers off.
      </WebhookDetails>
    ),
  };
}

function rotatedReveal(rotated: GitlabWebhookSecret): SecretReveal {
  return {
    title: "New webhook secret",
    description: "The previous secret no longer works. Paste this one into Secret token in GitLab, under Settings → Webhooks, to restore delivery. It is shown only once.",
    secretLabel: "Webhook secret",
    value: rotated.webhookSecret,
    details: (
      <WebhookDetails webhookUrl={rotated.webhookUrl}>
        The webhook URL is unchanged. Keep <strong className="font-medium text-foreground">Job events</strong> ticked.
      </WebhookDetails>
    ),
  };
}

const scopeKindLabel = (scopeKind: ScopeKind) => SCOPE_KINDS.find((kind) => kind.value === scopeKind)?.label ?? scopeKind;

// ---- Targets -----------------------------------------------------------------------------------

const groupHeading = (label: string) => `GitLab · ${label}`;

function connectionOption(kindId: string, label: string, connection: GitlabPoolConnection, index: number): TargetOption {
  return {
    key: `${kindId}:${connection.id}`,
    kindId,
    connectionId: connection.id,
    nativeId: connection.id,
    label: connection.name,
    description: `${connection.baseUrl}/${connection.scopePath}`,
    group: groupHeading(label),
    loadIndex: index,
  };
}

/** One kind per GitLab scope: each connection is exactly one project or one group, so an option is a connection. */
function scopeKind(id: string, label: "Project" | "Group", kind: ScopeKind, plural: string): TargetKind {
  return {
    id,
    label,
    group: groupHeading(label),
    cardinality: "many",
    standalone: true,
    excludes: OTHER_PLATFORM_KINDS,
    exclusionReason: "Can’t combine with GitLab targets",
    // A new pool lists the connections in its options; a GitLab pool's settings fetch them (below).
    available: (context) => context.form === "create"
      ? (context.options.gitlabConnections ?? []).some((connection) => connection.scopeKind === kind)
      : context.pool.scope === GITLAB,
    notConnected: `No GitLab ${plural} are connected yet`,
    // A pool never changes between GitLab and another platform's scope.
    offered: (context) => context.form === "create" || context.pool.scope === GITLAB,
    limit: ({ maxCount }) => maxCount,
    // Each connection registers its own runner, so the maximum must cover them all.
    raisesMinMaxCount: true,
    planNote: (count, { maxCount }) => ({
      ok: count <= maxCount,
      text: `Each selected GitLab project or group registers its own runner; the ${maxCount}-runner maximum must cover them all.`,
    }),
    async load(context, signal) {
      // The pool detail lists only the connections the pool already uses, so settings fetch every one.
      const connections: readonly GitlabPoolConnection[] = context.form === "create"
        ? context.options.gitlabConnections ?? []
        : (await getGitlabConnections(signal)).items;
      return connections
        .filter((connection) => connection.scopeKind === kind)
        .map((connection, index) => connectionOption(id, label, connection, index));
    },
  };
}

const projectKind = scopeKind(PROJECT, "Project", "project", "projects");
const groupKind = scopeKind(GROUP, "Group", "group", "groups");

function selectedConnectionIds(selection: ResolvedSelection) {
  return [...(selection[PROJECT] ?? []), ...(selection[GROUP] ?? [])].map((option) => option.nativeId);
}

function poolSelection(pool: RunnerPoolDetail): ResolvedSelection {
  const connections = pool.gitlabConnections ?? [];
  const selection: Record<string, TargetOption[]> = {};
  for (const [kindId, label, kind] of [[PROJECT, "Project", "project"], [GROUP, "Group", "group"]] as const) {
    const options = connections.filter((connection) => connection.scopeKind === kind).map((connection, index) => connectionOption(kindId, label, connection, index));
    if (options.length) selection[kindId] = options;
  }
  return selection;
}

// ---- Platform options --------------------------------------------------------------------------

type GitlabOptions = { runUntagged: boolean };

const initialOptions = (pool?: RunnerPoolDetail): GitlabOptions => ({ runUntagged: pool?.runUntagged ?? false });

/** The form's GitLab option values; a form that holds none keeps `fallback` (the pool's, or the defaults). */
function gitlabOptions(values: TargetCreateValues | TargetUpdateValues | undefined, fallback: GitlabOptions): GitlabOptions {
  const own = values?.platformOptions?.[GITLAB] as Partial<GitlabOptions> | undefined;
  return { runUntagged: typeof own?.runUntagged === "boolean" ? own.runUntagged : fallback.runUntagged };
}

function RunUntaggedOption({ values, onChange }: PlatformOptionsProps) {
  const options = (values as GitlabOptions | undefined) ?? initialOptions();
  return (
    <div className="flex items-start justify-between gap-4 pt-1">
      <div className="min-w-0">
        <div className="text-sm text-foreground" id="gitlab-run-untagged-label">Run untagged jobs</div>
        <p className="text-xs leading-5 text-muted-foreground">Let this pool’s runners also pick up jobs that have no tags. Off by default.</p>
      </div>
      <Switch
        aria-labelledby="gitlab-run-untagged-label"
        checked={options.runUntagged}
        onCheckedChange={(checked) => onChange({ ...options, runUntagged: checked })}
      />
    </div>
  );
}

// ---- Pool destination --------------------------------------------------------------------------

/** The parts of a connection a label is made from: a pool's connection, or the one a runner carries (any part may be missing). */
type ConnectionParts = { name?: string | null; baseUrl?: string | null; scopePath?: string | null };

/** A connection's `baseUrl/scopePath`, with only the parts that are present. */
function connectionAddress(connection: ConnectionParts) {
  return [connection.baseUrl?.replace(/\/+$/, ""), connection.scopePath].filter(Boolean).join("/");
}

/** A connection's name, or its address when it has none. */
const connectionLabel = (connection: ConnectionParts) => connection.name || connectionAddress(connection);

/** A runner's connection name, else its address, else the platform; never the pool's installation. */
function runnerDestination(runner: Runner): RunnerDestinationView {
  const text = connectionLabel({ name: runner.gitlabConnectionName, baseUrl: runner.gitlabBaseUrl, scopePath: runner.gitlabScopePath }) || "GitLab";
  return { text, title: text };
}

/** The list cell: the one connection's name, or how many there are with every name in the title. */
function poolDestination(pool: RunnerPool): PoolDestinationView {
  const labels = (pool.gitlabConnections ?? []).map(connectionLabel);
  const leading = <Gitlab aria-hidden="true" className="size-4 shrink-0" />;
  // No connections listed (none, or a server before #159): the platform, never the pool's installation.
  if (!labels.length) return { text: "GitLab", title: "GitLab", leading };
  return { text: labels.length === 1 ? labels[0] ?? "" : `${labels.length} connections`, title: labels.join(", "), leading };
}

/** The pool detail's Destination rows: the scope, then each connection (name, then address) in the server's order. */
function PoolDestination({ pool }: { pool: RunnerPoolDetail }) {
  const connections = pool.gitlabConnections ?? [];
  return (
    <>
      <PropertyRow label="Scope">GitLab</PropertyRow>
      {connections.length ? (
        <div className="space-y-1 pt-1">
          {connections.map((connection) => {
            const address = connectionAddress(connection);
            return (
              <div className="rounded-md bg-panel-subtle px-2 py-1 text-xs" key={connection.id} title={[connection.name, address].filter(Boolean).join("\n")}>
                <div className="truncate text-secondary-foreground">{connectionLabel(connection)}</div>
                {connection.name && address ? <div className="truncate text-muted-foreground">{address}</div> : null}
              </div>
            );
          })}
        </div>
      ) : <div className="pt-1 text-xs text-faint">No connections listed</div>}
    </>
  );
}

// ---- Descriptor --------------------------------------------------------------------------------

export const gitlabPlatform: PlatformDescriptor = {
  id: GITLAB,
  name: "GitLab",
  icon: Gitlab,
  badgeDotClassName: "bg-warning",
  connections: {
    list: { kind: "fetch", load: (signal) => getGitlabConnections(signal) },
    describe: (item) => {
      const connection = item as GitlabConnection;
      return {
        id: connection.id,
        name: connection.name,
        badges: <Badge variant="outline">{scopeKindLabel(connection.scopeKind)}</Badge>,
        detail: `${connection.baseUrl.replace(/^https?:\/\//, "")}/${connection.scopePath}`,
        aside: <span className="hidden text-xs text-faint sm:inline">Verified {formatRelativeTime(connection.updatedAt)}</span>,
      };
    },
    emptyText: () => "No GitLab projects or groups are connected yet.",
    chooserDescription: "GitLab projects and groups with managed runners.",
    fields: [
      { key: "name", label: "Connection name", type: "text", required: true, placeholder: "Mobile app", editableOnUpdate: true },
      {
        key: "baseUrl",
        label: "GitLab address",
        type: "text",
        required: false,
        defaultValue: DEFAULT_ADDRESS,
        placeholder: DEFAULT_ADDRESS,
        help: "Change it for a self-managed GitLab, for example https://git.example.com.",
        validate: gitlabAddressError,
        editableOnUpdate: false,
      },
      {
        key: "scopeKind",
        label: "Connect a",
        type: "select",
        options: SCOPE_KINDS,
        defaultValue: "project",
        required: true,
        help: "A project registers a project runner; a group registers a group runner shared by its projects.",
        editableOnUpdate: false,
      },
      {
        key: "scopePath",
        label: "Project or group path",
        type: "text",
        required: true,
        placeholder: "acme/platform/mobile-app",
        help: "The path in the project’s or group’s URL, without a leading or trailing slash.",
        validate: gitlabScopePathError,
        editableOnUpdate: false,
      },
      {
        key: "accessToken",
        label: "Access token",
        type: "secret",
        required: true,
        placeholder: "Paste a token",
        help: "The token needs the create_runner, manage_runner and read_api scopes (or api), and its user needs the Maintainer role or higher on a project, or the Owner role on a group. GridOps checks both before saving.",
        editableOnUpdate: true,
        updateLabel: "New access token",
        updatePlaceholder: "Leave blank to keep the current token",
      },
    ],
    create: (values) => createGitlabConnection({
      name: values.name ?? "",
      baseUrl: normalizeAddress(values.baseUrl),
      scopeKind: values.scopeKind === "group" ? "group" : "project",
      scopePath: (values.scopePath ?? "").trim(),
      accessToken: values.accessToken ?? "",
    }),
    update: (item, values) => {
      const token = values.accessToken?.trim();
      return updateGitlabConnection((item as GitlabConnection).id, { name: values.name ?? "", ...(token ? { accessToken: token } : {}) });
    },
    remove: (item) => removeGitlabConnection((item as GitlabConnection).id),
    actions: (item) => {
      const connection = item as GitlabConnection;
      return [
        { label: "Verify", run: () => verifyGitlabConnection(connection.id), success: "GitLab connection verified." },
        {
          label: "Rotate webhook secret",
          run: async () => rotatedReveal(await rotateGitlabWebhookSecret(connection.id)),
          confirm: `Rotate the webhook secret for ${connection.name}? The old secret stops working at once, so GitLab’s webhook will fail until you paste the new secret into it.`,
          confirmOptions: { confirmLabel: "Rotate secret" },
          success: "Webhook secret rotated.",
        },
      ];
    },
    editValues: (item) => ({ name: (item as GitlabConnection).name, accessToken: "" }),
    createTitle: "Connect GitLab",
    createDescription: "GridOps verifies the token before saving. The token is encrypted at rest and never returned to the browser. Each project or group is connected once per GitLab address; edit an existing connection to replace its token.",
    createSubmitLabel: "Connect GitLab",
    updateSuccess: (values) => values.accessToken?.trim() ? "Connection updated and token replaced." : "Connection updated.",
    removeConfirm: (item) => `Remove ${(item as GitlabConnection).name}? GridOps deletes its stored token and webhook secret. Pools that use it must be changed first. Its fleet targets are retired and their machine grants revoked; their fleet history is kept. The webhook in GitLab is not deleted; remove it there too.`,
    removeSuccess: (item) => `${(item as GitlabConnection).name} removed.`,
  },
  postCreate: (created: GitlabConnectionCreated) => createdReveal(created),
  targetKinds: [projectKind, groupKind],
  labelRules: {
    help: "Comma-separated tags for the GitLab runner. A tag cannot contain a comma and is at most 64 characters.",
    placeholder: "build, ios",
    validate: (labels) => gitlabLabelError(labels),
  },
  // GITLAB_RUNNER_IMAGE in crates/gridops-core/src/provisioning.rs (#172); a test pins the two equal.
  defaultDockerImage: "gitlab/gitlab-runner:ubuntu-v19.6.0",
  platformOptions: {
    help: [
      "Labels on a GitLab pool become the runner’s tags. A tag may contain any character except a comma and is at most 64 characters.",
      "Jobs with no tags run on this runner only when “Run untagged jobs” is on.",
    ],
    initialValues: initialOptions,
    Component: RunUntaggedOption,
  },
  runnerLink: (runner) => {
    const { gitlabRunnerId, gitlabBaseUrl, gitlabScopePath } = runner;
    if (gitlabRunnerId === null || gitlabRunnerId === undefined || !gitlabBaseUrl || !gitlabScopePath) return null;
    return { href: `${gitlabBaseUrl.replace(/\/+$/, "")}/${gitlabScopePath}/-/runners/${gitlabRunnerId}`, label: "Open on GitLab" };
  },
  toCreatePayload(selection, context, values) {
    const gitlabConnectionIds = selectedConnectionIds(selection);
    if (!gitlabConnectionIds.length) return {};
    // #138 still requires an installation on every pool, so a GitLab pool takes the first one.
    const installationId = context.form === "create" ? context.options.installations[0]?.id ?? 0 : 0;
    return {
      scope: GITLAB,
      installationId,
      repositoryIds: [],
      runnerGroupId: 1,
      gitlabConnectionIds,
      runUntagged: gitlabOptions(values, initialOptions()).runUntagged,
    };
  },
  toUpdatePayload(selection, pool, values) {
    if (pool.scope !== GITLAB) return {};
    const stored = initialOptions(pool);
    const { runUntagged } = gitlabOptions(values, stored);
    return {
      // The full set replaces the stored one.
      gitlabConnectionIds: selectedConnectionIds(selection),
      // Sent only when the switch changed it.
      ...(runUntagged !== stored.runUntagged ? { runUntagged } : {}),
    };
  },
  initialSelection: poolSelection,
  readOnlySummary: (pool) => pool.gitlabConnections?.length ? pool.gitlabConnections.map((connection) => connection.name).join(", ") : null,
  pool: { scopes: [GITLAB], destination: poolDestination, Destination: PoolDestination, runnerDestination, scopeLabel: () => "GitLab" },
};
