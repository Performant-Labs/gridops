/**
 * The Targets rules for the runner pool forms (#145): pure functions over the platform descriptors
 * (no React). They read kind properties such as `standalone`, `requires`, `excludes` and `limit`,
 * never a platform id, so a new platform's kinds follow the same rules with no change here.
 */
import type {
  PayloadPartial,
  PlatformDescriptor,
  PlatformOptionValues,
  PrerequisiteEmptyState,
  ResolvedSelection,
  TargetContext,
  TargetCreateValues,
  TargetKind,
  TargetLimits,
  TargetOption,
  TargetUpdateValues,
} from "~/features/platform-connections/types";
import type { RunnerPoolDetail } from "~/features/runner-pools/runner-pools.functions";

type Descriptors = readonly PlatformDescriptor[];

export type KindEntry = { platform: PlatformDescriptor; kind: TargetKind };

/** Every registered kind with its platform, in registry order. */
export function targetKinds(descriptors: Descriptors): KindEntry[] {
  return descriptors.flatMap((platform) => platform.targetKinds.map((kind) => ({ platform, kind })));
}

const selectedCount = (selection: ResolvedSelection, kindId: string) => selection[kindId]?.length ?? 0;

export const isOffered = (kind: TargetKind, context: TargetContext) => kind.offered?.(context) ?? true;

/** The platforms that have at least one selected target. */
export function targetedPlatforms(descriptors: Descriptors, selection: ResolvedSelection) {
  return descriptors.filter((platform) => platform.targetKinds.some((kind) => selectedCount(selection, kind.id) > 0));
}

/** Why an unselected option of `kind` cannot be picked right now, or null when it can. */
export function unavailableReason(descriptors: Descriptors, selection: ResolvedSelection, kind: TargetKind, limits?: TargetLimits): string | null {
  for (const { kind: other } of targetKinds(descriptors)) {
    if (other.id === kind.id || selectedCount(selection, other.id) === 0) continue;
    if (other.excludes?.includes(kind.id) || kind.excludes?.includes(other.id)) {
      return other.exclusionReason ?? `Can’t combine with ${other.label.toLowerCase()} targets`;
    }
  }
  if (kind.requires?.length && !kind.requires.some((id) => selectedCount(selection, id) > 0)) {
    return kind.requiresReason ?? "Select a target it can share a pool with first";
  }
  const limit = limits && kind.limit ? kind.limit(limits) : undefined;
  if (limit !== undefined && kind.cardinality === "many" && selectedCount(selection, kind.id) >= limit) {
    return `Up to ${limit} per pool: raise the maximum runner count to add more`;
  }
  return null;
}

/** The plan preview's target check: the selected kind's sentence, or the default kind's on an empty selection. */
export function targetPlanCheck(descriptors: Descriptors, selection: ResolvedSelection, limits: TargetLimits, context?: TargetContext): { ok: boolean; text: string } {
  const kinds = targetKinds(descriptors)
    .map(({ kind }) => kind)
    .filter((kind) => kind.planNote && (!context || isOffered(kind, context)));
  const kind = kinds.find((candidate) => selectedCount(selection, candidate.id) > 0)
    ?? kinds.find((candidate) => candidate.defaultForPlan)
    ?? kinds[0];
  return kind?.planNote?.(selectedCount(selection, kind.id), limits) ?? { ok: true, text: "Targets define where the pool’s runners register." };
}

/** The create-button gate: some standalone kind has a connection in the already-loaded context. */
export function anyStandaloneKindAvailable(descriptors: Descriptors, context: TargetContext): boolean {
  return targetKinds(descriptors).some(({ kind }) => kind.standalone && kind.available(context));
}

/** What the new-pool page shows while no standalone kind is available. */
export function prerequisiteEmptyState(descriptors: Descriptors, context: TargetContext): PrerequisiteEmptyState | null {
  for (const platform of descriptors) {
    if (!platform.targetKinds.some((kind) => kind.standalone)) continue;
    const state = platform.prerequisiteEmptyState?.(context);
    if (state) return state;
  }
  return null;
}

