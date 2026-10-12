/** Pool settings: run this pool's runners on enrolled machines instead of the built-in runner host. */
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { LoaderCircle } from "lucide-react";
import { type FormEvent, type ReactNode, useState } from "react";
import { createPortal } from "react-dom";
import { toast } from "sonner";

import { Callout, InlineLoading } from "~/components/page";
import { SettingsRow, SettingsSection, Switch, UnitField } from "~/components/settings-ui";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { SearchableSelect } from "~/components/ui/searchable-select";
import { getPoolProfile, getPoolQueue, savePoolProfile } from "./admin-api";
import type { Profile, ProfileInput } from "./admin-contracts";
import { fleetErrorMessage, isForbidden } from "./fleet-ui";

type Runtime = ProfileInput["runtimeKind"];
type Os = ProfileInput["executionOs"];

const FORM_ID = "fleet-profile-form";
const defaultImage: Record<Runtime, string> = {
  docker: "ghcr.io/actions/actions-runner:latest",
  native_process: "native:actions-runner",
  tart_vm: "",
};
const osOptions: Record<Runtime, Os[]> = {
  docker: ["linux"],
  native_process: ["linux", "macos", "windows"],
  tart_vm: ["macos"],
};
const osLabels: Record<Os, string> = { linux: "Linux", macos: "macOS", windows: "Windows" };
const GIB = 1024 ** 3;

const defaults: ProfileInput = {
  enabled: false, runtimeKind: "docker", executionOs: "linux", architecture: "arm64", mode: "ephemeral",
  image: defaultImage.docker, labels: [], cpuMillis: 2000, memoryMib: 2048, diskBytes: 0, requiresDockerSocket: false,
};

export function FleetExecutionSection({ poolId }: { poolId: string }) {
  const profile = useQuery({ queryKey: ["fleet", "profile", poolId], queryFn: ({ signal }) => getPoolProfile(poolId, signal) });
  const description = <>When on, this pool’s runners run on your <Link className="underline underline-offset-2" to="/fleet">machines</Link> instead of the built-in runner host.</>;

  if (profile.isPending) return <SettingsSection description={description} title="Fleet execution"><InlineLoading label="Loading…" /></SettingsSection>;
  if (profile.isError) {
    return (
      <SettingsSection description={description} title="Fleet execution">
        <div className="px-4 py-3">
          {isForbidden(profile.error) ? (
            <p className="text-sm text-muted-foreground">Only system administrators can change where this pool runs.</p>
          ) : (
            <Callout action={<Button onClick={() => void profile.refetch()} size="sm" variant="outline">Retry</Button>} title="Couldn’t load fleet settings" tone="danger">
              {fleetErrorMessage(profile.error, "")}
            </Callout>
          )}
        </div>
      </SettingsSection>
    );
  }
  const item = profile.data.item;
  return <ProfileEditor description={description} key={item ? `${item.id}:${item.revision}` : "new"} poolId={poolId} profile={item} />;
}

const waitingReason: Record<string, string> = {
  no_active_machine: "No machine is approved and active.",
  machines_offline: "No active machine is online.",
  target_not_granted: "No online machine is approved for this repository.",
  no_matching_backend: "No approved machine has a runtime of this kind, OS and architecture.",
  backend_disabled: "The matching runtime is switched off on every machine.",
  native_not_trusted: "No machine allows native runs for this repository.",
  machines_under_pressure: "Every matching machine is under memory or CPU pressure.",
  physical_resource_exhausted: "Matching machines don’t have enough free CPU, memory or disk.",
  child_resource_exhausted: "The machine’s VM or container budget is full.",
  pool_capacity_exhausted: "The pool is at its maximum runner count.",
  image_unavailable: "No machine allows this image. Add it to GRIDOPS_AGENT_IMAGES on the machine.",
  capability_unverified: "The machine hasn’t confirmed this runtime recently.",
  unsupported_mode: "The machine’s runtime doesn’t support this runner mode.",
};

