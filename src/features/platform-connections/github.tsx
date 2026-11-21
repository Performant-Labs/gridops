/** The GitHub platform descriptor (#145): App installations, repositories and organization runner groups. */
import { ChevronDown, ExternalLink, Github } from "lucide-react";
import { useState } from "react";

import { PropertyRow } from "~/components/page";
import { Avatar, githubAvatar } from "~/components/ui/avatar";
import { Badge } from "~/components/ui/badge";
import { buttonVariants } from "~/components/ui/button";
import type { Runner, RunnerPool, SettingsPage } from "~/features/operations/operations.functions";
import {
  type RunnerPoolDetail,
  getInstallationRunnerGroups,
  getRunnerPoolRepositories,
} from "~/features/runner-pools/runner-pools.functions";
import { cn, formatRelativeTime } from "~/lib/utils";

import type { PlatformDescriptor, PoolDestinationView, ResolvedSelection, RunnerDestinationView, TargetContext, TargetKind, TargetOption } from "./types";

type Installation = NonNullable<SettingsPage["data"]>["installations"][number];

const REPOSITORY = "github.repository";
const RUNNER_GROUP = "github.runner-group";
const REPOSITORY_GROUP = "GitHub · Repository";
const RUNNER_GROUP_GROUP = "GitHub · Organization runner group";
const RUNNER_GROUP_SENTENCE = "GitHub runner-group access defines which repositories can use the pool.";

const repositoryKey = (id: number) => `${REPOSITORY}:${id}`;
const runnerGroupKey = (nativeId: string) => `${RUNNER_GROUP}:${nativeId}`;

function isAbort(cause: unknown) {
  return cause instanceof DOMException && cause.name === "AbortError";
}

/** A runner group's native id is `<installationId>/<groupId>`: every organization has a group 1. */
function runnerGroupParts(option: TargetOption | undefined) {
  const [installationId, groupId] = (option?.nativeId ?? "").split("/");
  return { installationId: Number(installationId) || 0, groupId: Number(groupId) || 0 };
}

function manualRunnerGroup(installation: { id: number; accountLogin: string }, description: string): TargetOption {
  return {
    key: runnerGroupKey(`${installation.id}/manual`),
    kindId: RUNNER_GROUP,
    connectionId: String(installation.id),
    nativeId: `${installation.id}/0`,
    label: `${installation.accountLogin} · Enter runner group ID`,
    description,
    group: RUNNER_GROUP_GROUP,
    manual: true,
  };
}

/** One installation's runner groups, or the manual entry when they cannot be listed (as today). */
async function installationRunnerGroups(installation: { id: number; accountLogin: string; accountType?: string }, signal: AbortSignal): Promise<TargetOption[]> {
  if (installation.accountType !== undefined && installation.accountType !== "Organization") {
    return [manualRunnerGroup(installation, "Runner groups are listed for organizations only")];
  }
  try {
    const { items } = await getInstallationRunnerGroups(installation.id, signal);
    if (!items.length) return [manualRunnerGroup(installation, "No runner groups are listed for this installation")];
    return items.map((group) => ({
      key: runnerGroupKey(`${installation.id}/${group.id}`),
      kindId: RUNNER_GROUP,
      connectionId: String(installation.id),
      nativeId: `${installation.id}/${group.id}`,
      label: `${installation.accountLogin} · ${group.name}`,
      description: group.isDefault ? "Default runner group" : `${group.visibility} visibility`,
      group: RUNNER_GROUP_GROUP,
    }));
  } catch (cause) {
    if (isAbort(cause)) throw cause;
    return [manualRunnerGroup(installation, "Runner groups could not be loaded")];
  }
}

const installationsAvailable = (context: TargetContext) =>
  context.form === "create" ? context.options.installations.length > 0 : true;

