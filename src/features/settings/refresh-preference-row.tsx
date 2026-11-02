import { useId } from "react";

import { SettingsRow } from "~/components/settings-ui";
import { REFRESH_OPTIONS } from "~/lib/refresh-interval";
import { useRefreshPreference } from "~/lib/use-refresh-interval";
import { cn } from "~/lib/utils";

/** Settings -> Preferences: how often live pages reload in this browser. */
export function RefreshPreferenceRow() {
  const [preference, setPreference] = useRefreshPreference();
  const name = useId();
  return (
    <SettingsRow
      description={
        <>
          How often the Overview and other live pages reload their data in this browser. Pages that already update faster, such as a running
          workflow run, stay proportionally faster, but never more often than every 3 seconds. Off stops automatic updates; pages then show a
          Refresh button and the time of the last update.
        </>
      }
      label="Refresh"
      stacked
    >
      <div aria-label="Refresh interval" className="flex flex-wrap rounded-md border border-border-strong p-0.5" role="radiogroup">
        {REFRESH_OPTIONS.map((option) => (
          <label
            className={cn(
              "relative flex h-7 flex-1 cursor-pointer items-center justify-center whitespace-nowrap rounded-[4px] px-2.5 text-xs font-medium transition-colors has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring/50",
              preference === option.value ? "bg-selected text-foreground" : "text-muted-foreground hover:text-foreground",
            )}
            key={option.value}
          >
            <input
              checked={preference === option.value}
              className="sr-only"
              name={name}
              onChange={() => setPreference(option.value)}
              type="radio"
              value={option.value}
            />
            {option.label}
          </label>
        ))}
      </div>
    </SettingsRow>
  );
}
