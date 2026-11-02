import { Callout } from "~/components/page";

/** Shown when creating a pool while the built-in runner host is off. */
export function BuiltinHostOffNotice() {
  return (
    <Callout title="The built-in runner host is off" tone="info">
      Pools run on your enrolled machines only. This pool starts no runner until you turn on <strong>Run on machines</strong> in its settings,
      which GridOps opens for you right after you create it.
    </Callout>
  );
}