/** The Maximum runner count's floor: one, or the number of selected targets that each need a runner. */
export function minMaxCount(descriptors: Descriptors, selection: ResolvedSelection): number {
  const counted = targetKinds(descriptors)
    .filter(({ kind }) => kind.raisesMinMaxCount)
    .reduce((total, { kind }) => total + selectedCount(selection, kind.id), 0);
  return Math.max(1, counted);
}

/** Providers the selected kinds need the pool to include. */
export function requiredProviders(descriptors: Descriptors, selection: ResolvedSelection): Array<"docker" | "tart"> {
  const providers = targetKinds(descriptors)
    .filter(({ kind }) => selectedCount(selection, kind.id) > 0)
    .flatMap(({ kind }) => kind.requiresProviders ?? []);
  return [...new Set(providers)];
}

/** Whether the form offers a kind with a Priority section. */
export function prioritySupported(descriptors: Descriptors, context: TargetContext): boolean {
  return targetKinds(descriptors).some(({ kind }) => kind.supportsPriority && isOffered(kind, context));
}

/** The selected targets the Priority section may prioritise. */
export function priorityCandidates(descriptors: Descriptors, selection: ResolvedSelection): TargetOption[] {
  return targetKinds(descriptors)
    .filter(({ kind }) => kind.supportsPriority)
    .flatMap(({ kind }) => [...(selection[kind.id] ?? [])]);
}

/** "Additional labels" rules of the targeted platforms (the first that declares help). */
export function labelRules(descriptors: Descriptors, selection: ResolvedSelection) {
  return targetedPlatforms(descriptors, selection).map((platform) => platform.labelRules).find((rules) => rules?.help) ?? null;
}

/** The first label problem any targeted platform reports, or null. */
export function labelError(descriptors: Descriptors, selection: ResolvedSelection, labels: string[]): string | null {
  for (const platform of targetedPlatforms(descriptors, selection)) {
    const error = platform.labelRules?.validate?.(labels);
    if (error) return error;
  }
  return null;
}

/** The pool's current targets on the settings form. */
export function initialSelection(descriptors: Descriptors, pool: RunnerPoolDetail): ResolvedSelection {
  return Object.assign({}, ...descriptors.map((platform) => platform.initialSelection?.(pool) ?? {}));
}

/**
 * The Platform options values a pool form starts from, one entry per platform that declares
 * `platformOptions.initialValues`: the pool's own on settings, the defaults on a new pool.
 */
export function initialPlatformOptionValues(descriptors: Descriptors, pool?: RunnerPoolDetail): PlatformOptionValues {
  return Object.fromEntries(descriptors.flatMap((platform) => {
    const initialValues = platform.platformOptions?.initialValues;
    return initialValues ? [[platform.id, initialValues(pool)]] : [];
  }));
}

export function initialPriorityKeys(descriptors: Descriptors, pool: RunnerPoolDetail): string[] {
  return descriptors.flatMap((platform) => platform.initialPriorityKeys?.(pool) ?? []);
}

/** One summary per platform the read-only pool view lists. */
export function readOnlySummaries(descriptors: Descriptors, pool: RunnerPoolDetail) {
  return descriptors.flatMap((platform) => {
    const value = platform.readOnlySummary?.(pool);
    return value ? [{ platform, value }] : [];
  });
}

/**
 * The target fields of a create payload. Every descriptor's partial is merged, whether or not its
 * platform has a selected target, because today's payload always carries each platform's defaults.
 */
export function buildCreatePayload(descriptors: Descriptors, selection: ResolvedSelection, context: TargetContext, values: TargetCreateValues = {}): PayloadPartial {
  return Object.assign({}, ...descriptors.map((platform) => platform.toCreatePayload(selection, context, values)));
}

/** The target fields of an update payload (no `poolId`: the settings form adds it). */
export function buildUpdatePayload(descriptors: Descriptors, selection: ResolvedSelection, pool: RunnerPoolDetail, values: TargetUpdateValues): PayloadPartial {
  return Object.assign({}, ...descriptors.map((platform) => platform.toUpdatePayload(selection, pool, values)));
}
