# Evidence: #241 (unchanged source the diff relies on)

## F (Phase 6, implementation)

- **Fact:** `RunStatusIcon` resolves `failure`, `waiting` and `skipped` through `RUN_STATES` and draws a different fixed glyph for each, with the `svg` `aria-hidden`; `skipped` reaches the `default` case. Item 1's severity map relies on this.
  **Source:** `src/components/status-icon.tsx:53-55`, `:59-60`, `:65-66`, `:69-70`
  **Verbatim excerpt:**
  > export function RunStatusIcon({ status, className, size = 14 }: { status: string | null | undefined; className?: string; size?: number }) {
  >   const state = runState(status) ?? "skipped";
  >   const common = { width: size, height: size, viewBox: "0 0 14 14", fill: "none", "aria-hidden": true as const, className: cn("shrink-0", className) };
  > ...
  >     case "waiting":
  >       return <svg {...common}><circle cx="7" cy="7" r="5.75" stroke="var(--warning)" strokeWidth="1.5" /><path d="M7 4.2V7l1.8 1.2" stroke="var(--warning)" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" /></svg>;
  > ...
  >     case "failure":
  >       return <svg {...common}><circle cx="7" cy="7" r="6.5" fill="var(--danger)" /><path d="m5 5 4 4M9 5 5 9" stroke="var(--panel)" strokeWidth="1.5" strokeLinecap="round" /></svg>;
  > ...
  >     default:
  >       return <svg {...common}><circle cx="7" cy="7" r="5.75" stroke="var(--neutral)" strokeWidth="1.5" /><path d="m4.9 9.1 4.2-4.2" stroke="var(--neutral)" strokeWidth="1.5" strokeLinecap="round" /></svg>;

- **Fact:** the icon-plus-hidden-label pattern item 1 copies is `StatusBadge`'s icon-only branch.
  **Source:** `src/components/status-icon.tsx:89`
  **Verbatim excerpt:**
  > if (iconOnly) return <span className={cn("inline-flex", className)} title={label}>{icon}<span className="sr-only">{label}</span></span>;

- **Fact:** the router's `location.hash` has no leading `#` and has already been through `decodePath`, which is why `hashHostId` now accepts `machine-…` as well as `#machine-…` (installed `@tanstack/router-core` is 1.171.15).
  **Source:** `node_modules/@tanstack/router-core/src/location.ts:27-30` and `node_modules/@tanstack/router-core/src/router.ts:1335`
  **Verbatim excerpt:**
  > /**
  >  * The hash of the location, excluding the leading hash character.
  >  * (e.g., '123' instead of '#123')
  >  */
  > ...
  >           hash: decodePath(hash.slice(1)).path,

- **Fact:** `useRouterState` with a `select` is the repo's existing way to read router location state, which `HealthSections` follows for the hash.
  **Source:** `src/components/app-shell.tsx:121`
  **Verbatim excerpt:**
  > const pathname = useRouterState({ select: (state) => state.location.pathname });

- **Fact:** every page slot must have a child element (#197's page test), which is why the `machine-detail` slot always renders `MachineDetail` (the prompt or the named machine) rather than nothing.
  **Source:** `tests/fleet-health-page.test.ts:67-69`
  **Verbatim excerpt:**
  > for (const slot of all("[data-health-section]")) {
  >   expect(slot.childElementCount, `slot ${slot.getAttribute("data-health-section")} is empty`).toBeGreaterThan(0);
  > }

- **Fact:** a score component's `deduction` may be null and `weight` is always a number, so items 3 and 4 treat `score === null` as the only no-data signal and fall back when `deduction` is null or empty.
  **Source:** `src/features/fleet-health/types.ts:44-50`
  **Verbatim excerpt:**
  > export const scoreComponentSchema = z.strictObject({
  >   key: z.string().min(1),
  >   label: z.string(),
  >   weight: z.number().nonnegative(),
  >   score,
  >   deduction: z.string().nullable(),
  > });

- **Fact:** the oldest queued job carries both `queuedAt` and `waitMs` (epoch and duration millis), which item 7's `oldestWaitMs` combines.
  **Source:** `src/features/fleet-health/types.ts:110-116`
  **Verbatim excerpt:**
  >   oldestQueued: z.strictObject({
  >     jobId: z.number().int(),
  >     jobName: z.string(),
  >     repository: z.string(),
  >     queuedAt: millis,
  >     waitMs: millis,
  >   }).nullable(),

- **Fact:** item 5's chart copies the live strip's stacked horizontal `BarChart` structure (vertical layout, hidden numeric `XAxis` with a `domain`, hidden category `YAxis`, one `Bar` per segment sharing a `stackId`).
  **Source:** `src/features/fleet-health/live-strip.tsx:33-43`
  **Verbatim excerpt:**
  > <ResponsiveContainer height="100%" width="100%">
  >   <BarChart data={rows} layout="vertical" margin={{ bottom: 0, left: 0, right: 8, top: 0 }}>
  >     <XAxis allowDecimals={false} domain={[0, scale]} hide type="number" />
  >     <YAxis dataKey="name" hide type="category" />
  >     {/* The legend rows above and below carry the values, so only the cursor is drawn. */}
  >     <ChartTooltip content={() => null} cursor={{ fill: "var(--hover)" }} isAnimationActive={false} />
  >     {JOB_CATEGORIES.map((category) => (
  >       <Bar barSize={16} dataKey={category} fill={categoryColor(category)} isAnimationActive={false} key={category} name={categoryLabel(category)} stackId="live" />
  >     ))}
  >   </BarChart>
  > </ResponsiveContainer>
