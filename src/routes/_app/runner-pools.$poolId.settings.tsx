import { builtinHostOn, showsBuiltinGuardrails } from "~/features/runner-pools/builtin-host";
import { Link, createFileRoute, getRouteApi, useNavigate } from "@tanstack/react-router";
import { LoaderCircle } from "lucide-react";
import { type FormEvent, useMemo, useState } from "react";

import { Callout } from "~/components/page";
import { PoolPlanPreview } from "~/components/pool-plan-preview";
import { SettingsLayout, SettingsRow, SettingsSection, SettingsValue, Switch, UnitField } from "~/components/settings-ui";
import { Badge } from "~/components/ui/badge";
import { Button, buttonVariants } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { SearchableMultiSelect } from "~/components/ui/searchable-multi-select";
import { SearchableSelect } from "~/components/ui/searchable-select";
import { FleetExecutionSection } from "~/features/fleet/fleet-profile-section";
import { platforms } from "~/features/platform-connections/registry";
import type { ResolvedSelection, TargetContext } from "~/features/platform-connections/types";
import { providerLabel } from "~/features/runner-pools/pool-actions";
import { HostCapacityWarning } from "~/features/runner-pools/host-capacity-warning";
import { hostResourceWarning } from "~/features/runner-pools/resource-risk";
import { poolImages, startingImage } from "~/features/runner-pools/pool-images";
import { PlatformOptionsSection } from "~/features/runner-pools/platform-options-section";
import type { UpdateRunnerPoolInput } from "~/features/runner-pools/schemas";
import { TartBaseVmRow } from "~/features/runner-pools/tart-base-vm-row";
import { type RunnerPoolDetail, updateRunnerPoolAction } from "~/features/runner-pools/runner-pools.functions";
import {
  buildUpdatePayload, initialPriorityKeys, initialSelection, minMaxCount, priorityCandidates, prioritySupported, readOnlySummaries, targetPlanCheck,
} from "~/features/runner-pools/targets";
import { TargetsRow } from "~/features/runner-pools/targets-row";

export const Route = createFileRoute("/_app/runner-pools/$poolId/settings")({
  component: PoolSettingsTab,
});

const poolRoute = getRouteApi("/_app/runner-pools/$poolId");

function PoolSettingsTab() {
  const pool = poolRoute.useLoaderData();
  return pool.canManage ? <RunnerPoolEditor key={`${pool.id}:${pool.configurationVersion}`} pool={pool} /> : <ReadOnlyPool pool={pool} />;
}

