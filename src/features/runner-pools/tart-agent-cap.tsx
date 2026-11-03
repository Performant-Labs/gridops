import { Callout } from "~/components/page";
import { SettingsRow, UnitField } from "~/components/settings-ui";
import { Input } from "~/components/ui/input";

/**
 * The warning for a Tart agent whose own runner cap (`GRIDOPS_TART_MAX_RUNNERS`, its health's
 * `capacity.maxRunners`) is below the "macOS VM slots on the Tart host" setting. The lower of the
 * two wins: the runner manager admits a start and the agent turns it away. A cap at or above the
 * setting needs no warning, and neither does a deployment without a Tart agent.
 */
export function tartAgentCapWarning(
  slots: number | undefined,
  agentMaxRunners: number | null | undefined,
): { title: string; body: string } | null {
  if (slots == null || agentMaxRunners == null || agentMaxRunners >= slots) return null;
  const runners = `${agentMaxRunners} runner${agentMaxRunners === 1 ? "" : "s"}`;
  return {
    title: `The Tart agent allows ${runners}; slots are set to ${slots}`,
    body: `GridOps uses at most ${agentMaxRunners} of the Mac's ${slots} slots: the runner manager admits a start that the agent then turns away. `
      + `Raise GRIDOPS_TART_MAX_RUNNERS to ${slots} on the Mac and run scripts/install-tart-agent.sh again, or set the slots to ${agentMaxRunners}.`,
  };
}

/** The warning as a callout, or nothing. */
export function TartAgentCapWarning({ slots, agentMaxRunners }: { slots?: number; agentMaxRunners?: number | null }) {
  const warning = tartAgentCapWarning(slots, agentMaxRunners);
  if (!warning) return null;
  return <Callout className="mt-2 text-left first:mt-0" title={warning.title} tone="warning">{warning.body}</Callout>;
}

/** The "macOS VM slots on the Tart host" field on Settings → General, with the agent cap warning under its description. */
export function MacosVmSlotsRow({ slots, agentMaxRunners }: { slots: number; agentMaxRunners?: number | null }) {
  return (
    <SettingsRow
      description={(
        <>
          How many macOS VMs GridOps lets run at once on the Tart Mac, counting VMs other software runs there. macOS allows at most two; waiting pools get a free slot in turn.
          <TartAgentCapWarning agentMaxRunners={agentMaxRunners} slots={slots} />
        </>
      )}
      htmlFor="macos-vm-slots"
      label="macOS VM slots on the Tart host"
    >
      <UnitField unit="VMs"><Input className="pr-12" defaultValue={slots} id="macos-vm-slots" max={2} min={1} name="macosVmSlots" required type="number" /></UnitField>
    </SettingsRow>
  );
}
