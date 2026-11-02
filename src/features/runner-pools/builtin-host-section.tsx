import { LoaderCircle } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { Callout } from "~/components/page";
import { SettingsRow, SettingsSection, SettingsValue, Switch } from "~/components/settings-ui";
import { Link } from "@tanstack/react-router";
import { type BuiltinHostChange, builtinHostChangeMessage, saveBuiltinRunnerHost } from "./builtin-host";

/**
 * Settings → Runner host: whether GridOps may start runners on the machine it
 * runs on. Off is enforced where runners start, not only here.
 */
export function BuiltinHostSection({ enabled, isAdmin, dockerAvailable = true, managerSetting, onChanged }: {
  enabled: boolean;
  isAdmin: boolean;
  /** False when the runner manager runs without the Docker socket. */
  dockerAvailable?: boolean;
  /** What the runner manager says it has been told: `null` before GridOps has told it, `undefined` when unreachable. */
  managerSetting?: boolean | null;
  onChanged?: (result: BuiltinHostChange) => void | Promise<void>;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [last, setLast] = useState<BuiltinHostChange | null>(null);
  const shown = last?.builtinRunnerHostEnabled ?? enabled;

  async function change(next: boolean) {
    setPending(true);
    setError(null);
    try {
      const result = await saveBuiltinRunnerHost(next);
      setLast(result);
      toast.success(builtinHostChangeMessage(result));
      await onChanged?.(result);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not change the built-in runner host.");
    } finally {
      setPending(false);
    }
  }

  return (
    <SettingsSection
      description="The machine GridOps itself runs on. When this is on, pools that have no fleet execution, or that allow it in their placement, start Docker runners there."
      title="Built-in runner host"
    >
      <SettingsRow
        description={shown
          ? "Runners may start on this machine. Turn it off to run every pool on the enrolled machines under Fleet."
          : "GridOps starts no runner on the machine it runs on. The reconciler, the API and the runner manager all refuse to, and every pool runs on the enrolled machines under Fleet."}
        inline
        label="Use the built-in runner host"
      >
        {isAdmin ? (
          <span className="flex items-center gap-2">
            {pending ? <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" /> : null}
            <Switch aria-label="Use the built-in runner host" checked={shown} disabled={pending} onCheckedChange={(next) => void change(next)} />
          </span>
        ) : (
          <span className="text-sm text-foreground">{shown ? "On" : "Off"}</span>
        )}
      </SettingsRow>
      {!isAdmin ? <SettingsValue label="Who can change it" value="Administrators" /> : null}
      {error ? <div className="px-4 py-3"><Callout title="Could not change the setting" tone="danger">{error}</Callout></div> : null}
      {!shown ? (
        <div className="space-y-2 px-4 py-3">
          <Callout title="Off" tone="info">
            Runners already on this machine are not restarted: idle ones are removed and busy ones finish their job. A pool that has no
            fleet execution shows “Cannot run: built-in runner host is off” until you turn on Run on machines in its settings.
            The capacity budget and disk guardrails below apply to the host only while it is in use, so they are not shown.
            {" "}<Link className="underline underline-offset-2" to="/fleet">Machines</Link>
          </Callout>
          {last && last.cannotRunPools.length > 0 ? (
            <Callout title={`${last.cannotRunPools.length} ${last.cannotRunPools.length === 1 ? "pool cannot" : "pools cannot"} run`} tone="warning">
              {last.cannotRunPools.map((pool) => pool.name).join(", ")} — give {last.cannotRunPools.length === 1 ? "it" : "each"} fleet execution in the pool’s settings.
            </Callout>
          ) : null}
        </div>
      ) : null}
      {!dockerAvailable ? (
        <div className="px-4 py-3">
          <Callout title="The runner manager has no Docker access" tone="info">
            It runs without the Docker socket (<code className="font-mono">GRIDOPS_MANAGER_DOCKER=off</code>), so nothing can start on this machine whatever this setting says.
          </Callout>
        </div>
      ) : null}
      {managerSetting === null && shown === false ? (
        <SettingsValue label="Runner manager" value="Not told yet; it treats the host as off until the next reconcile pass" />
      ) : null}
      {typeof managerSetting === "boolean" && managerSetting !== shown ? (
        <SettingsValue label="Runner manager" value={`Still ${managerSetting ? "on" : "off"}; it catches up within one reconcile interval`} />
      ) : null}
    </SettingsSection>
  );
}
