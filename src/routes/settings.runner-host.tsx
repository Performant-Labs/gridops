import { createFileRoute, getRouteApi, useRouter } from "@tanstack/react-router";
import { DatabaseBackup, Server } from "lucide-react";

import { Callout, EmptyState } from "~/components/page";
import { SettingsLayout, SettingsSection, SettingsValue } from "~/components/settings-ui";
import { StatusBadge } from "~/components/status-icon";
import { buttonVariants } from "~/components/ui/button";
import { BuiltinHostSection } from "~/features/runner-pools/builtin-host-section";
import { showsBuiltinGuardrails } from "~/features/runner-pools/builtin-host";
import { cn } from "~/lib/utils";

export const Route = createFileRoute("/settings/runner-host")({
  component: RunnerHostSettings,
});

const settingsRoute = getRouteApi("/settings");

function RunnerHostSettings() {
  const page = settingsRoute.useLoaderData();
  const router = useRouter();
  if (!page.data) return <EmptyState description="Sign in to view runner host health." icon={Server} title="Runner host unavailable" />;
  const { manager, configuration, user, settings } = page.data;
  const hostOn = showsBuiltinGuardrails(settings.builtinRunnerHostEnabled);
  const capacity = manager.capacity;
  const used = capacity ? {
    runners: capacity.active.activeRunners + capacity.reserved.activeRunners,
    cpu: capacity.active.cpu + capacity.reserved.cpu,
    memoryMb: capacity.active.memoryMb + capacity.reserved.memoryMb,
  } : null;
  const disk = manager.disk;

  return (
    <SettingsLayout
      actions={user.role === "admin" ? <a className={buttonVariants({ size: "sm", variant: "outline" })} href="/api/backups/database"><DatabaseBackup />Download backup</a> : null}
      description={hostOn
        ? "The runner manager is the only service with Docker access; it admits each runner against these limits."
        : "The built-in runner host is off: GridOps starts no runner on the machine it runs on, and every pool runs on the enrolled machines under Fleet."}
      title="Runner host"
    >
      <BuiltinHostSection
        dockerAvailable={manager.builtinHost?.dockerAvailable ?? true}
        enabled={hostOn}
        isAdmin={user.role === "admin"}
        managerSetting={manager.ok ? manager.builtinHost?.enabled : undefined}
        onChanged={() => router.invalidate()}
      />
      {!manager.ok ? <Callout title="The runner manager is unavailable" tone="danger">{manager.error ?? "GridOps cannot reach the manager service."}</Callout> : null}

      <SettingsSection title="Status">
        <SettingsValue label="Manager" value={<StatusBadge status={manager.ok ? "healthy" : "offline"} />} />
        {manager.builtinHost && !manager.builtinHost.dockerAvailable ? null : (
          <>
            <SettingsValue label="Docker Engine" mono={Boolean(manager.dockerVersion)} value={manager.dockerVersion ?? "—"} />
            <SettingsValue label="Docker API" mono={Boolean(manager.apiVersion)} value={manager.apiVersion ?? "—"} />
            <SettingsValue label="Machine" value={manager.hostName || "—"} />
            <SettingsValue label="Host" value={manager.availableCpus ? `${manager.availableCpus} CPUs · ${manager.totalMemoryMb ? `${Math.round(manager.totalMemoryMb / 1024)} GB memory` : "memory unknown"}` : "—"} />
          </>
        )}
        <SettingsValue label="Provisioning" value={<StatusBadge status={manager.provisioningPaused ? "paused" : "active"} />} />
      </SettingsSection>

      {hostOn && capacity && used ? (
        <SettingsSection description="Includes short-lived reservations for runners that are starting." title="Capacity">
          <Meter label="Runners" limit={capacity.maxRunners} used={used.runners} />
          <Meter label="CPU" limit={capacity.cpuBudget} unit="cores" used={Number(used.cpu.toFixed(1))} />
          <Meter format={(value) => `${(value / 1024).toFixed(1)}`} label="Memory" limit={capacity.memoryBudgetMb} unit="GB" used={used.memoryMb} />
        </SettingsSection>
      ) : null}

      {hostOn && disk ? (
        <SettingsSection description={`Provisioning stops when free space drops below ${Math.round(disk.minimumFreeMb / 1024)} GB.`} title="Disk">
          <Meter format={(value) => `${Math.round(value / 1024)}`} label="Used" limit={disk.totalMb} unit="GB" used={disk.totalMb - disk.availableMb} warnAt={(disk.totalMb - disk.minimumFreeMb) / disk.totalMb} />
          <SettingsValue label="Free" value={`${Math.round(disk.availableMb / 1024)} GB`} />
        </SettingsSection>
      ) : null}

      <SettingsSection title="Control plane">
        <SettingsValue label="GitHub control token" value={configuration.installationTokens ? "Installation token" : "User token fallback"} />
        <SettingsValue label="Database" value="SQLite · WAL mode" />
        <SettingsValue label="Signed in as" value={`@${user.login} · ${user.role}`} />
      </SettingsSection>
    </SettingsLayout>
  );
}

function Meter({ label, used, limit, unit, format = (value) => String(value), warnAt = 0.85 }: { label: string; used: number; limit: number; unit?: string; format?: (value: number) => string; warnAt?: number }) {
  const ratio = limit > 0 ? Math.min(1, used / limit) : 0;
  const tone = ratio >= 1 ? "bg-danger" : ratio >= warnAt ? "bg-warning" : "bg-primary";
  return (
    <div className="flex items-center gap-4 px-4 py-3 text-sm">
      <span className="w-20 shrink-0 text-muted-foreground">{label}</span>
      <span className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-selected">
        <span className={cn("absolute inset-y-0 left-0 rounded-full", tone)} style={{ width: `${ratio * 100}%` }} />
      </span>
      <span className="tabular w-32 shrink-0 text-right text-foreground">{format(used)} <span className="text-faint">/ {format(limit)}{unit ? ` ${unit}` : ""}</span></span>
    </div>
  );
}
