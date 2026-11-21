## Brief Review (Round 1)

### BLOCK findings
None.

### Resolved findings
**[R-1]** Earlier B-1: "Done-when item 6 ('Rollback') is internally contradicted by the brief's own R3-NV-5 facts… `abandon_unstarted` 'returns `Ok(false)` and changes nothing once any `start_runner` command exists'… Fix: amend item 6 to condition the `abandon_unstarted` assertion on 'no `start_runner` command exists for the placement', and state the alternative acceptance for the case where a command row exists." — Resolved: consolidated Done-when item 6 now conditions the assertion on "no `start_runner` command row exists for the placement", requires the test to decide which case it is in from the `host_commands` table (never from `issue_command` returning `None`), and states the alternative acceptance (row failed, counted once, GitLab runner deleted or swept, no permanent capacity hold via the existing `follow`/`finish` path), matching the R4-NV-2/R4-B-1 placement facts quoted in the brief.

### Needs-verification findings
**[NV-1]** D17's GitLab endpoint claims (`GET /projects/:id/runners?type=project_type`, `type=group_type` on the group endpoint, and "the `type` filter keeps shared and instance runners out") — the brief's round-4 amendment describes the GitLab Runners API documentation page but quotes no verbatim doc excerpt; a quoted excerpt of docs.gitlab.com/api/runners (the `type` attribute paragraph for both endpoints) would settle it. #146 implements the endpoint; #139's `FakeGitLab`/sweep tests are written against this contract.

### WARN findings
**[W-1]** (carried forward, unresolved) The connection-selection rule remains non-normative: "it picks deterministically (e.g. the connection with fewest live GitLab runners, ties by `created_at, connection_id`)" — "e.g." leaves the actual rule to the implementer, and no done-when item pins it. Recommendation: delete "e.g." and add a done-when assertion that repeated starts across two connections distribute per the stated rule.

### NIT findings
**[NIT-1]** The brief's accumulated amendment layers (issue text, round-2/3/4 brief-gate amendments, consolidated done-when) make the authoritative text hard to locate; only the intro sentence and the "Done when (consolidated)" heading mark precedence. A short "read order" note at the top of the Clarifications would help implementers.

### Verdict
PASS — no BLOCK findings; implementation may proceed.