import { Link } from "@tanstack/react-router";
import { CheckCircle2 } from "lucide-react";
import { useState } from "react";

import { StatusDot, type HealthTone } from "~/components/status-icon";
import { Button } from "~/components/ui/button";
import { cn } from "~/lib/utils";
import type { Concern, FleetHealthViewProps, Severity } from "./types";

/** A concern's severity as a status tone: critical red, warning amber, info grey. */
const SEVERITY_TONES: Record<Severity, HealthTone> = { critical: "danger", warning: "warning", info: "neutral" };

/** How many concerns show before "Show all N". */
const VISIBLE_CONCERNS = 5;

/** The local clock time a concern has held ("since 3:12 PM"). */
const sinceFormat = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

const NO_KEYS: ReadonlySet<string> = new Set();

/**
 * The areas of concern (#195): the merged concern list, worst first then oldest, exactly as the
 * API orders it. One compact row per concern — its severity as a status dot, its title, its
 * detail sentence and how long it has held — the first five until "Show all N". Each row is keyed
 * by the concern's `key` (`data-concern-key`), so a refresh keeps it, and a concern that wasn't
 * in the previous data is marked `data-new="true"` and fades in; nothing is new on the first
 * render. "No areas of concern" with a green check when the list is empty.
 */
export function AreasOfConcern({ data }: FleetHealthViewProps) {
  const concerns = data.concerns;
  const [showAll, setShowAll] = useState(false);

  // The data the current "new" marks were computed from, adjusted while rendering when the data
  // changes (the pattern "adjusting state when a prop changes" replaces): a key that wasn't in
  // the previous data is new until the data changes again, and nothing is before the first data.
  // The page refreshes in place rather than remounting, so the marks — and the rows, keyed by
  // `key` — survive the re-renders between two changes.
  const [marks, setMarks] = useState<{ concerns: Concern[]; fresh: ReadonlySet<string> } | null>(null);
  if (marks === null || marks.concerns !== concerns) {
    const seen = marks === null ? null : new Set(marks.concerns.map((concern) => concern.key));
    setMarks({
      concerns,
      fresh: seen === null ? NO_KEYS : new Set(concerns.filter((concern) => !seen.has(concern.key)).map((concern) => concern.key)),
    });
  }

  return (
    <section aria-label="Areas of concern" className="rounded-lg border border-border bg-panel">
      <h2 className="border-b border-border px-4 py-2.5 text-sm font-medium text-foreground">Areas of concern</h2>
      {concerns.length === 0 ? (
        <p className="flex items-center gap-2 px-4 py-3 text-sm text-muted-foreground">
          <CheckCircle2 aria-hidden className="size-4 shrink-0 text-success" />
          No areas of concern
        </p>
      ) : (
        <>
          <ul className="divide-y divide-border">
            {(showAll ? concerns : concerns.slice(0, VISIBLE_CONCERNS)).map((concern) => (
              <ConcernRow concern={concern} isNew={marks?.fresh.has(concern.key) ?? false} key={concern.key} />
            ))}
          </ul>
          {!showAll && concerns.length > VISIBLE_CONCERNS ? (
            <div className="border-t border-border px-3 py-1.5">
              <Button onClick={() => setShowAll(true)} size="sm" variant="ghost">
                Show all {concerns.length}
              </Button>
            </div>
          ) : null}
        </>
      )}
    </section>
  );
}

/** One concern: a link to what explains it when the API has one, a plain row otherwise. */
function ConcernRow({ concern, isNew }: { concern: Concern; isNew: boolean }) {
  const body = (
    <>
      <StatusDot className="mt-1.5" tone={SEVERITY_TONES[concern.severity]} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="min-w-0 font-medium text-foreground">{concern.title}</span>
          {concern.since === null ? null : (
            <span className="ml-auto shrink-0 text-xs text-faint">
              since <time dateTime={new Date(concern.since).toISOString()}>{sinceFormat.format(concern.since)}</time>
            </span>
          )}
        </div>
        <p className="mt-0.5 text-xs leading-5 text-muted-foreground">{concern.detail}</p>
      </div>
    </>
  );
  const marked = { "data-concern-key": concern.key, ...(isNew ? { "data-new": "true" as const } : {}) };
  return (
    <li {...marked} className={cn("min-w-0", isNew && "animate-in fade-in-0 duration-700")}>
      {concern.link === null ? (
        <div className="flex items-start gap-2.5 px-4 py-2.5">{body}</div>
      ) : (
        <Link className="flex items-start gap-2.5 px-4 py-2.5 transition-colors hover:bg-hover focus-visible:bg-hover" to={concern.link}>
          {body}
        </Link>
      )}
    </li>
  );
}
