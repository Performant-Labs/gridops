/**
 * The one Targets row of the runner pool forms (#145): a searchable multi-select grouped by target
 * kind, filled from the platform descriptors. Each kind loads its own options and keeps its own
 * load state (loading / error with Retry / not connected); the mixing rules come from kind
 * properties through `targets.ts`. Nothing here names a platform.
 */
import { Link } from "@tanstack/react-router";
import { LoaderCircle } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { InlineError } from "~/components/page";
import { Input } from "~/components/ui/input";
import { type SearchableMultiSelectGroup, type SearchableMultiSelectOption, SearchableMultiSelect } from "~/components/ui/searchable-multi-select";
import { platforms } from "~/features/platform-connections/registry";
import type { PlatformDescriptor, ResolvedSelection, TargetContext, TargetKind, TargetLimits, TargetOption } from "~/features/platform-connections/types";
import { isOffered, targetKinds, unavailableReason } from "~/features/runner-pools/targets";

type KindLoad = {
  /** The context the result was loaded for; a result for another context counts as still loading. */
  context: TargetContext;
  status: "loading" | "ready" | "error";
  options: TargetOption[];
  error: string | null;
};

type KindEntries = ReturnType<typeof targetKinds>;

const isAbort = (cause: unknown) => cause instanceof DOMException && cause.name === "AbortError";

