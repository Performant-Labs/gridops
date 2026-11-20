import { CircleCheck, CircleX } from "lucide-react";

import { Badge } from "~/components/ui/badge";
import { cn } from "~/lib/utils";

type Provider = "docker" | "tart";

/** Preflight summary of the labels each provider registers and whether capacity fits. */
export function PoolPlanPreview({
  name,
  providers,
  labels,
  desiredCount,
  maxCount,
  targetCheck,
}: {
  name: string;
  providers: Provider[];
  labels: string[];
  desiredCount: number;
  maxCount: number;
  /** Whether the pool's targets fit, in one sentence; computed from the target kinds by the form. */
  targetCheck: { ok: boolean; text: string };
}) {
  const normalizedName = name.trim();
  const customLabels = labels.filter(Boolean);
  const repositoryCapacityOk = targetCheck.ok;
  const ready = Boolean(normalizedName) && providers.length > 0 && repositoryCapacityOk;
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium">Preflight</span>
        <Badge dot={ready ? "bg-success" : "bg-warning"}>{ready ? "Ready to save" : "Needs attention"}</Badge>
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        {providers.map((provider) => (
          <div className="rounded-md border border-border bg-panel-subtle p-2.5" key={provider}>
            <p className="text-xs font-medium text-secondary-foreground">{provider === "tart" ? "macOS runners register as" : "Linux runners register as"}</p>
            <div className="mt-1.5 flex flex-wrap gap-1">
              {(provider === "tart" ? ["self-hosted", "macOS", "ARM64"] : ["self-hosted", "Linux", "host arch"])
                .concat(normalizedName ? [normalizedName] : [])
                .concat(customLabels)
                .map((label) => <Badge key={label} variant="outline">{label}</Badge>)}
            </div>
          </div>
        ))}
      </div>
      <div className="space-y-1 text-xs">
        <PlanCheck ok={targetCheck.ok} text={targetCheck.text} />
        <PlanCheck ok={desiredCount <= maxCount} text={`${desiredCount} runners targeted now; autoscaling may grow to ${maxCount}.`} />
      </div>
      <p className="text-2xs leading-4 text-faint">This preview reserves nothing; host capacity is checked right before each runner starts.</p>
    </div>
  );
}

function PlanCheck({ ok, text }: { ok: boolean; text: string }) {
  const Icon = ok ? CircleCheck : CircleX;
  return <div className="flex items-start gap-2 text-muted-foreground"><Icon className={cn("mt-px size-3.5 shrink-0", ok ? "text-success" : "text-danger")} /><span>{text}</span></div>;
}
