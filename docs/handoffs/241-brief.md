# [#241](https://github.com/Performant-Labs/gridops/issues/241)

Part of #198. Found by the Opus review of the wave-1 UI PRs (#227, #231, #232), which the epic called for and which ran after they merged. Web only, so it can run alongside #204. References are to master at 73de941.

1. **Severity is shown by colour alone.** In `src/features/fleet-health/areas-of-concern.tsx` (~:85), `StatusDot` is `aria-hidden` and the row has no severity text, so neither colour-blind users nor screen readers can tell critical from warning.
   - Fix: reuse `RunStatusIcon` through a severity-to-`RunState` map (failure, waiting, skipped), plus visually hidden "Critical", "Warning" or "Info" text, as `StatusBadge` does. Add no new export or copied glyph to `status-icon.tsx`.
   - Test it.
2. **A same-page concern link doesn't expand the machine row.** `machine-score-table.tsx` (~:25-31) reads the hash only when it re-renders. A same-route TanStack navigation doesn't re-render it, so the page scrolls to a collapsed row.
   - Fix: subscribe to the hash (`useRouterState({ select: s => s.location.hash })`), or pass it down from `page.tsx`, and expand in an effect.
   - **Decided:** call `useRouterState({ select: s => s.location.hash })` in `HealthSections` (the pattern at `app-shell.tsx:121`) and pass the hash down as an optional prop. Don't put the hook inside `MachineScoreTable`: that breaks the Link-only `vi.mock` in the `*.contract.test.ts` file.
   - Expand through the render-time `seenHash` adjustment already in `machine-score-table.tsx`, not by setting state in an effect. `react-hooks/set-state-in-effect` is an error in this repo. Keep only the scroll in the effect.
   - The router's `location.hash` has no leading `#`. Extend `hashHostId(hash)` to cover that rather than writing a second parser.
   - Rewrite `tests/fleet-health-202-detail.test.ts` (~:145-152) to click the real concern Link under a real `createRouter`/`RouterProvider` (as `tests/page-header-nav.test.ts` does), not `router.navigate` and not a second `render()`. That test currently hides the bug. Extending #197's router mock is in scope.
3. **Weights render as raw floats.** `fleet-score-card.tsx` (~:126) shows values such as "47.05882352941177%". Round the weight, and show "—" for a component with no data.
4. **The "not enough data" tooltip ignores the API's reason.** `fleet-score-card.tsx` (~:13, :135, :140) always says the same thing. Show `component.deduction` when the score is null. `machine-score-table.tsx` (~:120) has the same fixed text: fix it the same way.
5. **The CPU and memory bars aren't stacked.** In `machine-detail.tsx` (~:93-114), "used" is drawn on top of "reserved", so reserved disappears when used is larger.
   - Fix: a Recharts horizontal stacked `BarChart` (reserved, used, budget), as the issue asked. Copy `live-strip.tsx`'s stacked `BarChart` structure, and export the segment calculation as a pure, tested function, as `usagePoints` and `slowestKeys` are. Any hatch goes in the chart's `defs` and uses theme variables only.
6. **The detail placeholder is wrong.** `machine-detail.tsx` (~:15-23) always says "Expand a machine above".
   - **Decided (option B):** name the expanded machine. Move the expansion state up into `HealthSections` (`page.tsx`), so the `machine-detail` slot always has a child: the named machine, or the "Expand a machine above" prompt when none is expanded.
   - `MachineScoreTable` must still work with only `{data, window}`, with the expansion props optional, so the #202 contract test stays untouched.
   - Update `tests/fleet-health-202-detail.test.ts` (~:128-133) to match.
   - Don't change `tests/fleet-health-page.test.ts`, or the design doc's rule that every section slot has content.
7. **The oldest wait can be understated.** `live-strip.tsx` (~:97) shows `oldest.waitMs`, fixed when the cached or stale response was built. Show `max(waitMs, now - queuedAt)` through a pure helper whose `now` defaults to `Date.now()`, as `formatElapsed` does (`react-hooks/purity`). Pin the clock in the test.

## Done when

- [ ] Each item has a test that fails before the fix.
- [ ] Web build, lint and tests pass.
- [ ] Recharts only, with no new dependency.
- [ ] New tests go in `tests/fleet-health-241-*.test.ts`. Never add tests to a `*.contract.test.ts` file.

## Constraints

- Keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`. Use the shared cargo target from the untracked `.cargo/config.toml`, and never set `CARGO_TARGET_DIR`. Prefer `-p` crate test runs.
- Work headless: no visible browsers and no screenshots on the desktop.
- Open the PR against master on Performant-Labs/gridops, never upstream iamngoni/gridops.
