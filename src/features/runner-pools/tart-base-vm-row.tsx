import { SettingsRow } from "~/components/settings-ui";
import { Input } from "~/components/ui/input";

/**
 * The Tart base VM a pool's macOS runners clone, shared by the create and
 * settings forms so both ask for it the same way. Shown only while the pool
 * includes macOS · Tart with the VM runtime; native execution clones no VM.
 */
export function TartBaseVmRow({ value, onChange, bitbucket = false }: { value: string; onChange: (value: string) => void; bitbucket?: boolean }) {
  return (
    <SettingsRow
      description={(
        <>
          The name of a stopped local Tart VM on the Mac, not an OCI reference, that each runner clones.{" "}
          {bitbucket
            ? <>For Bitbucket workspaces, prepare it with <code className="font-mono">scripts/prepare-tart-bitbucket-runner-image.sh</code>.</>
            : <>Prepare it with <code className="font-mono">scripts/prepare-tart-runner-image.sh</code>; xcodebuild needs an Xcode-ready VM.</>}
        </>
      )}
      htmlFor="tart-image"
      label="Tart base VM"
    >
      <Input className="font-mono text-xs" id="tart-image" name="tartImage" onChange={(event) => onChange(event.target.value)} required value={value} />
    </SettingsRow>
  );
}