/** Loads every listed kind once per context, and one kind again on its Retry. */
function useKindLoads(entries: KindEntries, context: TargetContext) {
  const [loads, setLoads] = useState<Record<string, KindLoad>>({});
  const controllers = useRef(new Map<string, AbortController>());

  const fetchKind = useCallback((kind: TargetKind) => {
    controllers.current.get(kind.id)?.abort();
    const controller = new AbortController();
    controllers.current.set(kind.id, controller);
    kind.load(context, controller.signal)
      .then((options) => {
        if (!controller.signal.aborted) setLoads((current) => ({ ...current, [kind.id]: { context, status: "ready", options, error: null } }));
      })
      .catch((cause: unknown) => {
        if (isAbort(cause) || controller.signal.aborted) return;
        const error = cause instanceof Error ? cause.message : `${kind.label} options could not be loaded.`;
        setLoads((current) => ({ ...current, [kind.id]: { context, status: "error", options: [], error } }));
      });
  }, [context]);

  const kindIds = entries.map(({ kind }) => kind.id).join("|");
  useEffect(() => {
    const active = controllers.current;
    for (const { kind } of entries) fetchKind(kind);
    return () => {
      for (const controller of active.values()) controller.abort();
      active.clear();
    };
    // `entries` is derived from the descriptors; reload when the listed kinds or the context change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kindIds, fetchKind]);

  function retry(kind: TargetKind) {
    setLoads((current) => ({ ...current, [kind.id]: { context, status: "loading", options: [], error: null } }));
    fetchKind(kind);
  }

  const current: Record<string, KindLoad> = {};
  for (const [kindId, load] of Object.entries(loads)) if (load.context === context) current[kindId] = load;
  return { loads: current, retry };
}

export function TargetsRow({
  descriptors = platforms,
  context,
  selection,
  onSelectionChange,
  limits,
  disabled = false,
}: {
  descriptors?: readonly PlatformDescriptor[];
  context: TargetContext;
  selection: ResolvedSelection;
  onSelectionChange: (selection: ResolvedSelection) => void;
  /** Form values the per-kind caps read; without them the kinds are not capped. */
  limits?: TargetLimits;
  disabled?: boolean;
}) {
  const entries = useMemo(() => targetKinds(descriptors), [descriptors]);
  const listed = useMemo(
    () => entries.filter(({ kind }) => isOffered(kind, context) && kind.available(context)),
    [context, entries],
  );
  const { loads, retry } = useKindLoads(listed, context);
  const [manualText, setManualText] = useState<Record<string, string>>({});

  const loadedByKey = new Map<string, TargetOption>();
  for (const load of Object.values(loads)) for (const option of load.options) loadedByKey.set(option.key, option);

  const values = entries.flatMap(({ kind }) => (selection[kind.id] ?? []).map((option) => option.key));

  const options: Array<SearchableMultiSelectOption<string>> = [];
  const groups: SearchableMultiSelectGroup[] = [];
  for (const { platform, kind } of entries) {
    const Icon = platform.icon;
    const icon = <Icon className="size-3.5" />;
    const selected = selection[kind.id] ?? [];
    if (!isOffered(kind, context)) {
      // Kept as they are: shown as chips, not offered for picking.
      for (const option of selected) options.push({ value: option.key, label: option.label, description: option.description, group: kind.group, icon, locked: true });
      continue;
    }
    const load = loads[kind.id];
    const reason = unavailableReason(descriptors, selection, kind, limits);
    const loaded = load?.options ?? [];
    for (const option of loaded) {
      options.push({
        value: option.key,
        label: option.label,
        description: option.description,
        keywords: [platform.name, kind.label, option.connectionId],
        group: kind.group,
        icon,
        disabled: reason !== null,
        disabledReason: reason ?? undefined,
      });
    }
    // A selected target missing from the loaded list (still loading, or the load failed) keeps its chip.
    for (const option of selected) {
      if (!loaded.some((candidate) => candidate.key === option.key)) options.push({ value: option.key, label: option.label, description: option.description, group: kind.group, icon });
    }
    groups.push({
      name: kind.group,
      status: !kind.available(context) ? (
        <>{kind.notConnected ?? `No ${platform.name} connections yet`}. Add one in <Link className="text-foreground underline-offset-2 hover:underline" to="/settings/connections">Settings → Connections</Link>.</>
      ) : !load || load.status === "loading" ? (
        <span className="inline-flex items-center gap-1.5" role="status"><LoaderCircle className="size-3 animate-spin" />Loading…</span>
      ) : load.status === "error" ? (
        <InlineError className="mt-0" onRetry={() => retry(kind)} title={`Couldn’t load ${kind.label.toLowerCase()} options`}>{load.error}</InlineError>
      ) : loaded.length === 0 ? `No ${kind.label.toLowerCase()} options` : undefined,
    });
  }

  function change(nextKeys: string[]) {
    const current = new Set(values);
    const next = new Set(nextKeys);
    const result: Record<string, TargetOption[]> = {};
    for (const { kind } of entries) {
      const kept = (selection[kind.id] ?? []).filter((option) => next.has(option.key));
      const added = nextKeys
        .filter((key) => !current.has(key))
        .map((key) => loadedByKey.get(key))
        .filter((option): option is TargetOption => option !== undefined && option.kindId === kind.id)
        .map((option) => {
          if (!option.manual || !kind.manualEntry) return option;
          const text = manualText[option.key] ?? kind.manualEntry.initialValue?.(context) ?? "";
          return { ...option, nativeId: kind.manualEntry.parse(text, option.connectionId) ?? option.nativeId };
        });
      const picked = kind.cardinality === "one" && added.length ? added.slice(-1) : [...kept, ...added];
      if (picked.length) result[kind.id] = picked;
    }
    onSelectionChange(result);
  }

  function typeManual(kindId: string, option: TargetOption, text: string) {
    const entry = entries.find(({ kind }) => kind.id === kindId);
    const manual = entry?.kind.manualEntry;
    if (!manual) return;
    setManualText((current) => ({ ...current, [option.key]: text }));
    const fallback = loadedByKey.get(option.key)?.nativeId ?? option.nativeId;
    onSelectionChange({
      ...selection,
      [kindId]: (selection[kindId] ?? []).map((candidate) => candidate.key === option.key
        ? { ...candidate, nativeId: manual.parse(text, candidate.connectionId) ?? fallback }
        : candidate),
    });
  }

  const manualInputs = entries.flatMap(({ kind }) => {
    const manual = kind.manualEntry;
    return manual ? (selection[kind.id] ?? []).filter((option) => option.manual).map((option) => ({ kind, option, manual })) : [];
  });

  return (
    <div className="space-y-3">
      <SearchableMultiSelect
        ariaLabel="Targets"
        disabled={disabled}
        emptyMessage="No targets match this search"
        groups={groups}
        onValueChange={change}
        options={options}
        placeholder="Choose where runners register…"
        searchPlaceholder="Search targets…"
        selectedNoun="targets"
        values={values}
      />
      {manualInputs.map(({ kind, option, manual }) => {
        const id = `target-manual-${option.key.replace(/[^a-z0-9-]/gi, "-")}`;
        return (
          <div className="space-y-1.5" key={option.key}>
            <label className="block text-xs font-medium text-foreground" htmlFor={id}>{manual.label}</label>
            <Input
              id={id}
              min="1"
              onChange={(event) => typeManual(kind.id, option, event.target.value)}
              required
              type="number"
              value={manualText[option.key] ?? manual.initialValue?.(context) ?? ""}
            />
            <p className="text-xs text-muted-foreground">{option.label}</p>
          </div>
        );
      })}
    </div>
  );
}