function RunnerPoolEditor({ pool }: { pool: RunnerPoolDetail }) {
  const navigate = useNavigate();
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<"ephemeral" | "persistent">(pool.mode);
  const poolProviders: Array<"docker" | "tart"> = pool.providers?.length ? pool.providers : [pool.provider];
  const [providers, setProviders] = useState(poolProviders);
  // Adding a provider asks for its image: it starts empty rather than with
  // anything the pool may still hold for a provider it did not use (#103).
  const [dockerImage, setDockerImage] = useState(() => startingImage(poolProviders, "docker", pool.dockerImage));
  const [tartImage, setTartImage] = useState(() => startingImage(poolProviders, "tart", pool.tartImage));
  const [macosRuntime, setMacosRuntime] = useState<"vm" | "native">(pool.macosRuntime ?? "vm");
  // Built once and handed, as the same object, to every target kind and payload partial.
  const context = useMemo<TargetContext>(() => ({ form: "update", pool }), [pool]);
  const [selection, setSelection] = useState<ResolvedSelection>(() => initialSelection(platforms, pool));
  const [priorityKeys, setPriorityKeys] = useState(() => initialPriorityKeys(platforms, pool));
  const [maxCount, setMaxCount] = useState(pool.maxCount);
  const [desiredCount, setDesiredCount] = useState(pool.desiredCount);
  const [cpuLimit, setCpuLimit] = useState(pool.cpuLimit);
  const [memoryLimitMb, setMemoryLimitMb] = useState(pool.memoryLimitMb);
  const [poolName, setPoolName] = useState(pool.name);
  const [labels, setLabels] = useState(pool.labels.join(", "));
  const primaryProvider = providers[0] ?? "docker";
  const includesTart = providers.includes("tart");
  const resourceWarning = hostResourceWarning({
    runnerCount: desiredCount,
    cpuLimit,
    memoryLimitMb,
    cpuBudget: pool.maxCpuLimit ?? 0,
    memoryBudgetMb: pool.maxMemoryLimitMb ?? 0,
  });
  const showPriority = prioritySupported(platforms, context);
  const prioritisable = priorityCandidates(platforms, selection);
  const labelList = labels.split(",").map((label) => label.trim()).filter(Boolean);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitting(true);
    setError(null);
    const form = new FormData(event.currentTarget);
    try {
      await updateRunnerPoolAction({
        data: {
          poolId: pool.id,
          // Each platform's own fields (repositories, priority, runner group …); the schema in
          // updateRunnerPoolAction stays the submit-time authority.
          ...(buildUpdatePayload(platforms, selection, pool, {
            priorityKeys,
            priorityWaitMinutes: showPriority ? Number(form.get("priorityWaitMinutes")) : undefined,
          }) as Partial<UpdateRunnerPoolInput>),
          name: poolName,
          mode,
          provider: primaryProvider,
          providers,
          labels: labelList,
          ...poolImages({ provider: primaryProvider, providers, dockerImage, tartImage }),
          macosRuntime,
          desiredCount: Number(form.get("desiredCount")),
          minCount: Number(form.get("minCount")),
          maxCount: Number(form.get("maxCount")),
          autoscalingEnabled: form.get("autoscalingEnabled") === "on",
          queueScaleFactor: Number(form.get("queueScaleFactor")),
          idleTimeoutMinutes: Number(form.get("idleTimeoutMinutes")),
          cpuLimit: Number(form.get("cpuLimit")),
          memoryLimitMb: Number(form.get("memoryLimitMb")),
        } as UpdateRunnerPoolInput & { poolId: string },
      });
      await navigate({ to: "/runner-pools/$poolId", params: { poolId: pool.id } });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Runner pool update failed.");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form className="contents" onSubmit={submit}>
      <SettingsLayout
        description={<>Generation {pool.configurationVersion}. Changing the runtime starts a rolling replacement: busy runners finish their jobs, and idle runners are replaced one at a time.</>}
        title="Configuration"
      >
        <SettingsSection description="Where this pool’s runners register." title="Destination">
          <SettingsRow description="The pool keeps its kind of target. Targets shown without a remove button stay as they are." label="Targets" stacked>
            <TargetsRow context={context} descriptors={platforms} limits={{ maxCount }} onSelectionChange={setSelection} selection={selection} />
          </SettingsRow>
        </SettingsSection>

        <PlatformOptionsSection descriptors={platforms} selection={selection} />

        <SettingsSection description="What each runner is and how jobs find it." title="Runner definition">
          <SettingsRow description="Also registered as a runner label." htmlFor="pool-name" label="Pool name">
            <Input id="pool-name" name="name" onChange={(event) => setPoolName(event.target.value)} pattern="[a-z0-9][a-z0-9-]*[a-z0-9]" required value={poolName} />
          </SettingsRow>
          <SettingsRow description="Jobs route to the first compatible provider; the first one also takes jobs that only ask for self-hosted." label="Providers">
            <SearchableMultiSelect
              ariaLabel="Runner providers"
              maxSelected={2}
              onValueChange={(values) => {
                const selected = values.filter((value): value is "docker" | "tart" => value === "docker" || value === "tart");
                if (!selected.length) return;
                setProviders(selected);
                if (selected.includes("tart")) setMode("ephemeral");
              }}
              options={[
                { value: "docker", label: "Linux · Docker", description: "Fast Linux containers" },
                { value: "tart", label: "macOS · Tart", description: "macOS VMs or native host execution" },
              ]}
              selectedNoun="providers"
              values={providers}
            />
          </SettingsRow>
          <SettingsRow description={includesTart ? "macOS runners are always ephemeral." : "Ephemeral runners take one job each; persistent runners are reused."} label="Mode">
            <SearchableSelect
              ariaLabel="Runner mode"
              onValueChange={(nextMode) => setMode(nextMode ?? "ephemeral")}
              options={includesTart
                ? [{ value: "ephemeral", label: "Ephemeral", description: "One clean runner per job" }]
                : [
                  { value: "ephemeral", label: "Ephemeral", description: "One clean runner per job" },
                  { value: "persistent", label: "Persistent", description: "Reuse the runner across jobs" },
                ]}
              searchable={false}
              value={mode}
            />
          </SettingsRow>
          {providers.includes("docker") ? (
            <SettingsRow description="OCI image for each Linux runner." htmlFor="docker-image" label="Docker image">
              <Input className="font-mono text-xs" id="docker-image" onChange={(event) => setDockerImage(event.target.value)} required value={dockerImage} />
            </SettingsRow>
          ) : null}
          {includesTart ? (
            <SettingsRow description="Native runs jobs directly on the macOS agent host: no VM image, TCC-gated APIs available, but jobs share the host." label="macOS execution">
              <SearchableSelect
                ariaLabel="macOS execution"
                onValueChange={(next) => setMacosRuntime(next === "native" ? "native" : "vm")}
                options={[
                  { value: "vm", label: "Virtual machine", description: "One copy-on-write Tart VM per job" },
                  { value: "native", label: "Native on host", description: "Run jobs directly on the agent host" },
                ]}
                searchable={false}
                value={macosRuntime}
              />
            </SettingsRow>
          ) : null}
          {includesTart && macosRuntime === "vm" ? (
            <TartBaseVmRow onChange={setTartImage} value={tartImage} />
          ) : null}
          <SettingsRow description="Comma-separated. self-hosted, OS, architecture, and the pool name are added automatically." htmlFor="pool-labels" label="Additional labels">
            <Input id="pool-labels" name="labels" onChange={(event) => setLabels(event.target.value)} placeholder="gpu, large" value={labels} />
          </SettingsRow>
        </SettingsSection>

        <SettingsSection description="How many runners GridOps keeps and what each one may use." title="Capacity">
          <SettingsRow description="Autoscaling moves the target between the minimum and maximum." label="Runner counts" stacked>
            <div className="grid grid-cols-3 gap-3">
              <NumberInput label="Target" onChange={setDesiredCount} name="desiredCount" value={desiredCount} />
              <NumberInput defaultValue={pool.minCount} label="Minimum" name="minCount" />
              <NumberInput label="Maximum" min={minMaxCount(platforms, selection)} name="maxCount" onChange={setMaxCount} value={maxCount} />
            </div>
          </SettingsRow>
          <SettingsRow description={includesTart ? "Whole cores, because this pool includes macOS." : "Applied to each Docker runner."} htmlFor="cpu-limit" label="CPU per runner">
            <UnitField unit="cores"><Input className="pr-12" id="cpu-limit" name="cpuLimit" onChange={(event) => setCpuLimit(Number(event.target.value))} required step={includesTart ? "1" : "0.25"} type="number" value={cpuLimit} /></UnitField>
          </SettingsRow>
          <SettingsRow description={showsBuiltinGuardrails(pool.builtinRunnerHostEnabled) ? "Checked against the host budget when each runner starts." : "Reserved on the machine for each runner."} htmlFor="memory-limit" label="Memory per runner">
            <UnitField unit="MB"><Input className="pr-10" id="memory-limit" name="memoryLimitMb" onChange={(event) => setMemoryLimitMb(Number(event.target.value))} required step="256" type="number" value={memoryLimitMb} /></UnitField>
          </SettingsRow>
          {resourceWarning && showsBuiltinGuardrails(pool.builtinRunnerHostEnabled) ? (
            <div className="px-4 py-3">
              <HostCapacityWarning host={pool.runnerHost} warning={resourceWarning} />
            </div>
          ) : null}
          <div className="px-4 py-3.5">
            <PoolPlanPreview desiredCount={desiredCount} labels={labelList} maxCount={maxCount} name={poolName} providers={providers} targetCheck={targetPlanCheck(platforms, selection, { maxCount }, context)} />
          </div>
        </SettingsSection>

        {showPriority ? (
          <SettingsSection description="Which repositories get the next runner when the pool is busy." title="Priority">
            <SettingsRow description="Their queued jobs get the next runner ahead of the pool’s other repositories, on machines and on this host." label="Prioritised repositories" stacked>
              <SearchableMultiSelect
                ariaLabel="Prioritised repositories"
                emptyMessage="No pool repositories match this search"
                onValueChange={setPriorityKeys}
                options={prioritisable.map((target) => ({ value: target.key, label: target.label, description: target.description }))}
                placeholder="None — every repository takes its turn"
                searchPlaceholder="Search the pool’s repositories…"
                selectedNoun="repositories"
                values={priorityKeys.filter((key) => prioritisable.some((target) => target.key === key))}
              />
            </SettingsRow>
            <SettingsRow description="A job from any repository that waits this long goes ahead too, so prioritised work can’t hold every runner." htmlFor="priority-wait" label="Fair-share wait">
              <UnitField unit="minutes"><Input className="pr-16" defaultValue={pool.priorityWaitMinutes} id="priority-wait" max="1440" min="1" name="priorityWaitMinutes" required type="number" /></UnitField>
            </SettingsRow>
          </SettingsSection>
        ) : null}

        <SettingsSection description="Grow from the queue, shrink when idle." title="Autoscaling">
          <SettingsRow description="Queued workflow jobs raise the target up to the maximum." inline label="Autoscale from queued jobs">
            <Switch aria-label="Autoscale from queued jobs" defaultChecked={pool.autoscalingEnabled} name="autoscalingEnabled" />
          </SettingsRow>
          <SettingsRow description="Runner slots requested per queued job, capped by the maximum." htmlFor="queue-factor" label="Runners per queued job">
            <Input defaultValue={pool.queueScaleFactor} id="queue-factor" max="20" min="1" name="queueScaleFactor" required type="number" />
          </SettingsRow>
          <SettingsRow description="Once every runner is idle and nothing is queued for this long, the target returns to the minimum." htmlFor="idle-timeout" label="Idle scale-down delay">
            <UnitField unit="minutes"><Input className="pr-16" defaultValue={pool.idleTimeoutMinutes} id="idle-timeout" max="1440" min="1" name="idleTimeoutMinutes" required type="number" /></UnitField>
          </SettingsRow>
        </SettingsSection>

        {/* Older Docker pools keep their image in `image`. */}
        <FleetExecutionSection builtinHostEnabled={builtinHostOn(pool.builtinRunnerHostEnabled)} dockerImage={pool.dockerImage || (pool.provider === "docker" ? pool.image : "")} poolId={pool.id} />

        {error ? <Callout title="Could not save" tone="danger">{error}</Callout> : null}
      </SettingsLayout>
      <div className="sticky bottom-0 z-20 border-t border-border bg-panel/90 backdrop-blur">
        <div className="mx-auto flex max-w-[720px] items-center justify-end gap-2 px-5 py-3 sm:px-8">
          <Link className={buttonVariants({ variant: "ghost" })} params={{ poolId: pool.id }} to="/runner-pools/$poolId">Cancel</Link>
          <Button disabled={submitting} type="submit">{submitting ? <LoaderCircle className="animate-spin" /> : null}{submitting ? "Saving…" : "Save changes"}</Button>
        </div>
      </div>
    </form>
  );
}

