/** Which group of the runner pool list a pool is shown in. */
import { type CannotRun, isBuiltinHostOff } from "~/features/runner-pools/builtin-host";

export type PoolGroupKey = "attention" | "waiting" | "changing" | "active" | "paused";

type GroupedPool = {
  paused: boolean;
  state: string;
  provisionCircuitOpen: boolean;
  failedRunners: number;
  outdatedRunners: number;
  cannotRun?: CannotRun;
};

/**
 * A real block comes first (paused, provisioning paused, built-in host off). A pool the server
 * shows `waiting` is listed as waiting even with earlier failures or an open provisioning
 * circuit, because a wait for capacity or a macOS VM slot is not a failure (#102); its row still
 * carries the circuit's badge.
 */
export function poolGroup(pool: GroupedPool): PoolGroupKey {
  if (pool.paused || pool.state === "paused" || pool.state === "provisioning-paused") return "paused";
  if (isBuiltinHostOff(pool)) return "attention";
  if (pool.state === "waiting") return "waiting";
  if (pool.provisionCircuitOpen || pool.failedRunners > 0 || pool.state === "blocked") return "attention";
  if (pool.state === "backoff") return "waiting";
  if (["scaling", "updating", "draining"].includes(pool.state) || pool.outdatedRunners > 0) return "changing";
  return "active";
}
