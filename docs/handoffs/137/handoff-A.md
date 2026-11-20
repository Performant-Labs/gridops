# Handoff-A: Phase 3 - GitLab shared contract (#137)  (up-front plan review, round 3)

**Date:** 2026-11-20
**Branch:** claude/gitlab-137-contract-fa3bbe (head 4b1f581)
**Brief reviewed:** docs/handoffs/137-brief.md   **Reuse map:** N/A. No `survey.md` was provided, so the brief's own analogues are the baseline: the Bitbucket connection, client, runner columns and `/v1/runners` fields, and the GitHub fleet path.   **Wireframe:** N/A (no UI surface)
**Verdict:** PASS

## Summary

PASS. The "Amendments (architecture review, round 2)" section resolves the round-2 block (R2-1) with option (a), and resolves all four round-2 warnings. What it adds fits the code:

- `for_selected_provider` takes `CiPlatform`. Core modules outside `fleet` already import from `crate::fleet`, for example `api_tokens.rs` and `build_info.rs`, so this follows the existing dependency direction.
- The `/v1/runners` GitLab fields follow the Bitbucket precedent: an optional non-secret object, a top-level secret delivered on stdin, and forwarding through `serde_json::to_value(&input)`.
- The exit codes follow the Tart agent's sysexits-style `exit 78`.
- The ownership is recorded as deviations D9 to D12.

Four new warnings, none blocking:

1. A second GitLab start path in `gridops-api` is assigned to #139 without saying so.
2. The manager's relay of the Tart listing would drop `exitReason`.
3. The pinned `GitlabBootstrap` attributes don't match either crate's `BitbucketBootstrap`, and won't compile in the Tart agent as written.
4. Some text still names #142 as a consumer of the fleet path.

### Resolved findings

- **[R2-1] (block): the runner-manager path wasn't pinned. Resolved.** I checked each part against the code:
  - **`for_selected_provider`.** It takes `platform: CiPlatform`, and the GitHub and Bitbucket results are unchanged. NV-6 matches `provisioning.rs:16-60`. `CiPlatform` is `Copy` (`domain.rs:228`).
  - **The two callers.** Both are mechanical changes: `gridops-api/src/resources.rs:4259` and `gridops-reconciler/src/main.rs:2404`.
  - **The `ProvisionRunner` fields.** `gitlab: Option<GitlabBootstrap>` and `gitlabRunnerToken` are added to both private structs:
    - the manager's, at `gridops-manager/src/main.rs:323-353`;
    - the Tart agent's, at `gridops-tart-agent/src/main.rs:250-270`.
  - **Forwarding.** The manager forwards the request with `serde_json::to_value(&*input)` (`main.rs:1690`). Neither struct denies unknown fields, so older agents ignore the new keys.
  - **Secret delivery.** It is stdin, matching `take_bootstrap_secret`, the `IFS= read -r GRIDOPS_BOOTSTRAP_SECRET` script, and the Tart agent's `tart exec -i` stdin write (`tart main.rs:1717-1740`).
  - **The `("gitlab", _)` arms.** They sit before the `"Runner platform is invalid."` arm in both crates (manager `:471`, Tart `:368`).
  - **Exit status.** The Tart agent can see the exit status of the in-VM script through `child.wait()` (`:1744`). Today it is only logged.
  - **Exit codes.** 75 and 77 don't collide with the image-missing `exit 78` (`:1769`, `:1792`).
  - **The reconciler's listing type.** Its `ManagedRunner` (`reconciler main.rs:321-327`) doesn't deny unknown fields, so a later `exitReason` is safe.
  - **Text fixes.** The `GitlabRunnerMaterial` doc comment and the first W-4 paragraph are corrected.
- **[R2-W1] admission. Resolved.** #139 owns the GitLab arms of `ensure_target_identity_for_pool` and `link_pool_target`. User-scoped visibility is deferred (D11). A-1 now says "stored and loaded".
- **[R2-W2] outbound FKs of `fleet_ci_targets`. Resolved.** The three FKs are listed, and the upgrade test asserts that `pragma_foreign_key_list('fleet_ci_targets')` returns exactly four rows.
- **[R2-W3] the shared `gitlab` field. Resolved.** #138 owns the `AppState` field and #139 owns the `Reconciler` field. An early lander adds an identical field and the later one rebases. That is an acceptable add-only merge.
- **[R2-W4] path parameter. Resolved.** It is `{connection_id}` everywhere, and `grep connectionId}` finds nothing.

