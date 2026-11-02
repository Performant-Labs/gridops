import { Link } from "@tanstack/react-router";

import { Callout } from "~/components/page";
import { type HostResourceWarning, type RunnerHost, formatResourceNumber, runnerHostBudgetNote, runnerHostLabel } from "./resource-risk";

/**
 * The target would not fit the built-in runner host. Says which host that is,
 * what budget it is measured against, and where that budget is set, since it
 * is not the machine page an operator may be looking at.
 */
export function HostCapacityWarning({ warning, host }: { warning: HostResourceWarning; host: RunnerHost }) {
  const label = runnerHostLabel(host);
  const note = runnerHostBudgetNote(host, warning.cpuBudget, warning.memoryBudgetMb);
  return (
    <Callout title={`Target exceeds ${label}’s safe capacity`} tone="warning">
      {warning.runnerCount} runners would request {formatResourceNumber(warning.cpuRequested)} cores and {warning.memoryRequestedMb.toLocaleString()} MB;
      {" "}{label} safely offers {formatResourceNumber(warning.cpuBudget)} cores and {warning.memoryBudgetMb.toLocaleString()} MB. {note}{" "}
      An administrator sets that budget under <Link className="underline underline-offset-2" to="/settings/runner-host">Settings → Runner host</Link> (Budget);
      {" "}a figure not set there comes from the runner manager (<code className="font-mono">GRIDOPS_RUNNER_CPU_BUDGET</code> and <code className="font-mono">GRIDOPS_RUNNER_MEMORY_BUDGET_MB</code>, or 95% of the host).
      {" "}Runners started on a <Link className="underline underline-offset-2" to="/fleet">machine</Link> count
      against that machine’s own budget instead. You can save, but GridOps won’t start runners beyond the budget.
    </Callout>
  );
}
