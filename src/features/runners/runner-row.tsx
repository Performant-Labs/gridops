import { Link } from "@tanstack/react-router";
import { MoreHorizontal, Pause, Play, RefreshCw, RotateCcw, Server, Square, Terminal, Trash2 } from "lucide-react";

import { listRowClassName, rowActionClassName } from "~/components/page";
import { StatusBadge, statusLabel } from "~/components/status-icon";
import { Badge } from "~/components/ui/badge";
import { Button, buttonVariants } from "~/components/ui/button";
import { Dialog, DialogTrigger, SheetContent } from "~/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "~/components/ui/dropdown-menu";
import { Tooltip } from "~/components/ui/tooltip";
import type { Runner, RunnerMachine } from "~/features/operations/operations.functions";
import { runnerAction } from "~/features/runner-pools/runner-pools.functions";
import { useAction } from "~/lib/use-action";
import { cn, formatAge } from "~/lib/utils";

const osLabel = { linux: "Linux", macos: "macOS", windows: "Windows" } as const;

/** "Windows · x64 · native" for a fleet runner, "macOS · arm64" for a Tart VM, and so on. */
export function runnerPlatformLabel(runner: Runner) {
  const machine = runner.machine;
  const os = machine?.os ? osLabel[machine.os] : runner.provider === "tart" ? "macOS" : "Linux";
  const runtime = machine?.runtime === "native_process" ? " · native" : "";
  return `${os} · ${runner.architecture.toLowerCase()}${runtime}`;
}

/** Links to the machine that runs a fleet runner. */
export function MachineLink({ className, machine }: { className?: string; machine: RunnerMachine }) {
  return (
    <Link className={cn("relative truncate rounded-md px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-selected hover:text-foreground", className)} params={{ hostId: machine.id }} title={`Runs on ${machine.name ?? "an enrolled machine"}`} to="/fleet/machines/$hostId">
      <Server className="mr-1 inline size-3 align-[-2px]" />{machine.name ?? "Machine"}
    </Link>
  );
}

/** Opens a side panel with what the runner is, where it runs and what it's doing. */
function RunnerDetails({ runner }: { runner: Runner }) {
  const state = runnerState(runner);
  const rows: Array<[string, React.ReactNode]> = [
    ["Status", statusLabel(state)],
    ["Platform", runnerPlatformLabel(runner)],
    ["Runs on", runner.machine ? <MachineLink className="-ml-1.5" machine={runner.machine} /> : "The GridOps host"],
    ["Pool", <Link className="underline-offset-2 hover:underline" params={{ poolId: runner.poolId }} to="/runner-pools/$poolId">{runner.poolName}</Link>],
    ["Repository", runner.repository ?? runner.accountLogin],
    ["Current job", runner.currentJobName
      ? runner.currentRunId
        ? <Link className="underline-offset-2 hover:underline" params={{ runId: String(runner.currentRunId) }} to="/workflow-runs/$runId">{runner.currentJobName}</Link>
        : runner.currentJobName
      : "None"],
    ["Mode", runner.ephemeral ? "Ephemeral" : "Persistent"],
    ["Created", new Date(runner.createdAt).toLocaleString()],
    ["Registered", runner.registeredAt ? new Date(runner.registeredAt).toLocaleString() : "Not yet"],
    ["Last heartbeat", runner.lastHeartbeatAt ? `${formatAge(runner.lastHeartbeatAt)} ago` : "None yet"],
  ];
  return (
    <Dialog>
      <DialogTrigger asChild>
        <button aria-label={`Show details for ${runner.name}`} className="absolute inset-0 outline-none" data-list-row="" type="button" />
      </DialogTrigger>
      <SheetContent
        actions={<Link className={buttonVariants({ size: "sm", variant: "outline" })} search={{ target: runner.id }} to="/live-logs"><Terminal />View logs</Link>}
        className="max-w-md"
        description={runner.failureReason ?? undefined}
        heading={<span className="font-mono">{runner.name}</span>}
      >
        <dl className="grid grid-cols-[8rem_1fr] gap-x-4 gap-y-2.5 overflow-y-auto px-5 py-4 text-sm">
          {rows.map(([label, value]) => (
            <div className="contents" key={label}>
              <dt className="text-muted-foreground">{label}</dt>
              <dd className="min-w-0 truncate text-foreground">{value}</dd>
            </div>
          ))}
        </dl>
      </SheetContent>
    </Dialog>
  );
}