const repositoryKind: TargetKind = {
  id: REPOSITORY,
  label: "Repository",
  group: REPOSITORY_GROUP,
  cardinality: "many",
  standalone: true,
  excludes: [RUNNER_GROUP],
  exclusionReason: "Can’t combine with repositories",
  available: installationsAvailable,
  notConnected: "Install the GitHub App first",
  offered: (context) => context.form === "create" || context.pool.scope === "repository",
  limit: ({ maxCount }) => maxCount,
  raisesMinMaxCount: true,
  supportsPriority: true,
  planNote: (count, { maxCount }) => ({ ok: count <= maxCount, text: `${count} selected repositories fit within the ${maxCount}-runner maximum.` }),
  defaultForPlan: true,
  async load(_context, signal) {
    const { items } = await getRunnerPoolRepositories(signal);
    return items.map((repository, index) => ({
      key: repositoryKey(repository.id),
      kindId: REPOSITORY,
      connectionId: String(repository.installationId),
      nativeId: String(repository.id),
      label: repository.fullName,
      description: `${repository.accountLogin} · ${repository.private ? "Private" : "Public"}`,
      group: REPOSITORY_GROUP,
      loadIndex: index,
    }));
  },
};

const runnerGroupKind: TargetKind = {
  id: RUNNER_GROUP,
  label: "Organization runner group",
  group: RUNNER_GROUP_GROUP,
  cardinality: "one",
  standalone: true,
  excludes: [REPOSITORY],
  exclusionReason: "Can’t combine with a runner group",
  available: installationsAvailable,
  notConnected: "Install the GitHub App first",
  offered: (context) => context.form === "create" || context.pool.scope === "organization",
  planNote: () => ({ ok: true, text: RUNNER_GROUP_SENTENCE }),
  manualEntry: {
    label: "Runner group ID",
    parse: (input, connectionId) => {
      const id = Number(input);
      return Number.isInteger(id) && id > 0 ? `${connectionId}/${id}` : null;
    },
    initialValue: (context) => String(context.form === "create" ? context.options.defaults?.runnerGroupId ?? 1 : context.pool.runnerGroupId),
  },
  async load(context, signal) {
    // Pool settings keep the pool's installation; a new pool may use any organization installation.
    const installations = context.form === "create"
      ? context.options.installations
      : [{ id: context.pool.installationId, accountLogin: context.pool.accountLogin }];
    const groups = await Promise.all(installations.map((installation) => installationRunnerGroups(installation, signal)));
    return groups.flat().map((option, index) => ({ ...option, loadIndex: index }));
  },
};

function repositorySelection(pool: RunnerPoolDetail): TargetOption[] {
  return pool.repositoryIds.map((id) => {
    const repository = pool.repositories.find((candidate) => candidate.id === id);
    return {
      key: repositoryKey(id),
      kindId: REPOSITORY,
      connectionId: String(repository?.installationId ?? pool.installationId),
      nativeId: String(id),
      label: repository?.fullName ?? `Repository ${id}`,
      description: repository ? `${repository.accountLogin} · ${repository.private ? "Private" : "Public"}` : undefined,
      group: REPOSITORY_GROUP,
    };
  });
}

function selectedIds(selection: ResolvedSelection) {
  return (selection[REPOSITORY] ?? []).map((option) => Number(option.nativeId));
}

function InstallationAside({ installation }: { installation: Installation }) {
  return installation.permission === "admin" ? (
    <a className={buttonVariants({ size: "sm", variant: "ghost" })} href={installation.manageUrl} rel="noreferrer" target="_blank">Configure<ExternalLink /></a>
  ) : <span className="text-xs text-faint">Owner approval required</span>;
}

// ---- Pool destination --------------------------------------------------------------------------

/** The list cell, as before #159: the repository count, the repository, or the account, with the account's avatar. */
function poolDestination(pool: RunnerPool): PoolDestinationView {
  const text = pool.scope === "repository" && pool.repositoryCount > 1 ? `${pool.repositoryCount} repositories` : pool.repository ?? pool.accountLogin;
  return { text, title: text, leading: <Avatar name={pool.accountLogin} size={16} square src={githubAvatar(pool.accountLogin)} /> };
}

const COLLAPSED_REPOSITORIES = 6;

