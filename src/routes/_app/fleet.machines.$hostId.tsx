import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, createFileRoute } from "@tanstack/react-router";
import { KeyRound, LoaderCircle, Plus, RefreshCw, Server, ShieldCheck } from "lucide-react";
import { useState } from "react";
import type * as React from "react";
import { toast } from "sonner";

import { confirmAction } from "~/components/confirm-dialog";
import { CopyButton } from "~/components/copy-button";
import {
  Callout, EmptyState, InlineLoading, PageBody, PageHeader, PropertiesPanel, PropertyGroup, PropertyRow, SectionHeading,
} from "~/components/page";
import { ResourcePageLoading } from "~/components/resource-page-loading";
import { Switch } from "~/components/settings-ui";
import { Badge } from "~/components/ui/badge";
import { Button, buttonVariants } from "~/components/ui/button";
import { SearchableMultiSelect } from "~/components/ui/searchable-multi-select";
import { listAgentLogs, listHostGrants, listTargets, retireHost, setBackendEnabled, setHostGrant, setHostLifecycle } from "~/features/fleet/admin-api";
import type { AgentLog, GrantPolicy, HostGrant, HostLifecycle } from "~/features/fleet/admin-contracts";
import {
  EnrollmentBadge, FreshnessBadge, PressureBadge, fleetErrorMessage, formatBytes, formatCores, formatMib, isForbidden,
  lifecycleLabel, newIdempotencyKey, osLabel, platformLabel, runtimeLabel,
} from "~/features/fleet/fleet-ui";
import { getHost, listBackends, listHostEvents } from "~/features/fleet/query-api";
import type { BackendReadV1, HostEventReadV1, HostReadV1 } from "~/features/fleet/query-contracts";
import { FleetRequestError, approveHost, recoverHost, rotateHostCredential } from "~/features/fleet/registry-api";
import { cn, formatDateTime, formatRelativeTime } from "~/lib/utils";

export const Route = createFileRoute("/_app/fleet/machines/$hostId")({
  component: MachinePage,
});

type HostItem = Extract<HostReadV1, { projection: "host" }>;
const REFRESH = 10_000;

function MachinePage() {
  const { hostId } = Route.useParams();
  const host = useQuery({
    queryKey: ["fleet", "host", hostId],
    queryFn: ({ signal }) => getHost(hostId, signal),
    refetchInterval: REFRESH,
  });

  if (host.isPending) return <ResourcePageLoading icon={Server} title="Machine" />;
  if (host.isError) {
    const missing = host.error instanceof FleetRequestError && host.error.status === 404;
    return (
      <>
        <PageHeader breadcrumbs={[{ label: "Machines", to: "/fleet" }]} icon={Server} title="Machine" />
        <PageBody>
          {missing ? (
            <EmptyState description="It may have been removed, or you may not have access to it." icon={Server} title="Machine not found">
              <Link className={buttonVariants({ variant: "outline" })} to="/fleet">Back to machines</Link>
            </EmptyState>
          ) : (
            <div className="p-4">
              <Callout action={<Button onClick={() => void host.refetch()} size="sm" variant="outline">Retry</Button>} title="Couldn’t load this machine" tone="danger">
                {fleetErrorMessage(host.error, "The machine could not be loaded.")}
              </Callout>
            </div>
          )}
        </PageBody>
      </>
    );
  }

  const item = host.data.item;
  return (
    <>
      <PageHeader
        actions={
          <>
            <Button aria-label="Refresh" disabled={host.isFetching} onClick={() => void host.refetch()} size="sm" variant="ghost">
              <RefreshCw className={cn(host.isFetching && "animate-spin")} />
            </Button>
            {item.projection === "host" && item.enrollment === "approved" ? <RotateCredentialButton generation={item.credentialGeneration} hostId={item.id} /> : null}
          </>
        }
        breadcrumbs={[{ label: "Machines", to: "/fleet" }]}
        documentTitle={item.name}
        icon={Server}
        title={item.name}
      />
      <PageBody>
        {item.projection === "host" ? <MachineDetail host={item} /> : (
          <div className="p-4">
            <Callout title={platformLabel(item.hostOs, item.architecture)}>
              Runs jobs for repositories you can see. A system administrator manages this machine.
            </Callout>
          </div>
        )}
      </PageBody>
    </>
  );
}