export function runnerState(runner: Runner) {
  return runner.busy ? "busy" : runner.status;
}

export function RunnerRow({ runner, showPool = true }: { runner: Runner; showPool?: boolean }) {
  const state = runnerState(runner);
  return (
    <div className={cn(listRowClassName, "relative pr-2")}>
      <RunnerDetails runner={runner} />
      <Tooltip content={statusLabel(state)}><span className="relative inline-flex"><StatusBadge iconOnly status={state} /></span></Tooltip>
      <span className="min-w-0 shrink">
        <span className="block truncate font-mono text-xs font-medium text-foreground">{runner.name}</span>
        {/* The job and repository columns appear at wider breakpoints; below them they sit under the name. */}
        <span className="block truncate text-2xs text-muted-foreground md:hidden">{runner.currentJobName ?? runner.repository ?? runner.accountLogin}</span>
      </span>
      <Badge className="hidden sm:inline-flex" dot={runner.provider === "tart" || runner.machine?.os === "macos" ? "bg-[#bb87fc]" : runner.machine?.os === "windows" ? "bg-warning" : "bg-info"}>{runnerPlatformLabel(runner)}</Badge>
      {runner.ephemeral ? null : <Badge className="hidden md:inline-flex" variant="outline">persistent</Badge>}
      {runner.currentJobName ? (
        runner.currentRunId ? (
          <Link className="relative hidden min-w-0 max-w-64 items-center gap-1.5 truncate rounded-full border border-info/30 bg-info/10 px-2 py-0.5 text-2xs font-medium text-info hover:bg-info/15 md:inline-flex" params={{ runId: String(runner.currentRunId) }} to="/workflow-runs/$runId">
            <span className="size-1.5 shrink-0 rounded-full bg-info live-pulse" /><span className="truncate">{runner.currentJobName}</span>
          </Link>
        ) : <Badge className="hidden md:inline-flex" dot="bg-info">{runner.currentJobName}</Badge>
      ) : null}
      {runner.failureReason ? <span className="hidden min-w-0 max-w-72 truncate text-xs text-danger lg:block" title={runner.failureReason}>{runner.failureReason}</span> : null}
      <span className="flex-1" />
      {runner.machine ? <MachineLink className="hidden max-w-40 sm:block" machine={runner.machine} /> : null}
      {showPool ? (
        <Link className="relative hidden max-w-40 truncate rounded-md px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-selected hover:text-foreground lg:block" params={{ poolId: runner.poolId }} to="/runner-pools/$poolId">{runner.poolName}</Link>
      ) : null}
      <span className="hidden w-36 shrink-0 truncate text-right text-xs text-faint xl:block" title={runner.repository ?? runner.accountLogin}>{runner.repository ?? runner.accountLogin}</span>
      <Tooltip content={runner.lastHeartbeatAt ? `Last heartbeat ${new Date(runner.lastHeartbeatAt).toLocaleString()}` : "No heartbeat yet"}>
        <span className="tabular relative w-10 shrink-0 text-right text-xs text-faint">{runner.lastHeartbeatAt ? formatAge(runner.lastHeartbeatAt) : "—"}</span>
      </Tooltip>
      <span className="relative flex w-7 shrink-0 justify-end">{runner.canManage ? <RunnerActionsMenu runner={runner} /> : null}</span>
    </div>
  );
}

