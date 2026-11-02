/** The built-in runner host: the machine GridOps itself runs on. An instance setting turns it off. */
import { api } from "~/lib/api";

/** Why a pool cannot run, as the API reports it. */
export type CannotRun = { code: string; message: string } | null;

/** `state` of a pool whose only place to run is the built-in host while it is off. */
export const BUILTIN_HOST_OFF_STATE = "builtin-host-off";
export const BUILTIN_HOST_OFF_LABEL = "Cannot run: built-in runner host is off";

export type BuiltinHostChange = {
  ok: true;
  builtinRunnerHostEnabled: boolean;
  changed: boolean;
  /** Whether the runner manager already enforces the new setting; the reconciler catches it up otherwise. */
  managerSynced: boolean;
  /** Pools with no fleet execution, which cannot run while the host is off. */
  cannotRunPools: Array<{ id: string; name: string }>;
  /** Live runners still on the host. */
  runnersOnBuiltinHost: number;
};

export const saveBuiltinRunnerHost = (enabled: boolean) =>
  api<BuiltinHostChange>("/api/v1/settings/runner-host", { method: "PUT", body: { builtinHostEnabled: enabled } });

/** A pool the built-in host's absence leaves with nowhere to run. */
export function isBuiltinHostOff(pool: { state: string; cannotRun?: CannotRun }): boolean {
  return Boolean(pool.cannotRun) || pool.state === BUILTIN_HOST_OFF_STATE;
}

/** `undefined` (an older server) counts as on, which is how it behaved. */
export const builtinHostOn = (enabled: boolean | undefined) => enabled !== false;

/** The host's capacity meters, budget warning and disk guardrail describe a host that is being used. */
export const showsBuiltinGuardrails = builtinHostOn;

/** What to tell the administrator once the setting has been saved. */
export function builtinHostChangeMessage(result: BuiltinHostChange): string {
  if (!result.changed) return `The built-in runner host was already ${result.builtinRunnerHostEnabled ? "on" : "off"}.`;
  if (result.builtinRunnerHostEnabled) return "Built-in runner host turned on.";
  const pools = result.cannotRunPools.length;
  const parts = ["Built-in runner host turned off."];
  if (pools > 0) parts.push(`${pools} ${pools === 1 ? "pool has" : "pools have"} no fleet execution and cannot run: ${result.cannotRunPools.map((pool) => pool.name).join(", ")}.`);
  if (result.runnersOnBuiltinHost > 0) parts.push(`${result.runnersOnBuiltinHost} ${result.runnersOnBuiltinHost === 1 ? "runner is" : "runners are"} still on the host; idle ones are removed and busy ones finish their job first.`);
  if (!result.managerSynced) parts.push("The runner manager has not confirmed yet; it refuses to start runners on the host until it does.");
  return parts.join(" ");
}

/** The built-in host's budget as set in GridOps; `null` uses the runner manager's own figure. */
export type BuiltinHostBudget = { cpuMillis: number | null; memoryMib: number | null };
/** Where a figure of the budget in force comes from. */
export type BudgetSource = "setting" | "environment" | "default";
/** The budget in force, as the runner manager reports it. */
export type BudgetInForce = { cpuBudget: number; memoryBudgetMb: number };
/** What the host has: the budget can't exceed it. */
export type HostFigures = { cpus: number; memoryMb: number };
export type BuiltinBudgetDraft = { cores: string; memoryGb: string };
export type BuiltinBudgetRequest = { cpuMillis?: number | null; memoryMib?: number | null };

export type BuiltinBudgetChange = {
  ok: true;
  budget: BuiltinHostBudget;
  changed: boolean;
  managerSynced: boolean;
  effective: null | BudgetInForce & { maxRunners: number; source: { cpu: BudgetSource; memory: BudgetSource } };
};

export const saveBuiltinHostBudget = (request: BuiltinBudgetRequest) =>
  api<BuiltinBudgetChange>("/api/v1/settings/runner-host/budget", { method: "PUT", body: request });

const plain = (value: number) => String(Number(value.toFixed(2)));

export const builtinBudgetDraft = (budget: BudgetInForce): BuiltinBudgetDraft => ({
  cores: plain(budget.cpuBudget),
  memoryGb: plain(budget.memoryBudgetMb / 1024),
});

/**
 * Turns the form into the request. Only a figure the operator edited is sent,
 * so a display-rounded value is never written back by accident.
 */
export function builtinBudgetRequest(
  inForce: BudgetInForce, draft: BuiltinBudgetDraft, host: HostFigures,
): { ok: true; request: BuiltinBudgetRequest } | { ok: false; error: string } {
  const shown = builtinBudgetDraft(inForce);
  const request: BuiltinBudgetRequest = {};
  const figures = [
    { key: "cpuMillis", label: "CPU", text: draft.cores, was: shown.cores, scale: 1000, limit: Math.round(host.cpus * 1000), unit: "cores" },
    { key: "memoryMib", label: "Memory", text: draft.memoryGb, was: shown.memoryGb, scale: 1024, limit: host.memoryMb, unit: "GB" },
  ] as const;
  for (const figure of figures) {
    if (figure.text.trim() === figure.was) continue;
    const entered = Number(figure.text);
    const wanted = Math.round(entered * figure.scale);
    if (figure.text.trim() === "" || !Number.isFinite(entered) || wanted < 1) {
      return { ok: false, error: `${figure.label} must be a number greater than zero.` };
    }
    if (wanted > figure.limit) {
      return { ok: false, error: `${figure.label} can’t exceed the ${plain(figure.limit / figure.scale)} ${figure.unit} this host reports.` };
    }
    request[figure.key] = wanted;
  }
  return Object.keys(request).length === 0 ? { ok: false, error: "Nothing to save." } : { ok: true, request };
}

/** Where a figure of the budget in force comes from, in words. */
export function budgetSourceLabel(source: BudgetSource | undefined, figure: "cpu" | "memory"): string {
  if (source === "setting") return "Set here";
  if (source === "environment") return `From ${figure === "cpu" ? "GRIDOPS_RUNNER_CPU_BUDGET" : "GRIDOPS_RUNNER_MEMORY_BUDGET_MB"} on the runner manager`;
  if (source === "default") return "Default: 95% of the host";
  return "Set on the runner manager";
}

/** What to tell the administrator once the budget has been saved. */
export const builtinBudgetSavedMessage = (result: BuiltinBudgetChange) =>
  result.managerSynced ? "Budget saved." : "Budget saved; the runner manager has not confirmed yet and picks it up within one reconcile interval.";
