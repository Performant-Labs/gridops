/** Pool settings: run this pool's runners on enrolled machines instead of the built-in runner host. */
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { LoaderCircle, Plus, Trash2 } from "lucide-react";
import { type FormEvent, type ReactNode, useState } from "react";
import { createPortal } from "react-dom";
import { toast } from "sonner";

import { Callout, InlineLoading } from "~/components/page";
import { SettingsRow, SettingsSection, Switch, UnitField } from "~/components/settings-ui";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { SearchableSelect } from "~/components/ui/searchable-select";
import { getPoolProfile, getPoolQueue, savePoolProfile } from "./admin-api";
import type { PlatformInput, Profile, ProfileInput } from "./admin-contracts";
import { fleetErrorMessage, isForbidden } from "./fleet-ui";

type Runtime = PlatformInput["runtimeKind"];
type Os = PlatformInput["executionOs"];

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

const runtimeLabels: Record<Runtime, string> = { docker: "Docker", native_process: "native", tart_vm: "Tart VM" };

const newPlatform: PlatformInput = {
  runtimeKind: "docker", executionOs: "linux", architecture: "arm64", image: defaultImage.docker, requiresDockerSocket: false,
};
/** A sensible next platform: Linux on the other architecture, else Windows x64. */
function nextPlatform(current: PlatformInput[]): PlatformInput {
  const taken = (platform: PlatformInput) => current.some((item) =>
    item.runtimeKind === platform.runtimeKind && item.executionOs === platform.executionOs && item.architecture === platform.architecture);
  const candidates: PlatformInput[] = [
    newPlatform,
    { ...newPlatform, architecture: "x64" },
    { runtimeKind: "native_process", executionOs: "windows", architecture: "x64", image: defaultImage.native_process, requiresDockerSocket: false },
    { runtimeKind: "native_process", executionOs: "macos", architecture: "arm64", image: defaultImage.native_process, requiresDockerSocket: false },
  ];
  return candidates.find((platform) => !taken(platform)) ?? newPlatform;
}

const defaults: ProfileInput = {
  enabled: false, placement: "machines", mode: "ephemeral", cpuMillis: 2000, memoryMib: 2048, diskBytes: 0, platforms: [],
};