## Findings

| # | Severity | Plan element | Drift dimension | Finding | Suggested fix |
|---|---|---|---|---|---|
| 1 | warn | R2-1 item 4, #139's row: "Choosing `CiPlatform::Gitlab` in both `for_selected_provider` callers, filling `gitlab` and `gitlabRunnerToken`" | cross-cutting (disjoint file ownership; one registration lifecycle) | **One of the two callers is the API's in-request start.** It is `gridops-api/src/resources.rs::provision` (`:4169`). That path starts Docker-only pools inside the request, because `starts_belong_to_reconciler` (`:4165`, gating `:2637`, `:2791` and `:4611`) hands only Tart pools to the reconciler (#119).<br>**Filling `gitlabRunnerToken` there is a whole lifecycle.** It needs `state.gitlab.create_runner`, the `runners` insert with the GitLab ids, and rollback on a failed manager call. So #139 would build a second GitLab registration lifecycle in the API crate, outside its `gitlab_lifecycle.rs`.<br>**It also crosses #138's ownership.** The epic gives #138 the GitLab sections of `resources.rs`. R2-W3 makes #138 the owner of `state.gitlab` and doesn't list #139 as a reader. No deviation row records any of this.<br>**Not drift from the code.** The code already duplicates Bitbucket registration in both places (API `:4303-4330`, reconciler `:2598-2620`). The problem is with the epic's rule that no two wave-1 issues own the same file. | Decide in writing between two options and record the choice as D14, in the brief and on #136.<br>**(a), recommended: GitLab starts belong to the reconciler, following the #119 Tart precedent.**<br>- `starts_belong_to_reconciler`, or its three call sites, also returns true for a pool with `runner_pool_gitlab_connections` rows.<br>- The API caller then only ever passes `Github` or `Bitbucket`.<br>- The GitLab lifecycle lives only in #139's module.<br>- Name the owner of that predicate: #138, because the line is in `resources.rs`.<br>**(b) Keep in-request GitLab starts.**<br>- Record that #139 edits `resources.rs::provision`.<br>- Add #139 to R2-W3's readers of `state.gitlab`.<br>- Require the API branch to call the same registration and rollback helper as the reconciler, so rollback isn't written twice. |
| 2 | warn | R2-1 item 3: "The manager reports it for Docker runners and the Tart agent for Tart runners ... #139 reads it" | pattern consistency (the relay hop in the listing contract) | **The reconciler reads only the manager's listing** (`MANAGER_RUNNER_LIST`, `reconciler main.rs:165-180`).<br>**The manager re-parses the Tart agent's listing.** It uses its own `ManagedRunner` struct (`manager main.rs:570-581`, `tart_runners` at `:1563-1570`), which has no `exit_reason` field, so a Tart `exitReason` is silently dropped. That holds unless #147 happens to add the field to this same struct. Even then, #142's `exitReason` is lost whenever #142 merges first.<br>**`ManagedRunner` serializes every field.** "Present only for an exited GitLab runner" therefore also needs `skip_serializing_if`.<br>**The brief doesn't mention this hop.** | Pin the relay: the manager's `ManagedRunner` gains `#[serde(default, skip_serializing_if = "Option::is_none")] exit_reason: Option<String>` (wire `exitReason`).<br>Prefer #137 adding it now, as shapes only. That matches the `ProvisionRunner` fields #137 already adds on the request side, and makes the relay independent of the order #142 and #147 merge in.<br>Add a manager test: a Tart listing entry carrying `exitReason` survives `tart_runners`, and an entry without it serializes with no `exitReason` key.<br>Otherwise, name #147 as the owner of that field and of that test. |
| 3 | warn | R2-1 item 2: one `GitlabBootstrap` definition pinned for both crates, `#[derive(Deserialize, Serialize)] #[serde(rename_all = "camelCase", deny_unknown_fields)]` | pattern consistency (local struct conventions) | **It won't compile in the Tart agent as written.** The Tart agent's `ProvisionRunner` derives `Debug` (`tart main.rs:250`), so a field type without `Debug` doesn't compile there. That crate's `BitbucketBootstrap` is `#[derive(Debug, Deserialize)]` (`:272-279`).<br>**`deny_unknown_fields` isn't the local pattern.** Neither crate uses it on `BitbucketBootstrap` or `ProvisionRunner`; the brief's own NV-8 notes this. These private relay structs are lenient on purpose: the manager re-serializes its own struct to a Tart agent that is deployed separately on the Mac. The strict serde convention belongs to the `gridops-core` fleet protocol types, not to these. | Mirror each crate's `BitbucketBootstrap`, both with `#[serde(rename_all = "camelCase")]` and no `deny_unknown_fields`:<br>- the manager's: `#[derive(Deserialize, Serialize)]`;<br>- the Tart agent's: `#[derive(Debug, Deserialize)]`.<br>Alternatively, justify the strictness in writing. |
| 4 | warn | Text still describing #142 as a fleet host-agent consumer: line 203, line 435, line 582, and deviation rows D1, D5 and D6 | naming / contract text (the deviation table is repeated in the PR and on #136) | **Line 203** says "the host agents of #147 and #142 advertise in `SupportedRunner`". The Tart agent has no `gridops-core` dependency and advertises no `SupportedRunner`.<br>**Line 435** says "#147, #142 and #139 must branch on `diagnostic.code`". #142 reports `exitReason` instead.<br>**Line 582** makes the GitLab `PrepareEnvironment` platform rules "#147's and #142's".<br>**D1** shows the material with three fields (no `wait_timeout_seconds`) and lists #142.<br>**D5 and D6** list #142.<br>The R2 precedence rule means #137's own F and T aren't misled. But #136 and the PR body are where the #142 and #147 implementers read the deviations. | Correct these in place:<br>- **Line 203:** "#147's host agent advertises".<br>- **Line 435:** "#147 (fleet path) and #139".<br>- **Line 582:** the fleet-path rules are #147's; the runner-manager rules live in each crate's `validate` (#147 manager, #142 Tart agent).<br>- **D1:** add `wait_timeout_seconds` and drop #142.<br>- **D5 and D6:** drop #142, and point to D10 for the Tart path. |

