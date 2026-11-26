# Browser tests (`npm run test:a11y`)

Headless Chromium only. `npm run build` first: the tests serve `dist/` with `vite preview` and
answer every `/api/v1` request from `api-fixtures.json`. CI runs them in the web job.

- `fleet-health-axe.spec.ts` (#256): axe over the Overview, fleet health and the job-category
  settings, light and dark at 360 and 1280 px.
- `mobile.spec.ts` (#269, epic #268): every route in `src/routes` at 360 and 390 px, light and
  dark, in each of its states, checked for sideways scrolling, targets under 24 px (WCAG 2.2 2.5.8),
  text under 11 px and text cut off by an ellipsis. A screen that fails today is `fixme` with the
  epic issue that fixes it; that issue removes the `fixme`.
- `mobile-checks.spec.ts`: each of those checks catching its problem on a small test page.

## Options

| Variable | Effect |
|---|---|
| `GRIDOPS_MOBILE_SHOTS=<dir>` | `mobile.spec.ts` saves screenshots of every screen and state into `<dir>`, in segments (the app scrolls inside a panel). Keep `<dir>` outside the repository. |
| `GRIDOPS_MOBILE_ALL=1` | Runs the `fixme` screens too, to see what they still fail. |
| `GRIDOPS_A11Y_REPORT=<file>` | The axe spec appends one JSON line per scan. |
| `GRIDOPS_A11Y_BASE_URL=<url>` | Drives a running GridOps instead of the fixtures, signed in with `GRIDOPS_A11Y_ADMIN_SESSION` and `GRIDOPS_A11Y_MEMBER_SESSION` (the `gridops_session` cookie values). |
| `GRIDOPS_A11Y_RECORD=<file>` | With `GRIDOPS_A11Y_BASE_URL` and `--workers=1`: writes the API's answers to `<file>`, with the time they were recorded. The recorded mode fixes the page's clock at that time, so relative times ("12m ago") read as they did. |

## Recording the fixtures

The data is the #204 walkthrough's fleet plus `seed-screens.py` (users, audit events, webhook
deliveries, agent runs, API tokens). Every secret is generated into a scratch env file; none goes
in the repository. `SCRATCH` is any new directory outside it.

```sh
# 1. The walkthrough's fleet, seeded around the current hour.
GRIDOPS_FLEET_HEALTH_WALKTHROUGH_DB=$SCRATCH/gridops.sqlite CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 \
  cargo test -p gridops-api --bin gridops-api seed_for_walkthrough -- --ignored
# 2. The other screens' data, and the secrets and session cookies (into $SCRATCH/env, mode 0600).
python3 -I tests/a11y/seed-screens.py $SCRATCH/gridops.sqlite $SCRATCH/env
# 3. The SPA and the API serving it, on loopback.
npm run build && cargo build -p gridops-api
set -a; . $SCRATCH/env; set +a
GITHUB_CLIENT_ID=placeholder GITHUB_CLIENT_SECRET=placeholder GRIDOPS_FLEET_ALLOW_LOOPBACK_HTTP=true \
GRIDOPS_DATABASE_PATH=$SCRATCH/gridops.sqlite GRIDOPS_API_BIND=127.0.0.1:18269 \
GRIDOPS_BASE_URL=http://127.0.0.1:18269 GRIDOPS_WEB_ROOT=$PWD/dist <cargo target dir>/debug/gridops-api &
# 4. Every browser test, live, recording (the fixme screens too, so their answers are recorded).
GRIDOPS_MOBILE_ALL=1 GRIDOPS_A11Y_BASE_URL=http://127.0.0.1:18269 GRIDOPS_A11Y_RECORD=$SCRATCH/fixtures.json \
  npx playwright test --workers=1
cp $SCRATCH/fixtures.json tests/a11y/api-fixtures.json
```

Then run `npm run test:a11y` without the variables: it must pass on the new fixtures.
