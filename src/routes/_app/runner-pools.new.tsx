import { Link, createFileRoute, useNavigate } from "@tanstack/react-router";
import { Boxes, LoaderCircle, Plus } from "lucide-react";
import { type FormEvent, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

import { EmptyState, Callout, PageBody, PageHeader } from "~/components/page";
import { PoolPlanPreview } from "~/components/pool-plan-preview";
import { ResourcePageLoading } from "~/components/resource-page-loading";
import { SettingsLayout, SettingsRow, SettingsSection, Switch, UnitField } from "~/components/settings-ui";
import { Button, buttonVariants } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { SearchableMultiSelect } from "~/components/ui/searchable-multi-select";
import { SearchableSelect } from "~/components/ui/searchable-select";
import { platforms } from "~/features/platform-connections/registry";
import type { ResolvedSelection, TargetContext } from "~/features/platform-connections/types";
import {
  type RunnerPoolOptions,
  createRunnerPoolAction,
  getCreateRunnerPoolOptions,
} from "~/features/runner-pools/runner-pools.functions";
import { BuiltinHostOffNotice } from "~/features/runner-pools/builtin-host-off-notice";
import { builtinHostOn } from "~/features/runner-pools/builtin-host";
import { HostCapacityWarning } from "~/features/runner-pools/host-capacity-warning";
import { PlatformOptionsSection } from "~/features/runner-pools/platform-options-section";
import { hostResourceWarning } from "~/features/runner-pools/resource-risk";
import { poolImages } from "~/features/runner-pools/pool-images";
import type { CreateRunnerPoolInput } from "~/features/runner-pools/schemas";
import { TartBaseVmRow } from "~/features/runner-pools/tart-base-vm-row";
import {
  anyStandaloneKindAvailable, buildCreatePayload, labelError, labelRules, minMaxCount, prerequisiteEmptyState, requiredProviders, targetPlanCheck,
} from "~/features/runner-pools/targets";
import { TargetsRow } from "~/features/runner-pools/targets-row";

export const Route = createFileRoute("/_app/runner-pools/new")({
  loader: () => getCreateRunnerPoolOptions(),
  pendingComponent: () => <ResourcePageLoading icon={Boxes} title="New runner pool" />,
  component: NewRunnerPoolPage,
});

function NewRunnerPoolPage() {
  const options = Route.useLoaderData();
  // Built once and handed, as the same object, to every target kind and payload partial.
  const context = useMemo<TargetContext>(() => ({ form: "create", options }), [options]);

  useEffect(() => {
    const search = new URLSearchParams(window.location.search);
    if (search.get("appCreated") === "1") {
      toast.success("GitHub App created and authorized. Install it on an account to continue.");
    }
    if (search.get("installationUpdated") === "1") {
      toast.success("GitHub App installation synchronized.");
    }
    if (search.has("appCreated") || search.has("installationUpdated")) {
      window.history.replaceState({}, "", window.location.pathname);
    }
  }, []);

  const header = <PageHeader breadcrumbs={[{ label: "Runner pools", to: "/runner-pools" }]} title="New pool" />;

  // A pool needs a standalone target; until one is available the descriptor says what to connect first.
  if (!options.authenticated || !options.defaults || !anyStandaloneKindAvailable(platforms, context)) {
    const blocker = prerequisiteEmptyState(platforms, context);
    return (
      <>
        {header}
        <PageBody>
          <EmptyState description={blocker?.description ?? "Connect a CI platform before creating a runner pool."} icon={blocker?.icon ?? Boxes} title={blocker?.title ?? "Connect a platform first"}>
            {blocker?.action ? <a className={buttonVariants()} href={blocker.action.href}><blocker.icon />{blocker.action.label}</a> : null}
          </EmptyState>
        </PageBody>
      </>
    );
  }

  return (
    <>
      {header}
      <PageBody>
        <RunnerPoolForm context={context} options={options} />
      </PageBody>
    </>
  );
}

type RunnerPoolFormOptions = Extract<RunnerPoolOptions, { authenticated: true }>;

type RunnerProvider = "docker" | "tart";

function RunnerPoolForm({ options, context }: { options: RunnerPoolFormOptions; context: TargetContext }) {
  const createPool = createRunnerPoolAction;
  const navigate = useNavigate();
  const hostOn = builtinHostOn(options.builtinRunnerHostEnabled);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selection, setSelection] = useState<ResolvedSelection>({});
  const [mode, setMode] = useState<"ephemeral" | "persistent">("ephemeral");
  const [providers, setProviders] = useState<RunnerProvider[]>(options.defaults.providers);
  const provider = providers[0] ?? options.defaults.provider;
  const includesTart = providers.includes("tart");
  const [dockerImage, setDockerImage] = useState(options.defaults.dockerImage);
  const [tartImage, setTartImage] = useState(options.defaults.tartImage);
  const [macosRuntime, setMacosRuntime] = useState<"vm" | "native">(options.defaults.macosRuntime ?? "vm");
  const [maxCount, setMaxCount] = useState(options.defaults.maxCount);
  const [desiredCount, setDesiredCount] = useState(options.defaults.desiredCount);
  const [cpuLimit, setCpuLimit] = useState(options.defaults.cpuLimit);
  const [memoryLimitMb, setMemoryLimitMb] = useState(options.defaults.memoryLimitMb);
  const [poolName, setPoolName] = useState("");
  const [labels, setLabels] = useState(options.defaults.labels.join(", "));
  const resourceWarning = hostResourceWarning({
    runnerCount: desiredCount,
    cpuLimit,
    memoryLimitMb,
    cpuBudget: options.defaults.maxCpuLimit,
    memoryBudgetMb: options.defaults.maxMemoryLimitMb,
  });

  function changeTargets(next: ResolvedSelection) {
    setSelection(next);
    // A kind that needs a provider adds it, as picking a workspace added macOS · Tart before.
    const missing = requiredProviders(platforms, next).filter((required) => !providers.includes(required));
    if (missing.length) setProviders((current) => [...current, ...missing.filter((required) => !current.includes(required))]);
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitting(true);
    setError(null);
    const form = new FormData(event.currentTarget);

    try {
      const data = {
        // installationId, scope, repositoryIds, runnerGroupId and each platform's own fields.
        ...buildCreatePayload(platforms, selection, context),
        name: poolName,
        mode,
        provider,
        providers,
        labels: labels
          .split(",")
          .map((label) => label.trim())
          .filter(Boolean),
        // Native execution clones no VM and hides the field, so the prefilled
        // default is not sent either: the pool stores no base VM, and
        // switching it to Virtual machine later asks for one. (Settings keeps
        // the base VM a native pool already has.)
        ...poolImages({ provider, providers, dockerImage, tartImage: macosRuntime === "native" ? "" : tartImage }),
        macosRuntime,
        desiredCount: Number(form.get("desiredCount")),
        minCount: Number(form.get("minCount")),
        maxCount: Number(form.get("maxCount")),
        autoscalingEnabled: form.get("autoscalingEnabled") === "on",
        queueScaleFactor: Number(form.get("queueScaleFactor")),
        idleTimeoutMinutes: Number(form.get("idleTimeoutMinutes")),
        cpuLimit: Number(form.get("cpuLimit")),
        memoryLimitMb: Number(form.get("memoryLimitMb")),
      };
      // The schema in createRunnerPoolAction is the submit-time authority for the target fields.
      const created = await createPool({ data: data as CreateRunnerPoolInput });
      // With the built-in host off the new pool has nowhere to run until it is given fleet execution.
      if (created.cannotRun) await navigate({ params: { poolId: created.id }, to: "/runner-pools/$poolId/settings" });
      else await navigate({ to: "/runner-pools" });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Runner pool creation failed.");
    } finally {
      setSubmitting(false);
    }
  }

  const labelList = labels.split(",").map((label) => label.trim()).filter(Boolean);
  const rules = labelRules(platforms, selection);
  const labelProblem = labelError(platforms, selection, labelList);

  return (
    <form className="contents" onSubmit={submit}>
      <SettingsLayout description="Choose where runners register, what they run on, and how far the pool may scale." title="New runner pool">
        {hostOn ? null : <BuiltinHostOffNotice />}
        <SettingsSection description="Where this pool’s runners register." title="Destination">
          <SettingsRow
            description="Repositories from different accounts can share capacity. Targets that cannot share a pool are disabled, with the reason. The maximum runner count must cover every repository."
            label="Targets"
            stacked
          >
            <TargetsRow context={context} descriptors={platforms} limits={{ maxCount }} onSelectionChange={changeTargets} selection={selection} />
          </SettingsRow>
        </SettingsSection>

        <PlatformOptionsSection descriptors={platforms} selection={selection} />

        <SettingsSection description="What each runner is and how jobs find it." title="Runner definition">
          <SettingsRow description="Lowercase letters, numbers, and hyphens. Also registered as a runner label." htmlFor="pool-name" label="Pool name">
            <Input id="pool-name" name="name" onChange={(event) => setPoolName(event.target.value)} pattern="[a-z0-9][a-z0-9-]*[a-z0-9]" placeholder={providers.length > 1 ? "cross-platform" : provider === "tart" ? "macos-arm64" : "linux-general"} required value={poolName} />
          </SettingsRow>
          <SettingsRow description="Jobs route to the first provider whose system labels match. Capacity limits are shared." label="Providers">
            <SearchableMultiSelect
              ariaLabel="Runner providers"
              maxSelected={2}
              onValueChange={(nextProviders) => {
                if (nextProviders.length === 0) return;
                setProviders(nextProviders);
                if (nextProviders.includes("tart")) setMode("ephemeral");
              }}
              options={[
                { value: "docker", label: "Linux · Docker", description: "Fast Linux containers" },
                { value: "tart", label: "macOS · Tart", description: "macOS VMs or native host execution" },
              ]}
              placeholder="Choose providers…"
              searchPlaceholder="Search providers…"
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
              <Input className="font-mono text-xs" id="docker-image" name="dockerImage" onChange={(event) => setDockerImage(event.target.value)} required value={dockerImage} />
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
          <SettingsRow description={rules?.help ?? "Comma-separated. OS and architecture labels are added automatically."} htmlFor="pool-labels" label="Additional labels">
            <Input aria-invalid={labelProblem ? true : undefined} id="pool-labels" name="labels" onChange={(event) => setLabels(event.target.value)} placeholder={rules?.placeholder ?? (providers.length > 1 ? "build, release" : provider === "tart" ? "xcode, apple-silicon" : "docker, build")} value={labels} />
            {labelProblem ? <p className="mt-1.5 text-left text-xs leading-5 text-danger">{labelProblem}</p> : null}
          </SettingsRow>
        </SettingsSection>

        <SettingsSection description="How many runners GridOps keeps and what each one may use." title="Capacity">
          <SettingsRow description="Autoscaling moves the target between the minimum and maximum." label="Runner counts" stacked>
            <div className="grid grid-cols-3 gap-3">
              <CountInput label="Target" name="desiredCount" onChange={setDesiredCount} value={desiredCount} />
              <CountInput defaultValue={options.defaults.minCount} label="Minimum" name="minCount" />
              <CountInput label="Maximum" min={minMaxCount(platforms, selection)} name="maxCount" onChange={setMaxCount} value={maxCount} />
            </div>
          </SettingsRow>
          <SettingsRow description={includesTart ? "Whole cores, because this pool includes macOS." : "Applied to each Docker runner."} htmlFor="cpu-limit" label="CPU per runner">
            <UnitField unit="cores"><Input className="pr-12" id="cpu-limit" name="cpuLimit" onChange={(event) => setCpuLimit(Number(event.target.value))} required step={includesTart ? "1" : "0.25"} type="number" value={cpuLimit} /></UnitField>
          </SettingsRow>
          <SettingsRow description={hostOn ? "Checked against the host budget when each runner starts." : "Reserved on the machine for each runner."} htmlFor="memory-limit" label="Memory per runner">
            <UnitField unit="MB"><Input className="pr-10" id="memory-limit" name="memoryLimitMb" onChange={(event) => setMemoryLimitMb(Number(event.target.value))} required step="256" type="number" value={memoryLimitMb} /></UnitField>
          </SettingsRow>
          {resourceWarning && hostOn ? (
            <div className="px-4 py-3">
              <HostCapacityWarning host={options.defaults.runnerHost} warning={resourceWarning} />
            </div>
          ) : null}
          <div className="px-4 py-3.5">
            <PoolPlanPreview desiredCount={desiredCount} labels={labelList} maxCount={maxCount} name={poolName} providers={providers} targetCheck={targetPlanCheck(platforms, selection, { maxCount }, context)} />
          </div>
        </SettingsSection>

        <SettingsSection description="Grow from the queue, shrink when idle." title="Autoscaling">
          <SettingsRow description="Queued workflow jobs raise the target up to the maximum." inline label="Autoscale from queued jobs">
            <Switch aria-label="Autoscale from queued jobs" defaultChecked={options.defaults.autoscalingEnabled} name="autoscalingEnabled" />
          </SettingsRow>
          <SettingsRow description="Runner slots requested per queued job, capped by the maximum." htmlFor="queue-factor" label="Runners per queued job">
            <Input defaultValue={options.defaults.queueScaleFactor} id="queue-factor" max="20" min="1" name="queueScaleFactor" required type="number" />
          </SettingsRow>
          <SettingsRow description="Once every runner is idle and nothing is queued for this long, the target returns to the minimum." htmlFor="idle-timeout" label="Idle scale-down delay">
            <UnitField unit="minutes"><Input className="pr-16" defaultValue={options.defaults.idleTimeoutMinutes} id="idle-timeout" max="1440" min="1" name="idleTimeoutMinutes" required type="number" /></UnitField>
          </SettingsRow>
        </SettingsSection>

        {error ? <Callout title="Could not create the pool" tone="danger">{error}</Callout> : null}
      </SettingsLayout>
      <div className="sticky bottom-0 z-20 border-t border-border bg-panel/90 backdrop-blur">
        <div className="mx-auto flex max-w-[720px] items-center justify-end gap-2 px-5 py-3 sm:px-8">
          <Link className={buttonVariants({ variant: "ghost" })} to="/runner-pools">Cancel</Link>
          <Button disabled={submitting || !anyStandaloneKindAvailable(platforms, context)} type="submit">
            {submitting ? <LoaderCircle className="animate-spin" /> : <Plus />}
            {submitting ? "Creating…" : "Create pool"}
          </Button>
        </div>
      </div>
    </form>
  );
}

function CountInput({ label, name, value, defaultValue, onChange, min = 0 }: { label: string; name: string; value?: number; defaultValue?: number; onChange?: (value: number) => void; min?: number }) {
  return (
    <label className="space-y-1.5">
      <span className="block text-xs text-muted-foreground">{label}</span>
      <Input defaultValue={defaultValue} max="100" min={min} name={name} onChange={onChange ? (event) => onChange(Number(event.target.value)) : undefined} required type="number" value={value} />
    </label>
  );
}
