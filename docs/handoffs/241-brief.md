# [#241](https://github.com/Performant-Labs/gridops/issues/241)

Part of #198. Found by the Opus review of the wave-1 UI PRs (#227, #231, #232), which the epic called for and which ran after they merged. Web only, so it can run alongside #204. References are to master at 73de941.

1. **Severity is shown by colour alone.** In `web/src/features/fleet-health/areas-of-concern.tsx` (~:85), `StatusDot` is `aria-hidden` and the row has no severity text, so neither colour-blind users nor screen readers can tell critical from warning.
   - Fix: a distinct icon per severity from `status-icon.tsx`, plus visually hidden "Critical", "Warning" or "Info" text.
   - Test it.
2. **A same-page concern link doesn't expand the machine row.** `machine-score-table.tsx` (~:25-31) reads the hash only when it re-renders. A same-route TanStack navigation doesn't re-render it, so the page scrolls to a collapsed row.
   - Fix: subscribe to the hash (`useRouterState({ select: s => s.location.hash })`), or pass it down from `page.tsx`, and expand in an effect.
   - Rewrite `tests/fleet-health-202-detail.test.ts` (~:145-152) to navigate through a real `RouterProvider` instead of calling `render()` again by hand. That test currently hides the bug. Extending #197's router mock is in scope.
3. **Weights render as raw floats.** `fleet-score-card.tsx` (~:126) shows values such as "47.05882352941177%". Round the weight, and show "—" for a component with no data.
4. **The "not enough data" tooltip ignores the API's reason.** `fleet-score-card.tsx` (~:13, :135, :140) always says the same thing. Show `component.deduction` when the score is null.
5. **The CPU and memory bars aren't stacked.** In `machine-detail.tsx` (~:93-114), "used" is drawn on top of "reserved", so reserved disappears when used is larger.
   - Fix: a Recharts horizontal stacked `BarChart` (reserved, used, budget), as the issue asked.
6. **The detail placeholder is wrong.** `machine-detail.tsx` (~:15-23) always says "Expand a machine above". Hide it, or name the expanded machine.
7. **The oldest wait can be understated.** `live-strip.tsx` (~:97) shows `oldest.waitMs`, fixed when the cached or stale response was built. Show `max(waitMs, Date.now() - queuedAt)`.

## Done when

- [ ] Each item has a test that fails before the fix.
- [ ] Web build, lint and tests pass.
- [ ] Recharts only, with no new dependency.

## Constraints

- Keep `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`. Use the shared cargo target from the untracked `.cargo/config.toml`, and never set `CARGO_TARGET_DIR`. Prefer `-p` crate test runs.
- Work headless: no visible browsers and no screenshots on the desktop.
- Open the PR against master on Performant-Labs/gridops, never upstream iamngoni/gridops.