export function FleetExecutionSection({ poolId }: { poolId: string }) {
  const profile = useQuery({ queryKey: ["fleet", "profile", poolId], queryFn: ({ signal }) => getPoolProfile(poolId, signal) });
  const description = <>When on, this pool’s runners can run on your <Link className="underline underline-offset-2" to="/fleet">machines</Link>. Placement decides whether the built-in runner host is still used.</>;

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
  return <ProfileEditor description={description} key={item ? item.platforms.map((platform) => `${platform.id}:${platform.revision}`).join(",") : "new"} poolId={poolId} profile={item} />;
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

type PlatformDraft = PlatformInput & { key: string };

let nextKey = 0;
const draftKey = () => `platform-${++nextKey}`;
const platformName = (platform: PlatformInput) =>
  `${osLabels[platform.executionOs]} ${platform.architecture} · ${runtimeLabels[platform.runtimeKind]}`;

function ProfileEditor({ poolId, profile, description }: { poolId: string; profile: Profile | null; description: ReactNode }) {
  const queryClient = useQueryClient();
  const initial = profile ?? defaults;
  const [enabled, setEnabled] = useState(initial.enabled);
  const [mode, setMode] = useState(initial.mode);
  const [cores, setCores] = useState(initial.cpuMillis / 1000);
  const [memoryMib, setMemoryMib] = useState(initial.memoryMib);
  const [diskGib, setDiskGib] = useState(initial.diskBytes / GIB);
  const [placement, setPlacement] = useState(initial.placement);
  const [platforms, setPlatforms] = useState<PlatformDraft[]>(() =>
    (profile?.platforms ?? [newPlatform]).map((platform) => ({ ...platform, key: draftKey() })));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const native = platforms.some((platform) => platform.runtimeKind === "native_process");

  function update(key: string, change: (platform: PlatformDraft) => PlatformDraft) {
    setPlatforms((current) => current.map((platform) => (platform.key === key ? change(platform) : platform)));
  }

  function changeRuntime(platform: PlatformDraft, next: Runtime): PlatformDraft {
    return {
      ...platform,
      runtimeKind: next,
      executionOs: osOptions[next].includes(platform.executionOs) ? platform.executionOs : osOptions[next][0] ?? "linux",
      // Swap the image only when it still holds the previous runtime's default.
      image: platform.image.trim() === "" || platform.image === defaultImage[platform.runtimeKind] ? defaultImage[next] : platform.image,
      requiresDockerSocket: next === "docker" ? platform.requiresDockerSocket : false,
    };
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    // The form is portaled out of the pool form; keep its submit from reaching that one.
    event.preventDefault();
    event.stopPropagation();
    const seen = new Set<string>();
    for (const platform of platforms) {
      const key = `${platform.runtimeKind}:${platform.executionOs}:${platform.architecture}`;
      if (seen.has(key)) {
        setError(`${platformName(platform)} is listed twice.`);
        return;
      }
      seen.add(key);
    }
    if (enabled && platforms.length === 0) {
      setError("Add at least one platform to run on machines.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const saved = await savePoolProfile(poolId, {
        enabled, placement, mode: native ? "ephemeral" : mode,
        cpuMillis: Math.round(cores * 1000), memoryMib, diskBytes: Math.round(diskGib * GIB),
        platforms: platforms.map(({ key: _key, ...platform }) => ({ ...platform, image: platform.image.trim() })),
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
      <div className="space-y-2 px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <div>
            <p className="text-sm font-medium text-foreground">Platforms</p>
            <p className="text-xs text-muted-foreground">Each job gets a runner on the platform its labels ask for, such as Windows x64 or Linux arm64.</p>
          </div>
          <Button
            disabled={platforms.length >= 12}
            onClick={() => setPlatforms((current) => [...current, { ...nextPlatform(current), key: draftKey() }])}
            size="sm"
            type="button"
            variant="outline"
          >
            <Plus />Add platform
          </Button>
        </div>
        {platforms.length === 0 ? <p className="text-sm text-muted-foreground">No platforms yet.</p> : null}
        {platforms.map((platform) => (
          <div className="space-y-2 rounded-lg border border-border p-3" key={platform.key}>
            <div className="grid gap-2 sm:grid-cols-[1.4fr_1fr_0.8fr_auto]">
              <SearchableSelect
                ariaLabel={`Runtime for ${platformName(platform)}`}
                onValueChange={(next) => next && update(platform.key, (current) => changeRuntime(current, next))}
                options={[
                  { value: "docker", label: "Docker container", description: "Linux containers" },
                  { value: "native_process", label: "Native process", description: "Directly on a Linux, macOS or Windows machine" },
                  { value: "tart_vm", label: "Tart VM", description: "macOS virtual machines" },
                ]}
                searchable={false}
                value={platform.runtimeKind}
              />
              <SearchableSelect
                ariaLabel={`Operating system for ${platformName(platform)}`}
                onValueChange={(next) => next && update(platform.key, (current) => ({ ...current, executionOs: next }))}
                options={osOptions[platform.runtimeKind].map((os) => ({ value: os, label: osLabels[os] }))}
                searchable={false}
                value={platform.executionOs}
              />
              <SearchableSelect
                ariaLabel={`Architecture for ${platformName(platform)}`}
                onValueChange={(next) => next && update(platform.key, (current) => ({ ...current, architecture: next }))}
                options={[{ value: "arm64", label: "arm64" }, { value: "x64", label: "x64" }]}
                searchable={false}
                value={platform.architecture}
              />
              <Button
                aria-label={`Remove ${platformName(platform)}`}
                onClick={() => setPlatforms((current) => current.filter((item) => item.key !== platform.key))}
                size="icon-sm"
                type="button"
                variant="ghost"
              >
                <Trash2 />
              </Button>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <Input
                aria-label={`Image for ${platformName(platform)}`}
                className="min-w-0 flex-1 font-mono text-xs"
                disabled={platform.runtimeKind === "native_process"}
                form={FORM_ID}
                onChange={(event) => update(platform.key, (current) => ({ ...current, image: event.target.value }))}
                required
                title={platform.runtimeKind === "native_process" ? "Native runners don’t use a container image." : platform.runtimeKind === "docker" ? "The machine must allow this image in GRIDOPS_AGENT_IMAGES." : "The Tart VM image each runner clones."}
                value={platform.image}
              />
              {platform.runtimeKind === "docker" ? (
                <label className="flex items-center gap-2 text-xs text-muted-foreground">
                  <Switch
                    aria-label={`Give ${platformName(platform)} jobs the Docker socket`}
                    checked={platform.requiresDockerSocket}
                    onCheckedChange={(next) => update(platform.key, (current) => ({ ...current, requiresDockerSocket: next }))}
                  />
                  Docker socket
                </label>
              ) : null}
            </div>
          </div>
        ))}
      </div>
      <SettingsRow description="Where each new runner starts. When both have room, the built-in host goes first because it starts runners fastest. Windows and other platforms the host can’t run always go to machines." label="Placement">
        <SearchableSelect
          ariaLabel="Fleet placement"
          onValueChange={(next) => next && setPlacement(next)}
          options={[
            { value: "machines", label: "Machines only", description: "Wait for a machine when none is free" },
            { value: "host_first", label: "Wherever there’s room", description: "The built-in runner host or a machine, whichever is free" },
            ...(placement === "machines_first" ? [{ value: "machines_first" as const, label: "Machines first", description: "Earlier setting" }] : []),
          ]}
          searchable={false}
          value={placement}
        />
      </SettingsRow>
      <SettingsRow description={native ? "Native runners are always ephemeral, so this pool’s runners are too." : undefined} label="Mode">
        <SearchableSelect
          ariaLabel="Fleet runner mode"
          onValueChange={(next) => next && setMode(next)}
          options={native
            ? [{ value: "ephemeral", label: "Ephemeral", description: "One clean runner per job" }]
            : [
              { value: "ephemeral", label: "Ephemeral", description: "One clean runner per job" },
              { value: "persistent", label: "Persistent", description: "Reuse the runner across jobs" },
            ]}
          searchable={false}
          value={native ? "ephemeral" : mode}
        />
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
      {error ? <div className="px-4 py-3"><Callout title="Could not save" tone="danger">{error}</Callout></div> : null}
    </SettingsSection>
  );
}
