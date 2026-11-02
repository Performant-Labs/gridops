/** The machine the built-in runner manager starts runners on, as its manager reports it. */
export type RunnerHost = { name: string | null; cpus: number | null; memoryMb: number | null } | null | undefined;

export function runnerHostLabel(host: RunnerHost) {
  return host?.name ? `the built-in runner host (${host.name})` : "the built-in runner host";
}

/** What share of the host the budget is, so an operator sees it is a setting, not the hardware. */
export function runnerHostBudgetNote(host: RunnerHost, cpuBudget: number, memoryBudgetMb: number) {
  if (!host?.cpus || !host.memoryMb) return null;
  const percent = (budget: number, total: number) => Math.round((budget / total) * 100);
  return `That is ${percent(cpuBudget, host.cpus)}% of its ${formatResourceNumber(host.cpus)} cores and ${percent(memoryBudgetMb, host.memoryMb)}% of its ${host.memoryMb.toLocaleString()} MB.`;
}

export type HostResourceWarning = {
  cpuRequested: number;
  cpuBudget: number;
  memoryRequestedMb: number;
  memoryBudgetMb: number;
  runnerCount: number;
};

export function hostResourceWarning(input: {
  cpuLimit: number;
  cpuBudget: number;
  memoryBudgetMb: number;
  memoryLimitMb: number;
  runnerCount: number;
}): HostResourceWarning | null {
  const values = [
    input.cpuLimit,
    input.cpuBudget,
    input.memoryBudgetMb,
    input.memoryLimitMb,
    input.runnerCount,
  ];
  if (values.some((value) => !Number.isFinite(value) || value <= 0)) return null;

  const cpuRequested = input.cpuLimit * input.runnerCount;
  const memoryRequestedMb = input.memoryLimitMb * input.runnerCount;
  if (cpuRequested <= input.cpuBudget && memoryRequestedMb <= input.memoryBudgetMb) return null;

  return { ...input, cpuRequested, memoryRequestedMb };
}

export function formatResourceNumber(value: number) {
  return Number.isInteger(value) ? String(value) : value.toFixed(1).replace(/\.0$/, "");
}
