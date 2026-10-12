/** "Add machine": issue a one-time enrollment code and show how to run the host agent. */
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, LoaderCircle, Plus } from "lucide-react";
import { useState } from "react";

import { CopyButton } from "~/components/copy-button";
import { Callout, InlineError } from "~/components/page";
import { Button } from "~/components/ui/button";
import { Dialog, DialogTrigger, SheetContent } from "~/components/ui/dialog";
import { SearchableMultiSelect } from "~/components/ui/searchable-multi-select";
import { formatDateTime } from "~/lib/utils";
import { listTargets } from "./admin-api";
import { fleetErrorMessage, newIdempotencyKey } from "./fleet-ui";
import { issueEnrollment } from "./registry-api";

type Issued = { code: string; expiresAt: string };

export function AddMachineButton() {
  const [open, setOpen] = useState(false);
  const queryClient = useQueryClient();
  return (
    <Dialog
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) void queryClient.invalidateQueries({ queryKey: ["fleet", "hosts"] });
      }}
      open={open}
    >
      <DialogTrigger asChild>
        <Button size="sm"><Plus />Add machine</Button>
      </DialogTrigger>
      <SheetContent className="max-w-lg" description="Connect a Linux or macOS machine to run jobs." heading="Add machine">
        {/* Remounting on open drops the one-time code from memory once the sheet closes. */}
        {open ? <AddMachineFlow onDone={() => setOpen(false)} /> : null}
      </SheetContent>
    </Dialog>
  );
}

function AddMachineFlow({ onDone }: { onDone: () => void }) {
  const targets = useQuery({ queryKey: ["fleet", "targets"], queryFn: ({ signal }) => listTargets(signal) });
  const [targetIds, setTargetIds] = useState<string[]>([]);
  // One key per attempt: a retried request after a dropped response replays instead of issuing twice.
  const [key, setKey] = useState(newIdempotencyKey);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [issued, setIssued] = useState<Issued | null>(null);

  async function issue() {
    setPending(true);
    setError(null);
    try {
      const result = await issueEnrollment({ targetIds }, key);
      if (result.status === "created") {
        setIssued({ code: result.code, expiresAt: result.expiresAt });
      } else {
        setError("This code was already issued and can’t be shown again. Start over to get a new one.");
      }
    } catch (cause) {
      setError(fleetErrorMessage(cause, "The code could not be issued."));
    } finally {
      setPending(false);
    }
  }

  if (issued) return <EnrollmentInstructions issued={issued} onDone={onDone} />;

  return (
    <div className="space-y-5 p-4">
      <div className="space-y-1.5">
        <div className="text-sm font-medium text-foreground">Repositories and organizations</div>
        <p className="text-xs leading-5 text-muted-foreground">The machine may only be approved for the ones you pick here.</p>
        <SearchableMultiSelect
          ariaLabel="Repositories and organizations"
          emptyMessage="No repositories or organizations found"
          loading={targets.isPending}
          onValueChange={setTargetIds}
          options={(targets.data?.items ?? []).map((target) => ({
            value: target.id,
            label: target.label,
            description: target.kind === "github_organization" ? "Organization" : "Repository",
          }))}
          placeholder="Choose where this machine can run jobs…"
          maxSelected={64}
          searchPlaceholder="Search repositories and organizations…"
          selectAll
          selectedNoun="targets"
          values={targetIds}
        />
        {targets.isError ? <InlineError onRetry={() => void targets.refetch()} title="Couldn’t load repositories">{fleetErrorMessage(targets.error, "")}</InlineError> : null}
      </div>
      {error ? (
        <Callout action={<Button onClick={() => { setKey(newIdempotencyKey()); setError(null); }} size="sm" variant="ghost">Start over</Button>} title="No code issued" tone="danger">
          {error}
        </Callout>
      ) : null}
      <div className="flex justify-end gap-2">
        <Button onClick={onDone} variant="ghost">Cancel</Button>
        <Button disabled={pending || targetIds.length === 0} onClick={() => void issue()}>
          {pending ? <LoaderCircle className="animate-spin" /> : <KeyRound />}Get enrollment code
        </Button>
      </div>
    </div>
  );
}

function EnrollmentInstructions({ issued, onDone }: { issued: Issued; onDone: () => void }) {
  const script = [
    `export GRIDOPS_FLEET_URL=${window.location.origin}`,
    `export GRIDOPS_ENROLLMENT_CODE=${issued.code}`,
    "gridops-host-agent enroll",
    "gridops-host-agent run",
  ].join("\n");

  return (
    <div className="space-y-5 p-4">
      <Callout title="Copy this code now" tone="warning">
        It is shown only once and expires {formatDateTime(issued.expiresAt)}. It can be used to enroll one machine.
      </Callout>
      <div className="space-y-1.5">
        <div className="text-sm font-medium text-foreground" id="enrollment-code-label">Enrollment code</div>
        <div className="flex items-center gap-2 rounded-lg border border-border-strong bg-panel-subtle px-3 py-2">
          <code aria-labelledby="enrollment-code-label" className="min-w-0 flex-1 break-all font-mono text-xs text-foreground">{issued.code}</code>
          <CopyButton label="Enrollment code" size="icon-sm" value={issued.code} variant="ghost" />
        </div>
      </div>
      <div className="space-y-1.5">
        <div className="flex items-center justify-between gap-2">
          <div className="text-sm font-medium text-foreground">On the machine</div>
          <CopyButton label="Setup commands" size="sm" value={script} variant="outline">Copy commands</CopyButton>
        </div>
        <pre className="overflow-x-auto rounded-lg border border-border-strong bg-panel-subtle px-3 py-2.5 font-mono text-xs leading-5 text-foreground">{script}</pre>
        <p className="text-xs leading-5 text-muted-foreground">
          To run Docker jobs, list the images the machine may pull in <code className="font-mono">GRIDOPS_AGENT_IMAGES</code> (comma-separated) before <code className="font-mono">run</code>. The default is <code className="font-mono">ghcr.io/actions/actions-runner:latest</code>.
        </p>
      </div>
      <p className="text-sm text-muted-foreground">Once the agent enrolls, the machine appears in the list as pending. Open it to approve it.</p>
      <div className="flex justify-end">
        <Button onClick={onDone}>Done</Button>
      </div>
    </div>
  );
}