## Notes for O

None required: the verdict is PASS. All four warnings are cheap text amendments to make before T starts:

- **Finding 3** changes code that #137 itself writes. Fix it now so F follows each crate's convention.
- **Finding 2** is best settled now as well, because the recommended fix adds a shape in #137.
- **Findings 1 and 4** only change the deviation table and the ownership rows that the PR and the #136 comment repeat.

## Patterns referenced

- `crates/gridops-core/src/provisioning.rs:1-60` (`ProvisioningPlan`), `crates/gridops-core/src/{api_tokens.rs:18,build_info.rs:43}` (core modules outside `fleet` importing from `crate::fleet`), and `crates/gridops-core/src/fleet/domain.rs:228-233` (`CiPlatform`)
- `crates/gridops-manager/src/main.rs:323-480` (`ProvisionRunner`, `BitbucketBootstrap`, `validate`), `:570-581` (`ManagedRunner`), `:1467-1570` (`list_runners`, `tart_runners`), `:1683-1697` (forwarding to Tart) and `:2640-2692` (stdin secret delivery)
- `crates/gridops-tart-agent/src/main.rs:250-370` (`ProvisionRunner`, `BitbucketBootstrap`, `validate`), `:660-718` (`list_runners`) and `:1711-1792` (`tart exec -i`, child exit status, `exit 78`)
- `crates/gridops-reconciler/src/main.rs:165-180,321-327` (manager listing types) and `:2585-2690` (the `json!` request builder); `crates/gridops-api/src/resources.rs:4160-4380` (`starts_belong_to_reconciler`, the in-request `provision` and its Bitbucket registration)
- Epic #136's ownership table, and issues #139, #142 and #147 (`gh issue view`)