function NumberInput({ label, name, value, defaultValue, onChange, min = 0 }: { label: string; name: string; value?: number; defaultValue?: number; onChange?: (value: number) => void; min?: number }) {
  return (
    <label className="space-y-1.5">
      <span className="block text-xs text-muted-foreground">{label}</span>
      <Input
        defaultValue={defaultValue}
        max="100"
        min={min}
        name={name}
        onChange={onChange ? (event) => onChange(Number(event.target.value)) : undefined}
        required
        type="number"
        value={value}
      />
    </label>
  );
}

function ReadOnlyPool({ pool }: { pool: RunnerPoolDetail }) {
  const providers = pool.providers?.length ? pool.providers : [pool.provider];
  const context: TargetContext = { form: "update", pool };
  return (
    <SettingsLayout actions={<Badge variant="outline">Read only</Badge>} description="An installation administrator manages this pool." title="Configuration">
      <SettingsSection title="Destination">
        {readOnlySummaries(platforms, pool).map(({ platform, value }) => <SettingsValue key={platform.id} label={platform.name} value={value} />)}
      </SettingsSection>
      <SettingsSection title="Runner definition">
        <SettingsValue label="Providers" value={providers.map((provider) => providerLabel(provider, pool.macosRuntime)).join(" + ")} />
        <SettingsValue label="Mode" value={pool.mode} />
        {providers.includes("docker") ? <SettingsValue label="Docker image" mono value={pool.dockerImage} /> : null}
        {providers.includes("tart") && pool.macosRuntime !== "native" ? <SettingsValue label="Tart base VM" mono value={pool.tartImage} /> : null}
      </SettingsSection>
      <SettingsSection title="Capacity">
        <SettingsValue label="Runners" value={`${pool.desiredCount} target · ${pool.minCount}–${pool.maxCount}`} />
        <SettingsValue label="Per runner" value={`${pool.cpuLimit} cores · ${pool.memoryLimitMb} MB`} />
        <SettingsValue label="Autoscaling" value={pool.autoscalingEnabled ? `On · ${pool.queueScaleFactor} per queued job · idle after ${pool.idleTimeoutMinutes}m` : "Off"} />
      </SettingsSection>
      {prioritySupported(platforms, context) ? (
        <SettingsSection title="Priority">
          <SettingsValue
            label="Prioritised"
            value={pool.repositories.filter((repository) => repository.prioritised).map((repository) => repository.fullName).join(", ") || "None"}
          />
          <SettingsValue label="Fair-share wait" value={`${pool.priorityWaitMinutes} minutes`} />
        </SettingsSection>
      ) : null}
    </SettingsLayout>
  );
}