/** Runs one fleet mutation at a time, reports it, and refreshes every fleet view. */
function useFleetAction() {
  const queryClient = useQueryClient();
  const [pending, setPending] = useState<string | null>(null);
  async function run(id: string, action: () => Promise<unknown>, success: string) {
    setPending(id);
    try {
      await action();
      toast.success(success);
    } catch (error) {
      toast.error(fleetErrorMessage(error, "Action failed."));
    } finally {
      setPending(null);
      await queryClient.invalidateQueries({ queryKey: ["fleet"] });
    }
  }
  return { pending, run };
}

function MachineDetail({ host }: { host: HostItem }) {
  const backends = useQuery({
    queryKey: ["fleet", "backends", host.id],
    queryFn: ({ signal }) => listBackends(host.id, { limit: 100 }, signal),
    refetchInterval: REFRESH,
  });
  const events = useQuery({
    queryKey: ["fleet", "events", host.id],
    queryFn: ({ signal }) => listHostEvents(host.id, { limit: 100 }, signal),
    refetchInterval: REFRESH,
  });
  const approved = host.enrollment === "approved";

  return (
    <div className="flex flex-col lg:flex-row">
      <div className="min-w-0 flex-1 space-y-8 p-4 sm:p-6">
        <HostAlerts host={host} />
        {host.enrollment === "pending" ? <ApprovalSection host={host} /> : null}
        {approved && host.lifecycle !== "retired" ? <LifecycleSection host={host} /> : null}
        {approved && host.lifecycle !== "retired" && (host.integrity === "quarantined" || host.heartbeatFreshness === "offline") ? <RecoverSection host={host} /> : null}
        {approved ? <GrantsSection hostId={host.id} /> : null}
        <BackendsSection hostId={host.id} items={backends.data?.items} query={backends} />
        <AgentLogSection hostId={host.id} />
        <EventsSection items={events.data?.items} query={events} />
      </div>
      <PropertiesPanel>
        <PropertyGroup title="Machine">
          <PropertyRow label="Platform">{platformLabel(host.hostOs, host.architecture)}</PropertyRow>
          <PropertyRow label="Enrollment"><EnrollmentBadge value={host.enrollment} /></PropertyRow>
          <PropertyRow label="Lifecycle">{lifecycleLabel[host.lifecycle]}</PropertyRow>
          <PropertyRow label="Integrity">
            {host.integrity === "quarantined" ? <Badge variant="destructive">Quarantined</Badge> : host.integrity === "verified" ? <Badge variant="success">Verified</Badge> : <Badge variant="outline">Unverified</Badge>}
          </PropertyRow>
          <PropertyRow label="Agent"><span className="font-mono text-xs">{host.agentVersion ?? "Unknown"}</span></PropertyRow>
        </PropertyGroup>
        <PropertyGroup title="Health">
          <PropertyRow label="Heartbeat"><FreshnessBadge value={host.heartbeatFreshness} /></PropertyRow>
          <PropertyRow label="Last seen" title={host.lastHeartbeatAt ?? undefined}>{host.lastHeartbeatAt ? formatRelativeTime(host.lastHeartbeatAt) : "Never"}</PropertyRow>
          <PropertyRow label="Pressure"><PressureBadge value={host.pressure} /></PropertyRow>
          <PropertyRow label="Hardware report" title={host.lastInventoryAt ?? undefined}>
            {host.inventoryEvidence.state === "current" ? "Current" : host.inventoryEvidence.state === "stale" ? "Out of date" : "Not received"}
          </PropertyRow>
        </PropertyGroup>
        <PropertyGroup title="Capacity">
          <PropertyRow label="CPU">{formatCores(host.physicalAllocation.charged.cpuMillis)} of {formatCores(host.physicalBudget.cpuMillis)}</PropertyRow>
          <PropertyRow label="Memory">{formatMib(host.physicalAllocation.charged.memoryMib)} of {formatMib(host.physicalBudget.memoryMib)}</PropertyRow>
          <PropertyRow label="Disk">{formatBytes(host.physicalAllocation.charged.diskBytes)} of {formatBytes(host.physicalBudget.diskBytes)}</PropertyRow>
          <PropertyRow label="Runners">{host.counts.knownManagedRunnerCount}</PropertyRow>
          {host.counts.uncertainPlacementCount > 0 ? <PropertyRow label="Unconfirmed">{host.counts.uncertainPlacementCount} jobs</PropertyRow> : null}
        </PropertyGroup>
        <MeasurementGroup host={host} />
      </PropertiesPanel>
    </div>
  );
}

