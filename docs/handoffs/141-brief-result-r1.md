## Brief Review (Round 1)

### BLOCK findings
None.

### Needs-verification findings
- **[NV-1]** The pool wire claims (#138, merged as `0086aa9`): that create still calls `assert_installation_admin` for non-repository scopes, that `validate_gitlab_targets` / `validate_gitlab_connections` exist with the described split in `models.rs`, that the GitLab kinds `exclude` all GitHub/Bitbucket kinds, and that `RunnerPoolOptions`/`RunnerPoolDetail` carry `gitlabConnections`/`gitlabConnectionIds`/`runUntagged`. — Evidence: the relevant excerpts from `crates/gridops-core/src/models.rs` and `crates/gridops-api/src/resources.rs` at the cited sites.
- **[NV-2]** The backend-edit claims for the runner list: the select is at `resources.rs` ~:1944, JSON at ~:1981-1995, it is the only runner JSON site ("grep bitbucketRunnerUuid"), and a test file `resources/gitlab_tests.rs` exists beside which the new test goes. — Evidence: those `resources.rs` lines and the grep result.
- **[NV-3]** The deliveries claim: #140 stores deliveries with ids `gitlab:<connection_id>:<key>` and the list API has no platform field. #140 is in progress, so this is unverifiable now and could change before #141 lands. — Evidence: #140's final merged schema/migration for `webhook_deliveries` and the list endpoint response in `docs/openapi/`.
- **[NV-4]** Library/framework defaults asserted: `z.object` strips unknown keys (`runner-pools.functions.ts:127-138` claim), `SearchableSelect` accepts `searchable={false}`, `SecretRevealDialog`'s `details` supports copying. — Evidence: zod docs for `.strip` default, the `SearchableSelect` and `SecretRevealDialog` source excerpts.
- **[NV-5]** The line-number citations into current files: `schemas.ts:13` (64-char label cap), `github.tsx:93-105, :225-257`, `webhooks.tsx:82-83, ~:119, :146`, `runner-pools.new.tsx:249-250`, `runner-pools.index.tsx:81`, `runner-pools.$poolId.index.tsx:129`, `src/` having no native `<select>`. — Evidence: those exact lines from the repo at the merge-base.
- **[NV-6]** The OpenAPI contract section (endpoints, status codes, `ApiErrorBody`, the stable error substrings from #138's mapping). — Evidence: `docs/openapi/gridops-v1.json` GitLab paths and #138's error-mapping code.

### WARN findings
- **[W-1]** The Scope section still says the operations view lists "polling errors", while the Contract explicitly says polling errors are out of reach (#148 logs only, no API). The precedence clause resolves it, but the stale scope line invites scope creep. — Delete or strike the "and polling errors" phrase in Scope.
- **[W-2]** "if none exists the GitLab kinds are still offered but submit fails … — record what the UI shows" is not a testable acceptance criterion; "record" is a documentation act, not a pass/fail condition. — Either specify the expected user-visible outcome (e.g. "the form surfaces the server's error message") or move it out of Done-when.
- **[W-3]** `maxCount` is used in the schema bounds rule ("≤ maxCount") but never defined in the brief — where it comes from (options payload? constant?) determines whether the refinement is implementable as specified. — Name the source of `maxCount` in the schema section.
- **[W-4]** The walkthrough says the settings save body "(PATCH/PUT body carries both ids)" without pinning the method for pool update; the brief is otherwise meticulous about PATCH-vs-PUT for connections. — State which method the pool update uses.

### NIT findings
- **[NIT-1]** The "Amended Done-when" allows `operations.functions.ts` changes and the Hard-limits wire-field list repeats this; the two lists of permitted GitLab-name locations are consistent but stated three times in different words — a single canonical list would reduce drift.
- **[NIT-2]** Gap 2 says existing descriptors "ignore" the new `platformOptions` values channel, but no Vitest item pins that GitHub/Bitbucket payloads are unchanged when the values object is populated — the "Payload partials" bullet covers GitLab cases only.

### Verdict
PASS — no BLOCK findings; implementation may proceed.