function RepositoryList({ repositories }: { repositories: RunnerPoolDetail["repositories"] }) {
  const [expanded, setExpanded] = useState(false);
  const hidden = repositories.length - COLLAPSED_REPOSITORIES;
  const visible = expanded ? repositories : repositories.slice(0, COLLAPSED_REPOSITORIES);
  return (
    <div className="space-y-1 pt-1">
      {visible.map((repository) => (
        <div className="truncate rounded-md bg-panel-subtle px-2 py-1 text-xs text-secondary-foreground" key={repository.id} title={repository.fullName}>{repository.fullName}</div>
      ))}
      {hidden > 0 ? (
        <button
          aria-expanded={expanded}
          className="flex h-6 items-center gap-1 rounded-md px-2 text-xs text-muted-foreground transition-colors hover:bg-hover hover:text-foreground"
          onClick={() => setExpanded((current) => !current)}
          type="button"
        >
          <ChevronDown className={cn("size-3 transition-transform duration-200 ease-out-strong", expanded && "rotate-180")} />
          {expanded ? "Show fewer" : `Show ${hidden} more`}
        </button>
      ) : null}
    </div>
  );
}

/** The pool detail's Destination rows: the scope, the account, then the repositories or the runner group. */
function PoolDestination({ pool }: { pool: RunnerPoolDetail }) {
  return (
    <>
      <PropertyRow label="Scope"><span className="capitalize">{pool.scope}</span></PropertyRow>
      <PropertyRow label="Account"><Avatar name={pool.accountLogin} size={16} square src={githubAvatar(pool.accountLogin)} />{pool.accountLogin}</PropertyRow>
      {pool.scope === "repository" ? (
        <RepositoryList repositories={pool.repositories} />
      ) : <PropertyRow label="Runner group"><span className="tabular">#{pool.runnerGroupId}</span></PropertyRow>}
    </>
  );
}

/** A runner's destination, as before #169: its repository, or the account it registers under. */
function runnerDestination(runner: Runner): RunnerDestinationView {
  const text = runner.repository ?? runner.accountLogin;
  return { text, title: text };
}

// ---- Descriptor --------------------------------------------------------------------------------

export const githubPlatform: PlatformDescriptor = {
  id: "github",
  name: "GitHub",
  icon: Github,
  badgeDotClassName: "bg-foreground",
  connections: {
    // Installations come from the `/settings` page data: no second fetch, nothing to retry.
    list: {
      kind: "page",
      read: (page) => ({ canManage: page.data?.user.role === "admin", items: page.data?.installations ?? [] }),
    },
    describe: (item) => {
      const installation = item as Installation;
      return {
        id: String(installation.id),
        name: installation.accountLogin,
        leading: <Avatar name={installation.accountLogin} size={28} square src={installation.accountAvatarUrl} />,
        badges: (
          <>
            <span className="text-xs text-faint">{installation.accountType}</span>
            {installation.suspended ? <Badge variant="destructive">Suspended</Badge> : null}
          </>
        ),
        detail: <>{installation.repositorySelection === "all" ? "All repositories" : "Selected repositories"} · {installation.poolCount} {installation.poolCount === 1 ? "pool" : "pools"}{installation.lastSyncedAt ? ` · synced ${formatRelativeTime(installation.lastSyncedAt)}` : ""}</>,
        aside: <InstallationAside installation={installation} />,
      };
    },
    // Before the GitHub App exists there is nothing to install, so GitHub shows no empty row.
    emptyText: (page) => page.data?.githubApp ? `No installations are visible to @${page.data.user.login}.` : null,
    chooserDescription: "Runners for GitHub Actions. Added by installing the GridOps GitHub App.",
    // GitHub accounts are connected on GitHub, by installing the App; GridOps has no create form for them.
    connectLink: (page) => {
      const installUrl = page.data?.githubApp?.installUrl ?? null;
      return {
        label: "Install GitHub App",
        description: "GitHub accounts are connected by installing the GridOps GitHub App on them. GridOps then picks the installation up automatically after you return and refresh access.",
        href: installUrl,
        ...(installUrl ? {} : { unavailableText: "Create the GitHub App first (below on this page)." }),
      };
    },
  },
  targetKinds: [repositoryKind, runnerGroupKind],
  // Deliveries with no other platform's id prefix are GitHub's; one with no repository or account came to the App.
  deliveryDestinationFallback: "GitHub App",
  runnerLink: (runner) => runner.repository && runner.githubRunnerId !== null
    ? { href: `https://github.com/${runner.repository}/settings/actions/runners/${runner.githubRunnerId}`, label: "Open on GitHub" }
    : null,
  toCreatePayload(selection) {
    const group = selection[RUNNER_GROUP]?.[0];
    if (group) {
      const { installationId, groupId } = runnerGroupParts(group);
      return { installationId, scope: "organization", repositoryIds: [], runnerGroupId: groupId || 1 };
    }
    const repositories = selection[REPOSITORY] ?? [];
    // The installation is the first selected repository's in the loaded list's order, as today;
    // repositories from other installations may still be selected together.
    const first = [...repositories].sort((a, b) => (a.loadIndex ?? 0) - (b.loadIndex ?? 0))[0];
    return {
      installationId: first ? Number(first.connectionId) : 0,
      scope: "repository",
      repositoryIds: repositories.map((option) => Number(option.nativeId)),
      runnerGroupId: 1,
    };
  },
  toUpdatePayload(selection, pool, values) {
    if (pool.scope === "repository") {
      const repositoryIds = selectedIds(selection);
      const selected = selection[REPOSITORY] ?? [];
      const priorityRepositoryIds = values.priorityKeys
        .map((key) => selected.find((option) => option.key === key))
        .filter((option): option is TargetOption => option !== undefined)
        .map((option) => Number(option.nativeId))
        .filter((id) => repositoryIds.includes(id));
      return { repositoryIds, priorityRepositoryIds, priorityWaitMinutes: values.priorityWaitMinutes, runnerGroupId: 1 };
    }
    // A pool of another platform's scope has no GitHub targets to save.
    if (pool.scope !== "organization") return {};
    const { groupId } = runnerGroupParts(selection[RUNNER_GROUP]?.[0]);
    return {
      repositoryIds: undefined,
      priorityRepositoryIds: undefined,
      priorityWaitMinutes: undefined,
      runnerGroupId: groupId || pool.runnerGroupId,
    };
  },
  // Repository and organization pools are GitHub's; any other scope belongs to another platform.
  initialSelection: (pool) => pool.scope === "repository"
    ? { [REPOSITORY]: repositorySelection(pool) }
    : pool.scope !== "organization" ? {} : {
      [RUNNER_GROUP]: [{
        key: runnerGroupKey(`${pool.installationId}/${pool.runnerGroupId}`),
        kindId: RUNNER_GROUP,
        connectionId: String(pool.installationId),
        nativeId: `${pool.installationId}/${pool.runnerGroupId}`,
        label: `${pool.accountLogin} · Runner group ${pool.runnerGroupId}`,
        group: RUNNER_GROUP_GROUP,
      }],
    },
  initialPriorityKeys: (pool) => pool.scope === "repository" ? pool.priorityRepositoryIds.map(repositoryKey) : [],
  readOnlySummary: (pool) => pool.scope === "repository" ? `${pool.repositoryIds.length} repositories` : pool.scope === "organization" ? pool.accountLogin : null,
  // A Bitbucket workspace rides on a GitHub repository or runner-group pool, so that pool's destination is GitHub's.
  // The Overview shows the raw scope in a `capitalize` span, so it reads "Repository" / "Organization" as before #169.
  pool: { scopes: ["repository", "organization"], destination: poolDestination, Destination: PoolDestination, runnerDestination, scopeLabel: (pool) => pool.scope },
  prerequisiteEmptyState(context) {
    if (context.form !== "create") return null;
    const { options } = context;
    if (!options.authenticated || !options.defaults) {
      return {
        icon: Github,
        title: "Connect GitHub first",
        description: "GridOps needs an authorized GitHub App installation before it can create a repository or organization runner pool.",
        action: { label: "Connect GitHub", href: "/auth/github?returnTo=/runner-pools/new" },
      };
    }
    if (options.installations.length === 0) {
      return {
        icon: Github,
        title: "Install the GitHub App",
        description: "Choose the account and repositories GridOps may operate, then come back here and refresh GitHub access.",
        action: { label: "Install GridOps on GitHub", href: options.installUrl },
      };
    }
    return null;
  },
};