function HostAlerts({ host }: { host: HostItem }) {
  const alerts: React.ReactNode[] = [];
  if (host.duplicateHardware) {
    alerts.push(
      <Callout key="duplicate" title="Another machine reports the same hardware" tone="danger">
        This can mean the agent’s identity was copied to another machine. Check both machines before approving or running jobs.
      </Callout>,
    );
  }
  if (host.integrity === "quarantined") {
    alerts.push(<Callout key="quarantine" title="Quarantined" tone="danger">GridOps stopped trusting this machine’s reports. No new jobs start on it.</Callout>);
  }
  if (host.heartbeatFreshness === "offline") {
    alerts.push(<Callout key="offline" title="Offline" tone="danger">No heartbeat recently. Check that <code className="font-mono">gridops-host-agent run</code> is running on the machine.</Callout>);
  } else if (host.heartbeatFreshness === "stale") {
    alerts.push(<Callout key="stale" title="Heartbeat is late" tone="warning">The machine hasn’t checked in on time. New jobs wait until it does.</Callout>);
  } else if (host.heartbeatFreshness === "unknown") {
    alerts.push(<Callout key="unknown" title="No heartbeat yet" tone="info">Start the agent on the machine with <code className="font-mono">gridops-host-agent run</code>.</Callout>);
  }
  return alerts.length ? <div className="space-y-2">{alerts}</div> : null;
}

const unavailableReason: Record<string, string> = {
  requires_system_admin: "Only system administrators can approve machines.",
  pending_host: "Waiting for the machine to finish enrolling.",
  retired_host: "This machine is retired.",
  missing_control_plane: "GridOps isn’t ready to approve machines yet. Try again shortly.",
  missing_observation: "Waiting for the agent’s first report. Make sure gridops-host-agent run is running.",
  observation_expired: "Waiting for a fresh report from the agent. Make sure gridops-host-agent run is running.",
  observation_conflicted: "The machine’s reports conflict. Check that only one machine uses this identity.",
  stale_inventory: "The hardware report is out of date. Waiting for a new one.",
  invalid_inventory_receipt: "The hardware report couldn’t be verified.",
};

