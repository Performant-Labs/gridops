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
