import { APP_VERSION_LABEL } from "~/lib/app-version";

/**
 * A thin strip at the top of every page (rendered by the root route, so the login and 404 pages have it too) naming the
 * build. Its height is `--version-bar-height` (app.css): the root pads the page by it and the fixed app shell starts below
 * it, so it never covers content. Modal overlays sit above it (z-50 and up).
 */
export function VersionBar() {
  return (
    <div
      className="fixed inset-x-0 top-0 z-40 flex h-[var(--version-bar-height)] items-center justify-center gap-2 border-b border-border bg-panel-subtle px-3 text-[11px] leading-none text-muted-foreground"
      data-testid="version-bar"
    >
      <span>GridOps</span>
      <span className="font-mono tabular-nums">{APP_VERSION_LABEL}</span>
    </div>
  );
}