export function RunnerActionsMenu({ runner }: { runner: Runner }) {
  const run = useAction();
  const control = runnerAction;
  const platform = runner.platform === "bitbucket" ? "Bitbucket" : "GitHub";
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button aria-label={`Actions for ${runner.name}`} className={rowActionClassName} size="icon-xs" variant="ghost"><MoreHorizontal /></Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="w-52">
        <DropdownMenuItem asChild icon={<Terminal />}><Link search={{ target: runner.id }} to="/live-logs">View logs</Link></DropdownMenuItem>
        <DropdownMenuSeparator />
        {runner.status === "paused" ? (
          <DropdownMenuItem icon={<Play />} onSelect={() => void run({ action: () => control({ data: { runnerId: runner.id, action: "resume" } }), success: `${runner.name} resumed.` })}>Resume</DropdownMenuItem>
        ) : runner.status === "stopped" && !runner.ephemeral ? (
          <DropdownMenuItem icon={<Play />} onSelect={() => void run({ action: () => control({ data: { runnerId: runner.id, action: "start" } }), success: `${runner.name} started.` })}>Start</DropdownMenuItem>
        ) : runner.status !== "stopped" ? (
          <DropdownMenuItem disabled={runner.busy || !runner.containerId} icon={<Pause />} onSelect={() => void run({ action: () => control({ data: { runnerId: runner.id, action: "pause" } }), success: `${runner.name} paused.` })}>Pause</DropdownMenuItem>
        ) : null}
        <DropdownMenuItem disabled={!runner.containerId || runner.status === "stopped"} icon={<Square />} onSelect={() => void run({ action: () => control({ data: { runnerId: runner.id, action: "stop" } }), confirm: `Stop ${runner.name}?`, success: `${runner.name} stopped.` })}>Stop</DropdownMenuItem>
        <DropdownMenuItem
          disabled={runner.ephemeral || !runner.containerId || runner.status === "stopped"}
          icon={<RotateCcw />}
          onSelect={() => void run({ action: () => control({ data: { runnerId: runner.id, action: "restart" } }), confirm: runner.busy ? `${runner.name} is busy. Restart it and interrupt the current job?` : undefined, success: `${runner.name} restarted.` })}
        >
          Restart
        </DropdownMenuItem>
        <DropdownMenuItem disabled={runner.busy} icon={<RefreshCw />} onSelect={() => void run({ action: () => control({ data: { runnerId: runner.id, action: "rebuild" } }), confirm: `Rebuild ${runner.name}? GridOps replaces it with a newly registered runner.`, success: `${runner.name} rebuilt.` })}>Rebuild</DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem destructive icon={<Trash2 />} onSelect={() => void run({ action: () => control({ data: { runnerId: runner.id, action: "delete" } }), confirm: `Delete ${runner.name} from its provider and ${platform}?`, success: `${runner.name} deleted.` })}>Delete runner</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const RUNNER_GROUPS = [
  { key: "busy", label: "Running jobs", status: "busy" },
  { key: "failed", label: "Failed", status: "failed" },
  { key: "starting", label: "Starting", status: "starting" },
  { key: "idle", label: "Idle", status: "idle" },
  { key: "stopped", label: "Paused or stopped", status: "paused" },
] as const;

function runnerGroup(runner: Runner): (typeof RUNNER_GROUPS)[number]["key"] {
  if (runner.busy) return "busy";
  const status = runner.status.toLowerCase();
  if (["failed", "error", "dead"].includes(status)) return "failed";
  if (["starting", "provisioning", "registering", "created"].includes(status)) return "starting";
  if (["paused", "stopped", "draining", "offline"].includes(status)) return "stopped";
  return "idle";
}

export function groupRunners(runners: Runner[]) {
  return RUNNER_GROUPS.map((group) => ({ ...group, runners: runners.filter((runner) => runnerGroup(runner) === group.key) })).filter((group) => group.runners.length);
}
