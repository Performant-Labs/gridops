/** The form behind a machine's physical budget: cores and GB in, the API's whole numbers out. */
import type { BudgetRequest } from "./admin-contracts";

const GIB = 1024 ** 3;

export type BudgetFigures = { cpuMillis: number; memoryMib: number; diskBytes: number };
/** What the machine itself reports; the budget can't exceed it. `null` means it has not reported that figure. */
export type ReportedFigures = { cpuMillis: number | null; memoryMib: number | null; diskBytes: number | null };
export type BudgetDraft = { cores: string; memoryGb: string; diskGb: string };

const plain = (value: number) => String(Number(value.toFixed(2)));

export const budgetDraft = (budget: BudgetFigures): BudgetDraft => ({
  cores: plain(budget.cpuMillis / 1000),
  memoryGb: plain(budget.memoryMib / 1024),
  diskGb: plain(budget.diskBytes / GIB),
});

export type BudgetOutcome = { ok: true; request: BudgetRequest } | { ok: false; error: string };

/**
 * Turns the form into the request. Only a figure the operator edited is sent,
 * so a display-rounded value is never written back by accident.
 */
export function budgetRequest(current: BudgetFigures, draft: BudgetDraft, reported: ReportedFigures): BudgetOutcome {
  const shown = budgetDraft(current);
  const request: BudgetRequest = {};
  const figures = [
    { key: "cpuMillis", label: "CPU", text: draft.cores, was: shown.cores, scale: 1000, limit: reported.cpuMillis, unit: "cores", noun: "CPU" },
    { key: "memoryMib", label: "Memory", text: draft.memoryGb, was: shown.memoryGb, scale: 1024, limit: reported.memoryMib, unit: "GB", noun: "memory" },
    { key: "diskBytes", label: "Disk", text: draft.diskGb, was: shown.diskGb, scale: GIB, limit: reported.diskBytes, unit: "GB", noun: "disk" },
  ] as const;
  for (const figure of figures) {
    if (figure.text.trim() === figure.was) continue;
    const entered = Number(figure.text);
    if (figure.text.trim() === "" || !Number.isFinite(entered) || entered <= 0) {
      return { ok: false, error: `${figure.label} must be a number greater than zero.` };
    }
    if (figure.limit === null || figure.limit <= 0) {
      return { ok: false, error: `This machine hasn’t reported its ${figure.noun}, so its budget can’t be set yet.` };
    }
    const wanted = Math.round(entered * figure.scale);
    if (wanted < 1) return { ok: false, error: `${figure.label} must be a number greater than zero.` };
    if (wanted > figure.limit) {
      return { ok: false, error: `${figure.label} can’t exceed the ${plain(figure.limit / figure.scale)} ${figure.unit} this machine reports.` };
    }
    request[figure.key] = wanted;
  }
  return Object.keys(request).length === 0 ? { ok: false, error: "Nothing to save." } : { ok: true, request };
}