function ApprovalSection({ host }: { host: HostItem }) {
  const evidence = host.approvalEvidence;
  const targets = useQuery({
    queryKey: ["fleet", "targets"],
    queryFn: ({ signal }) => listTargets(signal),
    enabled: evidence.state === "available",
  });
  const [selected, setSelected] = useState<string[] | null>(null);
  const { pending, run } = useFleetAction();
  // Approval may grant only the targets the enrollment code was issued for.
  const available = (targets.data?.items ?? []).filter((target) => host.enrollmentTargetIds.includes(target.id));
  const values = selected ?? available.map((target) => target.id);

  return (
    <section className="space-y-3">
      <SectionHeading>Approve machine</SectionHeading>
      {evidence.state === "unavailable" ? (
        <Callout title="Can’t approve yet" tone="warning">{unavailableReason[evidence.reason] ?? "Approval isn’t available yet."}</Callout>
      ) : (
        <div className="space-y-3 rounded-lg border border-border bg-panel p-4">
          <p className="text-sm text-muted-foreground">
            Check the hardware report below, then choose where this machine may run jobs.
          </p>
          <SearchableMultiSelect
            ariaLabel="Approve for repositories and organizations"
            emptyMessage="No repositories or organizations found"
            loading={targets.isPending}
            onValueChange={setSelected}
            options={available.map((target) => ({
              value: target.id, label: target.label,
              description: target.kind === "github_organization" ? "Organization" : "Repository",
            }))}
            placeholder="Choose repositories and organizations…"
            searchPlaceholder="Search…"
            selectAll
            selectedNoun="targets"
            values={values}
          />
          {targets.isError ? <Callout title="Couldn’t load repositories" tone="danger">{fleetErrorMessage(targets.error, "")}</Callout> : null}
          <div className="flex justify-end">
            <Button
              disabled={pending !== null || values.length === 0}
              onClick={() => void run("approve", () => approveHost(host.id, {
                expectedRevision: evidence.expectedRevision,
                expectedInventoryRevision: evidence.expectedInventoryRevision,
                expectedInventoryDigest: evidence.expectedInventoryDigest,
                targetIds: values,
              }), "Machine approved. Enable a backend and activate it to start running jobs.")}
            >
              {pending === "approve" ? <LoaderCircle className="animate-spin" /> : <ShieldCheck />}Approve
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

const lifecycleActions: Array<{ value: HostLifecycle; label: string; confirm?: string }> = [
  { value: "active", label: "Activate" },
  { value: "paused", label: "Pause", confirm: "Pause this machine? Running jobs continue; no new jobs start." },
  { value: "draining", label: "Drain", confirm: "Drain this machine? Running jobs finish, then its runners are removed." },
  { value: "maintenance", label: "Maintenance", confirm: "Put this machine in maintenance? No jobs run until you activate it again." },
];

function LifecycleSection({ host }: { host: HostItem }) {
  const { pending, run } = useFleetAction();
  return (
    <section className="space-y-3">
      <SectionHeading>Lifecycle</SectionHeading>
      <div className="flex flex-wrap items-center gap-2" role="group" aria-label="Machine lifecycle">
        {lifecycleActions.map((action) => {
          const current = host.lifecycle === action.value;
          return (
            <Button
              aria-pressed={current}
              disabled={current || pending !== null}
              key={action.value}
              onClick={async () => {
                if (action.confirm && !(await confirmAction(action.confirm, { confirmLabel: action.label }))) return;
                await run(action.value, () => setHostLifecycle(host.id, host.policyRevision, action.value), `Machine set to ${lifecycleLabel[action.value]?.toLowerCase()}.`);
              }}
              size="sm"
              variant={current ? "secondary" : "outline"}
            >
              {pending === action.value ? <LoaderCircle className="animate-spin" /> : null}{action.label}
            </Button>
          );
        })}
      </div>
      <p className="text-xs text-muted-foreground">Currently {lifecycleLabel[host.lifecycle]?.toLowerCase()}. Jobs start only on active machines with an enabled backend.</p>
      <Button
        disabled={pending !== null}
        onClick={async () => {
          if (!(await confirmAction("Retire this machine? Its credential stops working and it gets no more jobs. Drain it first if it’s running any. This can’t be undone; enroll it again to reuse it.", { confirmLabel: "Retire" }))) return;
          await run("retire", () => retireHost(host.id, host.policyRevision), "Machine retired.");
        }}
        size="sm"
        variant="destructive"
      >
        {pending === "retire" ? <LoaderCircle className="animate-spin" /> : null}Retire machine
      </Button>
    </section>
  );
}

/** A quarantined or lost machine gets a one-time recovery code to re-enroll as itself. */
function RecoverSection({ host }: { host: HostItem }) {
  const { pending, run } = useFleetAction();
  const [code, setCode] = useState<{ code: string; expiresAt: string } | null>(null);
  const script = code ? `gridops-host-agent enroll --replace\n# Recovery code: ${code.code}` : "";
  return (
    <section className="space-y-3">
      <SectionHeading>Recover machine</SectionHeading>
      {code ? (
        <Callout title="Copy this code now" tone="warning">
          <p className="mb-2">It’s shown once and expires {formatDateTime(code.expiresAt)}. Run this on the machine, paste the code when asked, then approve the machine again.</p>
          <div className="flex items-start gap-2 rounded-md border border-border bg-panel-subtle p-2">
            <pre className="min-w-0 flex-1 overflow-x-auto font-mono text-xs">{script}</pre>
            <CopyButton label="Recovery commands" size="icon-sm" value={script} variant="ghost" />
          </div>
        </Callout>
      ) : (
        <>
          <p className="text-sm text-muted-foreground">
            {host.integrity === "quarantined"
              ? "This machine was quarantined because its credential was used from two places at once. Recover it to give it a new credential."
              : "If this machine lost its credential or was rebuilt, recover it to re-enroll it as the same machine."}
          </p>
          <Button
            disabled={pending !== null}
            onClick={() => void run("recover", async () => {
              const result = await recoverHost(host.id, host.policyRevision, newIdempotencyKey());
              if (result.status === "created") setCode({ code: result.code, expiresAt: result.expiresAt });
            }, "Recovery code issued.")}
            size="sm"
            variant="outline"
          >
            {pending === "recover" ? <LoaderCircle className="animate-spin" /> : <KeyRound />}Issue recovery code
          </Button>
        </>
      )}
    </section>
  );
}

const logLevelClass: Record<AgentLog["level"], string> = {
  error: "text-destructive",
  warn: "text-amber-600 dark:text-amber-400",
  info: "text-muted-foreground",
  debug: "text-faint",
  trace: "text-faint",
};

/** The newest lines the machine's agent shipped (kept for a day). */
function AgentLogSection({ hostId }: { hostId: string }) {
  const logs = useQuery({
    queryKey: ["fleet", "logs", hostId],
    queryFn: ({ signal }) => listAgentLogs(hostId, signal),
    refetchInterval: REFRESH,
  });
  if (logs.isError && isForbidden(logs.error)) return null;
  const items = logs.data?.items ?? [];
  return (
    <section className="space-y-3">
      <SectionHeading>Agent log</SectionHeading>
      {items.length === 0 ? (
        <p className="text-sm text-muted-foreground">{logs.isPending ? "Loading…" : "No log lines in the last day."}</p>
      ) : (
        <div
          className="max-h-96 overflow-y-auto rounded-lg border border-border bg-panel-subtle p-3 font-mono text-2xs leading-5"
          ref={(element) => { if (element) element.scrollTop = element.scrollHeight; }}
        >
          {items.map((line) => (
            <div className="whitespace-pre-wrap break-words" key={line.id}>
              <span className="text-faint">{new Date(line.loggedAt).toLocaleTimeString()}</span>{" "}
              <span className={cn("font-semibold uppercase", logLevelClass[line.level])}>{line.level}</span>{" "}
              <span className="text-foreground">{line.message}</span>
            </div>
          ))}
        </div>
      )}
      <p className="text-xs text-muted-foreground">The latest {items.length} lines. Logs are kept for a day.</p>
    </section>
  );
}

function GrantsSection({ hostId }: { hostId: string }) {
  const grants = useQuery({
    queryKey: ["fleet", "grants", hostId],
    queryFn: ({ signal }) => listHostGrants(hostId, signal),
    refetchInterval: REFRESH,
  });
  const targets = useQuery({ queryKey: ["fleet", "targets"], queryFn: ({ signal }) => listTargets(signal) });
  const [adding, setAdding] = useState<string[]>([]);
  const { pending, run } = useFleetAction();
  if (grants.isError && isForbidden(grants.error)) return null;

  async function change(grant: HostGrant, field: keyof GrantPolicy, value: boolean) {
    if (value && field === "allowNative" && !(await confirmAction(
      "Run jobs natively on this machine? Jobs run directly on the host with the agent’s access, not in a container or VM.",
      { confirmLabel: "Allow" },
    ))) return;
    if (value && field === "allowDockerSocket" && !(await confirmAction(
      "Share the Docker socket with jobs? Jobs can then control every container on this machine.",
      { confirmLabel: "Allow" },
    ))) return;
    const policy: GrantPolicy = { allowSchedule: grant.allowSchedule, allowNative: grant.allowNative, allowDockerSocket: grant.allowDockerSocket, [field]: value };
    await run(`${grant.targetId}:${field}`, () => setHostGrant(hostId, grant.targetId, policy), "Access updated.");
  }

  const items = grants.data?.items ?? [];
  const granted = new Set(items.map((grant) => grant.targetId));
  const addable = (targets.data?.items ?? []).filter((target) => !granted.has(target.id));

  /** Lets the machine run jobs for more repositories, a few requests at a time. */
  async function addTargets() {
    const ids = adding;
    if (ids.length === 0) return;
    await run("add", async () => {
      for (let start = 0; start < ids.length; start += 6) {
        await Promise.all(ids.slice(start, start + 6).map((targetId) =>
          setHostGrant(hostId, targetId, { allowSchedule: true, allowNative: false, allowDockerSocket: false })));
      }
      setAdding([]);
    }, `Added ${ids.length} ${ids.length === 1 ? "repository" : "repositories"}.`);
  }

  /** Sets one column for every repository, a few requests at a time. */
  async function changeAll(field: keyof GrantPolicy, value: boolean) {
    const targets = items.filter((grant) => grant[field] !== value);
    if (targets.length === 0) return;
    if (value && field === "allowNative" && !(await confirmAction(
      `Run jobs natively on this machine for all ${targets.length} repositories? Jobs run directly on the host with the agent’s access, not in a container or VM.`,
      { confirmLabel: "Allow all" },
    ))) return;
    if (value && field === "allowDockerSocket" && !(await confirmAction(
      `Share the Docker socket with jobs from all ${targets.length} repositories? Jobs can then control every container on this machine.`,
      { confirmLabel: "Allow all" },
    ))) return;
    await run(`all:${field}`, async () => {
      for (let start = 0; start < targets.length; start += 6) {
        await Promise.all(targets.slice(start, start + 6).map((grant) => setHostGrant(hostId, grant.targetId, {
          allowSchedule: grant.allowSchedule, allowNative: grant.allowNative, allowDockerSocket: grant.allowDockerSocket, [field]: value,
        })));
      }
    }, `Updated ${targets.length} ${targets.length === 1 ? "repository" : "repositories"}.`);
  }


  return (
    <section className="space-y-3">
      <SectionHeading>Repositories</SectionHeading>
      {addable.length > 0 ? (
        <div className="flex flex-col gap-2 sm:flex-row sm:items-start">
          <div className="min-w-0 flex-1">
            <SearchableMultiSelect
              ariaLabel="Add repositories and organizations"
              emptyMessage="No repositories or organizations found"
              loading={targets.isPending}
              onValueChange={setAdding}
              options={addable.map((target) => ({
                value: target.id, label: target.label,
                description: target.kind === "github_organization" ? "Organization" : "Repository",
              }))}
              placeholder={`Add repositories (${addable.length} not approved yet)…`}
              searchPlaceholder="Search…"
              selectAll
              selectedNoun="repositories"
              values={adding}
            />
          </div>
          <Button disabled={pending !== null || adding.length === 0} onClick={() => void addTargets()} variant="outline">
            {pending === "add" ? <LoaderCircle className="animate-spin" /> : <Plus />}Add
          </Button>
        </div>
      ) : null}
      {grants.isPending ? <InlineLoading label="Loading repositories…" /> : grants.isError ? (
        <Callout action={<Button onClick={() => void grants.refetch()} size="sm" variant="outline">Retry</Button>} title="Couldn’t load repositories" tone="danger">{fleetErrorMessage(grants.error, "")}</Callout>
      ) : items.length === 0 ? (
        <p className="text-sm text-muted-foreground">This machine isn’t approved for any repositories or organizations.</p>
      ) : (
        <div className="overflow-x-auto rounded-lg border border-border bg-panel">
          <table className="w-full text-sm">
            <thead className="border-b border-border text-left text-xs text-muted-foreground">
              <tr>
                <th className="px-4 py-2 align-top font-medium" scope="col">Repository or organization</th>
                <th className="px-3 py-2 text-center font-medium" scope="col">Run jobs</th>
                <th className="px-3 py-2 text-center font-medium" scope="col">Run natively on the host</th>
                <th className="px-4 py-2 text-center font-medium" scope="col">Share Docker socket</th>
              </tr>
            </thead>
            <tbody>
              <tr className="border-b border-border bg-muted/40">
                <td className="px-4 py-2.5 font-medium text-foreground">All repositories</td>
                {(["allowSchedule", "allowNative", "allowDockerSocket"] as const).map((field) => (
                  <td className="px-3 py-2.5 text-center" key={field}>
                    <Switch
                      aria-label={`${field === "allowSchedule" ? "Run jobs" : field === "allowNative" ? "Run natively on the host" : "Share Docker socket"} for all repositories`}
                      checked={items.every((grant) => grant[field])}
                      disabled={pending !== null}
                      onCheckedChange={(value) => void changeAll(field, value)}
                    />
                  </td>
                ))}
              </tr>
              {items.map((grant) => (
                <tr className="border-b border-border last:border-0" key={grant.targetId}>
                  <td className="px-4 py-2.5 text-foreground">{grant.label}</td>
                  {(["allowSchedule", "allowNative", "allowDockerSocket"] as const).map((field) => (
                    <td className="px-3 py-2.5 text-center" key={field}>
                      <Switch
                        aria-label={`${field === "allowSchedule" ? "Run jobs" : field === "allowNative" ? "Run natively on the host" : "Share Docker socket"} for ${grant.label}`}
                        checked={grant[field]}
                        disabled={pending !== null}
                        onCheckedChange={(value) => void change(grant, field, value)}
                      />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <p className="text-xs text-muted-foreground">The “All repositories” row changes every repository at once. Native profiles need “Run natively on the host”: those jobs run directly on the machine, not in a container or VM.</p>
    </section>
  );
}

const readinessLabel: Record<string, string> = {
  ready: "Ready", reconciling: "Starting", unavailable: "Unavailable", unsupported: "Unsupported", unknown: "Unknown",
};

function BackendsSection({ hostId, items, query }: { hostId: string; items?: BackendReadV1[]; query: { isPending: boolean; isError: boolean; error: unknown; refetch: () => unknown } }) {
  const { pending, run } = useFleetAction();
  return (
    <section className="space-y-3">
      <SectionHeading>Backends</SectionHeading>
      {query.isPending ? <InlineLoading label="Loading backends…" /> : query.isError ? (
        <Callout action={<Button onClick={() => void query.refetch()} size="sm" variant="outline">Retry</Button>} title="Couldn’t load backends" tone="danger">{fleetErrorMessage(query.error, "")}</Callout>
      ) : !items?.length ? (
        <p className="text-sm text-muted-foreground">No backends reported yet. They appear after the agent sends its first hardware report.</p>
      ) : (
        <>
          <div className="overflow-x-auto rounded-lg border border-border bg-panel">
            <table className="w-full text-sm">
              <thead className="border-b border-border text-left text-xs text-muted-foreground">
                <tr>
                  <th className="px-4 py-2 font-medium" scope="col">Runtime</th>
                  <th className="px-3 py-2 font-medium" scope="col">Runs</th>
                  <th className="px-3 py-2 font-medium" scope="col">Status</th>
                  <th className="px-4 py-2 text-right font-medium" scope="col">Enabled</th>
                </tr>
              </thead>
              <tbody>
                {items.map((backend) => (
                  <tr className="border-b border-border last:border-0" key={backend.id}>
                    <td className="px-4 py-2.5 text-foreground">{runtimeLabel[backend.runtimeKind] ?? backend.runtimeKind}</td>
                    <td className="px-3 py-2.5 text-muted-foreground">{osLabel[backend.executionOs]} · {backend.architecture}</td>
                    <td className="px-3 py-2.5">
                      {backend.projection === "host" ? (
                        <div className="flex flex-wrap items-center gap-1">
                          <Badge variant={backend.observedReadiness === "ready" ? "success" : backend.observedReadiness === "reconciling" ? "info" : backend.observedReadiness === "unknown" ? "outline" : "warning"}>
                            {readinessLabel[backend.observedReadiness]}
                          </Badge>
                          {backend.probeFreshness !== "fresh" ? <FreshnessBadge value={backend.probeFreshness} /> : null}
                          {backend.safeReason ? <span className="text-xs text-muted-foreground">{backend.safeReason.replaceAll("_", " ")}</span> : null}
                        </div>
                      ) : <span className="text-faint">—</span>}
                    </td>
                    <td className="px-4 py-2.5 text-right">
                      {backend.projection === "host" ? (
                        <Switch
                          aria-label={`Enable ${runtimeLabel[backend.runtimeKind] ?? backend.runtimeKind} backend`}
                          checked={backend.enabled}
                          disabled={pending !== null}
                          onCheckedChange={(enabled) => void run(backend.id, () => setBackendEnabled(hostId, backend.id, enabled), enabled ? "Backend enabled." : "Backend disabled.")}
                        />
                      ) : <span className="text-faint">—</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="text-xs text-muted-foreground">New backends start disabled. Enable the ones this machine should use.</p>
        </>
      )}
    </section>
  );
}

const eventLabel: Record<HostEventReadV1["kind"], string> = {
  enrollment_issued: "Enrollment code issued",
  enrollment_consumed: "Agent enrolled",
  enrollment_revoked: "Enrollment revoked",
  host_approved: "Machine approved",
  host_recovered: "Machine recovered",
  host_credential_rotated: "Credential rotated",
  host_credential_revoked: "Credential revoked",
  inventory_received: "Hardware report received",
  inventory_conflict: "Conflicting hardware report",
  backend_changed: "Backend changed",
  placement_queued: "Job queued",
  placement_admitted: "Job started",
  placement_changed: "Job changed",
  command_changed: "Command changed",
  other: "Machine updated",
};

function EventsSection({ items, query }: { items?: HostEventReadV1[]; query: { isPending: boolean; isError: boolean; error: unknown } }) {
  const recent = [...(items ?? [])].sort((a, b) => b.occurredAt.localeCompare(a.occurredAt)).slice(0, 10);
  return (
    <section className="space-y-3">
      <SectionHeading>Recent events</SectionHeading>
      {query.isPending ? <InlineLoading label="Loading events…" /> : query.isError ? (
        <Callout title="Couldn’t load events" tone="danger">{fleetErrorMessage(query.error, "")}</Callout>
      ) : recent.length === 0 ? (
        <p className="text-sm text-muted-foreground">No events yet.</p>
      ) : (
        <ol className="divide-y divide-border rounded-lg border border-border bg-panel">
          {recent.map((event) => (
            <li className={cn("flex items-center justify-between gap-3 px-4 py-2.5 text-sm", event.kind === "inventory_conflict" && "text-danger")} key={event.id}>
              <span className="min-w-0 truncate">{eventLabel[event.kind]}</span>
              <time className="shrink-0 text-xs text-muted-foreground" dateTime={event.occurredAt} title={formatDateTime(event.occurredAt)}>{formatRelativeTime(event.occurredAt)}</time>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function MeasurementGroup({ host }: { host: HostItem }) {
  const measurement = host.latestRootMeasurement;
  if (!measurement) {
    return <PropertyGroup title="Latest measurement"><PropertyRow label="Status">None yet</PropertyRow></PropertyGroup>;
  }
  const value = (input: number | null, format: (value: number) => string) => input === null ? "Unknown" : format(input);
  return (
    <PropertyGroup title="Latest measurement">
      <PropertyRow label="Taken" title={measurement.observedAt}>{formatRelativeTime(measurement.observedAt)}{measurement.coverage !== "complete" ? <Badge variant="outline">{measurement.coverage}</Badge> : null}</PropertyRow>
      <PropertyRow label="CPU used">{value(measurement.cpuUsedMillis, formatCores)}</PropertyRow>
      <PropertyRow label="Memory used">{value(measurement.memoryUsedMib, formatMib)}</PropertyRow>
      <PropertyRow label="Disk free">{value(measurement.diskFreeBytes, formatBytes)}</PropertyRow>
      <PropertyRow label="Uptime">{value(measurement.uptimeSeconds, (seconds) => seconds >= 86_400 ? `${Math.floor(seconds / 86_400)}d` : `${Math.floor(seconds / 3600)}h`)}</PropertyRow>
    </PropertyGroup>
  );
}

function RotateCredentialButton({ hostId, generation }: { hostId: string; generation: number }) {
  const { pending, run } = useFleetAction();
  return (
    <Button
      aria-label="Rotate credential"
      disabled={pending !== null}
      onClick={async () => {
        if (!(await confirmAction("Rotate this machine’s credential? The agent switches to a new credential on its next check-in.", { confirmLabel: "Rotate" }))) return;
        await run("rotate", () => rotateHostCredential(hostId, generation, newIdempotencyKey()), "Credential rotation requested.");
      }}
      size="sm"
      variant="outline"
    >
      {pending === "rotate" ? <LoaderCircle className="animate-spin" /> : <KeyRound />}<span className="hidden sm:inline">Rotate credential</span>
    </Button>
  );
}
