import { cn } from "~/lib/utils";

/**
 * The card a fleet health section shows until its wave-1 issue fills it in: its title and "Coming
 * in #N", so the page renders end to end against the fixture from the start (#197).
 */
export function PlaceholderCard({ title, issue, className }: { title: string; issue: number; className?: string }) {
  return (
    <section aria-label={title} className={cn("rounded-lg border border-dashed border-border bg-panel px-4 py-5", className)}>
      <h2 className="text-sm font-medium text-foreground">{title}</h2>
      <p className="mt-1 text-sm text-muted-foreground">Coming in #{issue}</p>
    </section>
  );
}
