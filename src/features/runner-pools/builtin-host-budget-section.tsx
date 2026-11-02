/** Settings → Runner host: how much of the machine GridOps runs on it may fill with runners. */
import { LoaderCircle } from "lucide-react";
import { type FormEvent, useState } from "react";
import { toast } from "sonner";

import { SettingsSection, SettingsValue, UnitField } from "~/components/settings-ui";
import { Button } from "~/components/ui/button";
import { Input } from "~/components/ui/input";
import { formatCores, formatMib } from "~/features/fleet/fleet-ui";
import {
  type BudgetInForce, type BudgetSource, type BuiltinBudgetDraft, type BuiltinBudgetRequest, type BuiltinHostBudget, type HostFigures,
  budgetSourceLabel, builtinBudgetDraft, builtinBudgetRequest, builtinBudgetSavedMessage, saveBuiltinHostBudget,
} from "./builtin-host";

export function BuiltinHostBudgetSection({ isAdmin, inForce, reported, setting, source, onSaved }: {
  isAdmin: boolean;
  /** The budget the runner manager admits runners against now. */
  inForce: BudgetInForce;
  /** What the host has; `null` when the manager has not reported it. */
  reported: HostFigures | null;
  /** What is stored in GridOps; a `null` figure uses the runner manager's own. */
  setting: BuiltinHostBudget;
  source?: { cpu: BudgetSource; memory: BudgetSource };
  onSaved?: () => Promise<unknown> | void;
}) {
  return (
    <SettingsSection
      description="The most of this machine GridOps may fill with runners, like a fleet machine’s budget. Runners that would not fit wait; lowering it never stops a running job. A figure not set here comes from the runner manager (its environment, or 95% of the host)."
      title="Budget"
    >
      {isAdmin && reported ? (
        // Keyed by the figures in force, so a change made elsewhere replaces the form's.
        <BudgetForm inForce={inForce} key={JSON.stringify([inForce, setting])} onSaved={onSaved} reported={reported} setting={setting} source={source} />
      ) : (
        <>
          <SettingsValue label="CPU" value={`${formatCores(Math.round(inForce.cpuBudget * 1000))} · ${budgetSourceLabel(source?.cpu, "cpu")}`} />
          <SettingsValue label="Memory" value={`${formatMib(inForce.memoryBudgetMb)} · ${budgetSourceLabel(source?.memory, "memory")}`} />
          <SettingsValue label="Who can change it" value={isAdmin ? "Administrators, once the runner manager reports its host" : "Administrators"} />
        </>
      )}
    </SettingsSection>
  );
}

function BudgetForm({ inForce, reported, setting, source, onSaved }: {
  inForce: BudgetInForce; reported: HostFigures; setting: BuiltinHostBudget;
  source?: { cpu: BudgetSource; memory: BudgetSource }; onSaved?: () => Promise<unknown> | void;
}) {
  const [draft, setDraft] = useState<BuiltinBudgetDraft>(() => builtinBudgetDraft(inForce));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  async function send(request: BuiltinBudgetRequest) {
    setError(null);
    setSaving(true);
    try {
      const result = await saveBuiltinHostBudget(request);
      toast.success(builtinBudgetSavedMessage(result));
      await onSaved?.();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : "The budget wasn’t saved.");
    } finally {
      setSaving(false);
    }
  }

  function save(event: FormEvent) {
    event.preventDefault();
    const outcome = builtinBudgetRequest(inForce, draft, reported);
    if (!outcome.ok) { setError(outcome.error); return; }
    void send(outcome.request);
  }

  const dirty = JSON.stringify(draft) !== JSON.stringify(builtinBudgetDraft(inForce));
  const edit = (field: keyof BuiltinBudgetDraft) => (event: { target: { value: string } }) => {
    setDraft({ ...draft, [field]: event.target.value });
    setError(null);
  };
  const isSet = setting.cpuMillis !== null || setting.memoryMib !== null;

  return (
    <form className="space-y-3 px-4 py-3.5" onSubmit={save}>
      <div className="grid gap-3 sm:grid-cols-2">
        <BudgetField hint={`Host reports ${formatCores(Math.round(reported.cpus * 1000))} · ${budgetSourceLabel(source?.cpu, "cpu")}`} id="builtin-budget-cpu" label="CPU" onChange={edit("cores")} unit="cores" value={draft.cores} />
        <BudgetField hint={`Host reports ${formatMib(reported.memoryMb)} · ${budgetSourceLabel(source?.memory, "memory")}`} id="builtin-budget-memory" label="Memory" onChange={edit("memoryGb")} unit="GB" value={draft.memoryGb} />
      </div>
      {error ? <p className="text-sm text-danger" role="alert">{error}</p> : null}
      <div className="flex flex-wrap gap-2">
        <Button disabled={saving || !dirty} size="sm" type="submit" variant="outline">
          {saving ? <LoaderCircle className="animate-spin" /> : null}Save budget
        </Button>
        {isSet ? (
          <Button disabled={saving} onClick={() => void send({ cpuMillis: null, memoryMib: null })} size="sm" type="button" variant="ghost">
            Use the runner manager’s own figures
          </Button>
        ) : null}
      </div>
    </form>
  );
}

function BudgetField({ id, label, unit, value, hint, onChange }: {
  id: string; label: string; unit: string; value: string; hint: string; onChange: (event: { target: { value: string } }) => void;
}) {
  return (
    <div className="space-y-1">
      <label className="text-xs font-medium text-foreground" htmlFor={id}>{label}</label>
      <UnitField unit={unit}>
        <Input className="pr-14" id={id} inputMode="decimal" min="0" onChange={onChange} step="any" type="number" value={value} />
      </UnitField>
      <p className="text-xs text-muted-foreground">{hint}</p>
    </div>
  );
}
