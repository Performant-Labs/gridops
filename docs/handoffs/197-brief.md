# [Fleet health: shared contract (design doc, migration, types, fake source, OpenAPI)](https://github.com/Performant-Labs/gridops/issues/197)

Wave 0 of epic #198. The issue body is the spec: read it in full (`gh issue view 197 --repo Performant-Labs/gridops`), and read the epic (#198) and every wave-1 issue (#190, #191, #192, #193, #194, #195, #199, #200, #201, #202, #203, #204) for the "Done when" lists you must turn into contract tests and the "Owns" lists that go in `docs/design/fleet-health-owners.json`.

## Why this story is shaped the way it is

Eleven-plus wave-1 issues will be done in parallel by weaker agents, each in its own worktree. They only succeed if this PR lands **every** shared type, migration, hook and wiring line, so that no two wave-1 PRs touch the same file. When in doubt, wire it here. A wave-1 issue must never need to edit `main.rs`, any `mod.rs`/`lib.rs`, the API router, `src/lib/navigation.ts` or `page.tsx`'s composition.

## Corrections to the issue text

- **Migration number:** the issue says `0049`, but `0049_fleet_inventory_snapshot_retention.sql` already exists on master. Use the next free number (`0050` today; re-check with `ls migrations | tail -1` before writing).
- If any wave-1 issue's "Owns" list conflicts with the table in #197 (two issues owning one file, or a file missing from the stub table), resolve it in the plan, say which way and why, and make the owners file reflect it. Do not edit the GitHub issues.

## Build and machine constraints (the Mac)

- Rust builds share one target dir via the untracked `~/Projects/gridops/.cargo/config.toml`. Never set `CARGO_TARGET_DIR`, never commit `.cargo/`. "Blocking waiting for file lock" is normal.
- `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2` are set in `.claude/settings.local.json`. Prefer `-p <crate>` runs while iterating; run the full suite once at the end.
- Disk is tight (about 29 GB free). Don't leave large scratch files around.

## The Mars database timing check

"The migration applies to a copy of the Mars database in under a minute" needs a real copy. Get it read-only: on Mars (`ssh deploy@5.189.148.42`), use `VACUUM INTO` into a file under `/tmp` on the host via a throwaway container (see the project memory: `docker run --rm -v gridops_gridops-data:/data nginx:alpine` + `apk add sqlite`; `VACUUM INTO` took about 93 s last time), `scp` it to the Mac scratch area, delete the remote copy, apply all migrations up to and including the new one with the project's normal migration path, and record the wall time. Delete the local copy afterwards. Never write to the live DB, never restart a Mars service. If Mars is unreachable, say so in the result rather than skipping silently.

## Guard check test PRs

The "Done when" asks for two throwaway PRs proving `Fleet health guard` fails (one editing a file outside an issue's list, one editing a contract test). Open them against `Performant-Labs/gridops` only (always pass `--repo Performant-Labs/gridops`, never upstream `iamngoni/gridops`), from branches off this story's branch, then close them and delete their branches.

## Out of scope

Filling in any stub beyond what #197 says is implemented here (`category.rs`, `merge_concerns`, `api.ts`, `chart-theme.ts`, `score-gauge.tsx`, the fake source, the guard, the speed-check helper and the synthetic dataset). Deploying. Changing prod data. Adding the required status check (done by the orchestrating session after merge).

## Docs

CHANGELOG entry. The design doc `docs/design/fleet-health.md` is the main doc deliverable.
