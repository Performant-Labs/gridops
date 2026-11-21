/**
 * The Platform options section of the runner pool forms (#145): collapsed by default, rendered only
 * for the platforms the pool targets, and filled from each descriptor's `platformOptions`.
 */
import { ChevronRight } from "lucide-react";
import { useId, useState } from "react";

import { platforms } from "~/features/platform-connections/registry";
import type { PlatformDescriptor, ResolvedSelection } from "~/features/platform-connections/types";
import { targetedPlatforms } from "~/features/runner-pools/targets";
import { cn } from "~/lib/utils";

export function PlatformOptionsSection({ descriptors = platforms, selection }: { descriptors?: readonly PlatformDescriptor[]; selection: ResolvedSelection }) {
  const [open, setOpen] = useState(false);
  const contentId = useId();
  const targeted = targetedPlatforms(descriptors, selection).filter((platform) => platform.platformOptions);
  if (!targeted.length) return null;

  return (
    <section>
      <div className="overflow-hidden rounded-lg border border-border bg-panel">
        <button
          aria-controls={contentId}
          aria-expanded={open}
          className="flex w-full items-center gap-2 px-4 py-3 text-left hover:bg-hover"
          onClick={() => setOpen((current) => !current)}
          type="button"
        >
          <ChevronRight className={cn("size-4 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} />
          <span className="text-base font-medium text-foreground">Platform options</span>
          <span className="truncate text-sm text-muted-foreground">{targeted.map((platform) => platform.name).join(", ")}</span>
        </button>
        {open ? (
          <div className="divide-y divide-border border-t border-border" id={contentId}>
            {targeted.map((platform) => {
              const options = platform.platformOptions;
              if (!options) return null;
              const Icon = platform.icon;
              const Extra = options.Component;
              return (
                <div className="space-y-2 px-4 py-3.5" key={platform.id}>
                  <div className="flex items-center gap-2 text-sm font-medium text-foreground"><Icon className="size-4 text-muted-foreground" />{platform.name}</div>
                  {options.help.length ? (
                    <ul className="list-disc space-y-1 pl-5 text-xs leading-5 text-muted-foreground">
                      {options.help.map((item, index) => <li key={index}>{item}</li>)}
                    </ul>
                  ) : null}
                  {Extra ? <Extra /> : null}
                </div>
              );
            })}
          </div>
        ) : null}
      </div>
    </section>
  );
}
