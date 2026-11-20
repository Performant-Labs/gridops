/** The Bitbucket Cloud platform descriptor (#145): connected workspaces, which share a GitHub pool's capacity. */
import { CloudCog } from "lucide-react";

import { bitbucketLabelError } from "~/features/runner-pools/schemas";
import { formatRelativeTime } from "~/lib/utils";

import {
  type BitbucketConnection,
  createBitbucketConnection,
  getBitbucketConnections,
  removeBitbucketConnection,
  updateBitbucketConnection,
} from "./platform-connections.functions";
import type { PlatformDescriptor, TargetContext, TargetKind, TargetOption } from "./types";

const WORKSPACE = "bitbucket.workspace";
const WORKSPACE_GROUP = "Bitbucket · Workspace";

type WorkspaceRecord = { id: string; name: string; workspace: string; workspaceUuid: string };

function workspaceOption(connection: WorkspaceRecord, index: number): TargetOption {
  return {
    key: `${WORKSPACE}:${connection.id}`,
    kindId: WORKSPACE,
    connectionId: connection.id,
    nativeId: connection.id,
    label: connection.name,
    description: `${connection.workspace} workspace`,
    group: WORKSPACE_GROUP,
    loadIndex: index,
  };
}

/** The workspaces the form already holds: the options of a new pool, or the pool's own on settings. */
const workspaces = (context: TargetContext): WorkspaceRecord[] =>
  context.form === "create" ? context.options.bitbucketConnections : context.pool.bitbucketConnections;

const workspaceKind: TargetKind = {
  id: WORKSPACE,
  label: "Workspace",
  group: WORKSPACE_GROUP,
  cardinality: "many",
  // A pool still needs a GitHub installation and scope (createRunnerPoolSchema), so a workspace
  // shares a GitHub target's pool rather than standing alone.
  standalone: false,
  requires: ["github.repository", "github.runner-group"],
  requiresReason: "Select a GitHub repository or runner group first",
  requiresProviders: ["tart"],
  available: (context) => workspaces(context).length > 0,
  notConnected: "No Bitbucket workspaces are connected yet",
  // Pool settings never sent workspaces: they show there as locked chips.
  offered: (context) => context.form === "create",
  limit: ({ maxCount }) => maxCount,
  load: async (context) => workspaces(context).map(workspaceOption),
};

export const bitbucketPlatform: PlatformDescriptor = {
  id: "bitbucket",
  name: "Bitbucket",
  icon: CloudCog,
  badgeDotClassName: "bg-info",
  connections: {
    list: { kind: "fetch", load: (signal) => getBitbucketConnections(signal) },
    describe: (item) => {
      const connection = item as BitbucketConnection;
      return {
        id: connection.id,
        name: connection.name,
        detail: `bitbucket.org/${connection.workspace}`,
        aside: <span className="hidden text-xs text-faint sm:inline">Verified {formatRelativeTime(connection.updatedAt)}</span>,
      };
    },
    emptyText: () => "No Bitbucket workspaces are connected yet.",
    fields: [
      { key: "name", label: "Connection name", type: "text", required: true, placeholder: "Mobile delivery", editableOnUpdate: true },
      { key: "workspace", label: "Workspace slug", type: "text", required: true, placeholder: "acme-mobile", editableOnUpdate: false },
      {
        key: "accessToken",
        label: "API token",
        type: "secret",
        required: true,
        placeholder: "Paste a token",
        help: "Needs the read:workspace, read:runner and write:runner Bitbucket scopes. GridOps checks them before saving.",
        editableOnUpdate: true,
        updateLabel: "New API token",
        updatePlaceholder: "Leave blank to keep the current token",
      },
    ],
    create: (values) => createBitbucketConnection({ name: values.name ?? "", workspace: values.workspace ?? "", accessToken: values.accessToken ?? "" }),
    update: (item, values) => {
      const token = values.accessToken?.trim();
      return updateBitbucketConnection((item as BitbucketConnection).id, { name: values.name ?? "", ...(token ? { accessToken: token } : {}) });
    },
    remove: (item) => removeBitbucketConnection((item as BitbucketConnection).id),
    editValues: (item) => ({ name: (item as BitbucketConnection).name, accessToken: "" }),
    createTitle: "Connect a Bitbucket workspace",
    createDescription: "GridOps verifies the workspace before saving. The token is encrypted at rest and never returned to the browser. Each workspace is connected once; edit an existing connection to replace its token.",
    createSubmitLabel: "Connect workspace",
    createSuccess: "Bitbucket workspace connected.",
    updateSuccess: (values) => values.accessToken?.trim() ? "Connection updated and token replaced." : "Connection updated.",
    removeConfirm: (item) => `Remove ${(item as BitbucketConnection).name}? GridOps deletes its stored token. Pools that use it must be changed first.`,
    removeSuccess: (item) => `${(item as BitbucketConnection).name} removed.`,
  },
  targetKinds: [workspaceKind],
  labelRules: {
    help: "Comma-separated. Bitbucket labels may use lowercase letters, numbers, and dots.",
    placeholder: "build, release.ios",
    validate: (labels) => bitbucketLabelError(labels),
  },
  platformOptions: {
    help: [
      "Each workspace shares this pool’s capacity with a managed macOS runner for Bitbucket Pipelines.",
      "Bitbucket labels may use lowercase letters, numbers, and dots.",
      <>Prepare the Tart base VM for Bitbucket workspaces with <code className="font-mono">scripts/prepare-tart-bitbucket-runner-image.sh</code>.</>,
    ],
  },
  runnerLink: () => null,
  toCreatePayload: (selection) => ({ bitbucketConnectionIds: (selection[WORKSPACE] ?? []).map((option) => option.nativeId) }),
  // Pool settings never sent workspaces; they stay as they are.
  toUpdatePayload: () => ({}),
  initialSelection: (pool) => pool.bitbucketConnections.length ? { [WORKSPACE]: pool.bitbucketConnections.map(workspaceOption) } : {},
  readOnlySummary: (pool) => pool.bitbucketConnections.length ? pool.bitbucketConnections.map((connection) => connection.name).join(", ") : null,
};