/** Runners this pool queued that no machine has taken yet, and why. */
function WaitingRunners({ poolId }: { poolId: string }) {
  const queue = useQuery({
    queryKey: ["fleet", "queue", poolId],
    queryFn: ({ signal }) => getPoolQueue(poolId, signal),
    refetchInterval: 10_000,
  });
  const items = queue.data?.items ?? [];
  if (items.length === 0) return null;
  const reason = items.find((item) => item.reason)?.reason;
  return (
    <div className="px-4 py-3">
      <Callout title={`${items.length} ${items.length === 1 ? "runner is" : "runners are"} waiting for a machine`} tone="warning">
        {(reason && waitingReason[reason]) ?? "No machine can take it yet."}{" "}
        <Link className="underline underline-offset-2" to="/fleet">Check machines</Link>
      </Callout>
    </div>
  );
}

function ProfileEditor({ poolId, profile, description }: { poolId: string; profile: Profile | null; description: ReactNode }) {
  const queryClient = useQueryClient();
  const initial = profile ?? defaults;
  const [enabled, setEnabled] = useState(initial.enabled);
  const [runtimeKind, setRuntimeKind] = useState<Runtime>(initial.runtimeKind);
  const [executionOs, setExecutionOs] = useState<Os>(initial.executionOs);
  const [architecture, setArchitecture] = useState(initial.architecture);
  const [mode, setMode] = useState(initial.mode);
  const [image, setImage] = useState(initial.image);
  const [labels, setLabels] = useState(initial.labels.join(", "));
  const [cores, setCores] = useState(initial.cpuMillis / 1000);
  const [memoryMib, setMemoryMib] = useState(initial.memoryMib);
  const [diskGib, setDiskGib] = useState(initial.diskBytes / GIB);
  const [dockerSocket, setDockerSocket] = useState(initial.requiresDockerSocket ?? false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function changeRuntime(next: Runtime) {
    setRuntimeKind(next);
    if (!osOptions[next].includes(executionOs)) setExecutionOs(osOptions[next][0] ?? "linux");
    if (next === "native_process") setMode("ephemeral");
    // Swap the image only when it still holds the previous runtime's default.
    if (image.trim() === "" || image === defaultImage[runtimeKind]) setImage(defaultImage[next]);
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    // The form is portaled out of the pool form; keep its submit from reaching that one.
    event.preventDefault();
    event.stopPropagation();
    setSaving(true);
    setError(null);
    try {
      const saved = await savePoolProfile(poolId, {
        enabled, runtimeKind, executionOs, architecture, mode, image: image.trim(),
        labels: labels.split(",").map((label) => label.trim()).filter(Boolean),
        cpuMillis: Math.round(cores * 1000), memoryMib, diskBytes: Math.round(diskGib * GIB),
        requiresDockerSocket: runtimeKind === "docker" ? dockerSocket : false,
      });
      queryClient.setQueryData(["fleet", "profile", poolId], saved);
      toast.success(saved.item.enabled ? "Fleet execution saved." : "Fleet execution saved and turned off.");
    } catch (cause) {
      setError(fleetErrorMessage(cause, "Fleet settings could not be saved."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <SettingsSection
      actions={<Button disabled={saving} form={FORM_ID} size="sm" type="submit">{saving ? <LoaderCircle className="animate-spin" /> : null}Save</Button>}
      description={description}
      title="Fleet execution"
    >
      {createPortal(<form aria-label="Fleet execution" id={FORM_ID} onSubmit={(event) => void submit(event)} />, document.body)}
      <SettingsRow
        description={profile ? `Runs for ${profile.targetIds.length} ${profile.targetIds.length === 1 ? "repository or organization" : "repositories and organizations"}. Machines must be approved for them.` : "Saved settings take effect on the next runner."}
        inline
        label="Run on machines"
      >
        <Switch aria-label="Run on machines" checked={enabled} onCheckedChange={setEnabled} />
      </SettingsRow>
      {profile?.enabled ? <WaitingRunners poolId={poolId} /> : null}
      <SettingsRow description={runtimeKind === "native_process" ? "Jobs run directly on the machine. The machine must allow native runs for this repository." : "How each runner is isolated on the machine."} label="Runtime">
        <SearchableSelect
          ariaLabel="Fleet runtime"
          onValueChange={(next) => next && changeRuntime(next)}
          options={[
            { value: "docker", label: "Docker container", description: "Linux containers" },
            { value: "native_process", label: "Native process", description: "Directly on a Linux, macOS or Windows machine" },
            { value: "tart_vm", label: "Tart VM", description: "macOS virtual machines" },
          ]}
          searchable={false}
          value={runtimeKind}
        />
      </SettingsRow>
      <SettingsRow label="Operating system">
        <SearchableSelect
          ariaLabel="Fleet operating system"
          onValueChange={(next) => next && setExecutionOs(next)}
          options={osOptions[runtimeKind].map((os) => ({ value: os, label: osLabels[os] }))}
          searchable={false}
          value={executionOs}
        />
      </SettingsRow>
      <SettingsRow label="Architecture">
        <SearchableSelect
          ariaLabel="Fleet architecture"
          onValueChange={(next) => next && setArchitecture(next)}
          options={[{ value: "arm64", label: "arm64" }, { value: "x64", label: "x64" }]}
          searchable={false}
          value={architecture}
        />
      </SettingsRow>
      <SettingsRow description={runtimeKind === "native_process" ? "Native runners are always ephemeral." : undefined} label="Mode">
        <SearchableSelect
          ariaLabel="Fleet runner mode"
          onValueChange={(next) => next && setMode(next)}
          options={runtimeKind === "native_process"
            ? [{ value: "ephemeral", label: "Ephemeral", description: "One clean runner per job" }]
            : [
              { value: "ephemeral", label: "Ephemeral", description: "One clean runner per job" },
              { value: "persistent", label: "Persistent", description: "Reuse the runner across jobs" },
            ]}
          searchable={false}
          value={mode}
        />
      </SettingsRow>
      <SettingsRow
        description={runtimeKind === "native_process" ? "Native runners don’t use a container image." : runtimeKind === "docker" ? "The machine must allow this image in GRIDOPS_AGENT_IMAGES." : "The Tart VM image each runner clones."}
        htmlFor="fleet-image"
        label="Image"
      >
        <Input className="font-mono text-xs" form={FORM_ID} id="fleet-image" onChange={(event) => setImage(event.target.value)} required value={image} />
      </SettingsRow>
      <SettingsRow description="Comma-separated runner labels." htmlFor="fleet-labels" label="Labels">
        <Input form={FORM_ID} id="fleet-labels" onChange={(event) => setLabels(event.target.value)} placeholder="self-hosted, arm64" value={labels} />
      </SettingsRow>
      <SettingsRow description="Reserved on the machine for each runner." htmlFor="fleet-cpu" label="CPU per runner">
        <UnitField unit="cores"><Input className="pr-12" form={FORM_ID} id="fleet-cpu" min="0.25" onChange={(event) => setCores(Number(event.target.value))} required step="0.25" type="number" value={cores} /></UnitField>
      </SettingsRow>
      <SettingsRow htmlFor="fleet-memory" label="Memory per runner">
        <UnitField unit="MB"><Input className="pr-10" form={FORM_ID} id="fleet-memory" min="256" onChange={(event) => setMemoryMib(Number(event.target.value))} required step="256" type="number" value={memoryMib} /></UnitField>
      </SettingsRow>
      <SettingsRow description="0 reserves no disk." htmlFor="fleet-disk" label="Disk per runner">
        <UnitField unit="GB"><Input className="pr-10" form={FORM_ID} id="fleet-disk" min="0" onChange={(event) => setDiskGib(Number(event.target.value))} required step="1" type="number" value={diskGib} /></UnitField>
      </SettingsRow>
      {runtimeKind === "docker" ? (
        <SettingsRow description="Jobs that build containers need it. The machine must allow it for the repository." inline label="Docker socket">
          <Switch aria-label="Give jobs the Docker socket" checked={dockerSocket} onCheckedChange={setDockerSocket} />
        </SettingsRow>
      ) : null}
      {error ? <div className="px-4 py-3"><Callout title="Could not save" tone="danger">{error}</Callout></div> : null}
    </SettingsSection>
  );
}
