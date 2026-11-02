/** Machine page: how much of the machine GridOps may fill with runners. */
import { useQuery } from "@tanstack/react-query";
import { LoaderCircle } from "lucide-react";
import { type FormEvent, useState } from "react";
import { toast } from "sonner";

import { Callout, InlineLoading, SectionHeading } from "~/components/page";
import { UnitField } from "~/components/settings-ui";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { setHostBudget } from "./admin-api";
import { fleetErrorMessage, formatBytes, formatCores, formatMib, isForbidden } from "./fleet-ui";
import { type BudgetDraft, type BudgetFigures, type ReportedFigures, budgetDraft, budgetRequest } from "./machine-budget";
import { listDomains } from "./query-api";

/** Reported capacity of the machine's physical (root) domain, `null` where it has not reported a figure. */
function reportedFigures(capacity: { state: string; cpuMillis?: number | null; memoryMib?: number | null; diskBytes?: number | null }): ReportedFigures {
  if (capacity.state === "missing") return { cpuMillis: null, memoryMib: null, diskBytes: null };
  return { cpuMillis: capacity.cpuMillis ?? null, memoryMib: capacity.memoryMib ?? null, diskBytes: capacity.diskBytes ?? null };
}

export function MachineBudgetSection({ hostId, budget, onSaved }: { hostId: string; budget: BudgetFigures; onSaved: () => Promise<unknown> | void }) {
  const domains = useQuery({
    queryKey: ["fleet", "domains", hostId],
    queryFn: ({ signal }) => listDomains(hostId, { limit: 100 }, signal),
    refetchInterval: 30_000,
  });
  if (domains.isError && isForbidden(domains.error)) return null;
  const root = domains.data?.items.find((domain) => domain.parentId === null);
  const reported = root ? reportedFigures(root.observedCapacity) : null;

  return (
    <section className="space-y-3">
      <SectionHeading>Budget</SectionHeading>
      <p className="text-xs text-muted-foreground">
        The most of this machine GridOps may fill with runners. A new machine starts at 80% of what it reports, which leaves room for the machine itself.
        Runners that would not fit wait. Lowering the budget below what is in use never stops a running job; nothing new starts until there is room.
      </p>
      {domains.isPending ? <InlineLoading label="Loading…" /> : !reported ? (
        domains.isError ? (
          <Callout action={<Button onClick={() => void domains.refetch()} size="sm" variant="outline">Retry</Button>} title="Couldn’t load the machine’s capacity" tone="danger">{fleetErrorMessage(domains.error, "")}</Callout>
        ) : <p className="text-sm text-muted-foreground">This machine hasn’t reported its capacity yet.</p>
      ) : (
        // Keyed by the stored budget, so a change made elsewhere replaces the form's figures.
        <BudgetForm budget={budget} hostId={hostId} key={JSON.stringify(budget)} onSaved={onSaved} reported={reported} />
      )}
    </section>
  );
}

function BudgetForm({ hostId, budget, reported, onSaved }: { hostId: string; budget: BudgetFigures; reported: ReportedFigures; onSaved: () => Promise<unknown> | void }) {
  const [draft, setDraft] = useState<BudgetDraft>(() => budgetDraft(budget));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function save(event: FormEvent) {
    event.preventDefault();
    const outcome = budgetRequest(budget, draft, reported);
    if (!outcome.ok) { setError(outcome.error); return; }
    setError(null);
    setSaving(true);
    try {
      await setHostBudget(hostId, outcome.request);
      toast.success("Budget saved.");
      await onSaved();
    } catch (failure) {
      setError(fleetErrorMessage(failure, "The budget wasn’t saved."));
    } finally {
      setSaving(false);
    }
  }
  const dirty = JSON.stringify(draft) !== JSON.stringify(budgetDraft(budget));
  const edit = (field: keyof BudgetDraft) => (event: { target: { value: string } }) => {
    setDraft({ ...draft, [field]: event.target.value });
    setError(null);
  };

  return (
    <form className="space-y-3" onSubmit={(event) => void save(event)}>
      <div className="grid gap-3 sm:grid-cols-3">
        <BudgetField id="budget-cpu" label="CPU" limit={reported.cpuMillis === null ? null : formatCores(reported.cpuMillis)} onChange={edit("cores")} unit="cores" value={draft.cores} />
        <BudgetField id="budget-memory" label="Memory" limit={reported.memoryMib === null ? null : formatMib(reported.memoryMib)} onChange={edit("memoryGb")} unit="GB" value={draft.memoryGb} />
        <BudgetField id="budget-disk" label="Disk" limit={reported.diskBytes === null ? null : formatBytes(reported.diskBytes)} onChange={edit("diskGb")} unit="GB" value={draft.diskGb} />
      </div>
      {error ? <p className="text-sm text-danger" role="alert">{error}</p> : null}
      <Button disabled={saving || !dirty} size="sm" type="submit" variant="outline">
        {saving ? <LoaderCircle className="animate-spin" /> : null}Save budget
      </Button>
    </form>
  );
}

function BudgetField({ id, label, unit, value, limit, onChange }: {
  id: string; label: string; unit: string; value: string; limit: string | null; onChange: (event: { target: { value: string } }) => void;
}) {
  return (
    <div className="space-y-1">
      <label className="text-xs font-medium text-foreground" htmlFor={id}>{label}</label>
      <UnitField unit={unit}>
        <Input className="pr-14" disabled={limit === null} id={id} inputMode="decimal" min="0" onChange={onChange} step="any" type="number" value={value} />
      </UnitField>
      <p className="text-xs text-muted-foreground">{limit === null ? "Not reported" : `Machine reports ${limit}`}</p>
    </div>
  );
}
