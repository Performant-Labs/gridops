#!/usr/bin/env python3
"""#269: the rest of the data the phone check records its API answers from.

The #204 walkthrough seed (`seed_for_walkthrough` in crates/gridops-api/src/fleet/
health_e2e_tests.rs) fills the fleet, a pool, a repository, workflow runs and runners. This adds,
to that database, what the other screens show: an administrator and a member with sessions, a few
audit events, webhook deliveries, agent runs and API tokens.

    python3 -I tests/a11y/seed-screens.py <database> <env file>

It generates a session secret and an encryption key and writes them, with the two session cookie
values, to <env file> (mode 0600) as shell assignments, for the API and for
`GRIDOPS_A11Y_ADMIN_SESSION` / `GRIDOPS_A11Y_MEMBER_SESSION`. None of them belongs in the
repository: they sign sessions for a scratch database only.
"""

import base64
import hashlib
import hmac
import json
import os
import secrets
import sqlite3
import sys
import time

MINUTE = 60_000
HOUR = 60 * MINUTE


def b64(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).decode().rstrip("=")


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    database, env_file = sys.argv[1], sys.argv[2]
    now = int(time.time() * 1000)
    session_secret = secrets.token_urlsafe(48)
    encryption_key = base64.b64encode(secrets.token_bytes(32)).decode()
    db = sqlite3.connect(database)
    db.execute("PRAGMA foreign_keys = ON")

    cookies = {}
    users = [("a11y-admin", 9001, "dana-admin", "Dana Ortiz", "admin"), ("a11y-member", 9002, "sam-member", "Sam Lee", "member")]
    for user_id, github_id, login, name, role in users:
        db.execute(
            "INSERT INTO users (id,github_id,login,name,role,access_token,last_login_at,created_at,updated_at) VALUES (?,?,?,?,?,'',?,?,?)",
            (user_id, github_id, login, name, role, now, now - 30 * 24 * HOUR, now),
        )
        token = f"a11y-{secrets.token_urlsafe(24)}"
        db.execute(
            "INSERT INTO sessions (id,token_hash,user_id,expires_at,last_seen_at,created_at) VALUES (?,?,?,?,?,?)",
            (f"a11y-session-{user_id}", b64(hashlib.sha256(token.encode()).digest()), user_id, now + 24 * HOUR, now, now),
        )
        signature = b64(hmac.new(session_secret.encode(), token.encode(), hashlib.sha256).digest())
        cookies[role] = f"{token}.{signature}"
        # The pool, repository and runs belong to the seeded installation. The administrator can
        # see it; the member, as in #256's recording, sees none of it.
        if role == "admin":
            db.execute(
                "INSERT INTO user_installations (user_id,installation_id,permission,created_at) SELECT ?,id,'admin',? FROM installations",
                (user_id, now),
            )

    audit = [
        ("runner_pool.updated", "runner_pool", "0f000004-0000-4000-8000-000000000001", {"name": "linux-docker", "fields": ["maxRunners", "labels"]}),
        ("fleet_host.approved", "fleet_host", "0f000001-0000-4000-8000-000000000002", {"name": "Jupiter"}),
        ("settings.updated", "settings", None, {"keys": ["jobCategories"]}),
        ("api_token.created", "api_token", "a11y-token-1", {"name": "deploy bot"}),
        ("member.role_changed", "user", "a11y-member", {"from": "admin", "to": "member"}),
        ("workflow_run.rerun_requested", "workflow_run", "960001", {"repository": "acme/app"}),
    ]
    for index, (action, target_type, target_id, metadata) in enumerate(audit):
        db.execute(
            "INSERT INTO audit_events (id,actor_user_id,actor_label,action,target_type,target_id,metadata,ip_address,created_at) VALUES (?,?,?,?,?,?,?,?,?)",
            (f"a11y-audit-{index}", "a11y-admin", "dana-admin", action, target_type, target_id, json.dumps(metadata), "203.0.113.7", now - (index + 1) * 47 * MINUTE),
        )

    deliveries = [
        ("workflow_job", "queued", "processed", None),
        ("workflow_job", "in_progress", "processed", None),
        ("workflow_job", "completed", "processed", None),
        ("workflow_run", "completed", "processed", None),
        ("push", None, "ignored", None),
        ("workflow_job", "queued", "failed", "no runner pool matches the job's labels: [self-hosted, linux, gpu-a100-80gb-extra-large]"),
    ]
    for index, (event, action, status, error) in enumerate(deliveries):
        received = now - (index + 1) * 13 * MINUTE
        db.execute(
            "INSERT INTO webhook_deliveries (id,event,action,hook_id,installation_id,repository_id,signature_valid,status,payload,error,received_at,processed_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
            (f"a11y-delivery-{index}-{secrets.token_hex(8)}", event, action, 4242, None, 4343, 1, status, json.dumps({"action": action, "repository": {"full_name": "acme/app"}, "workflow_job": {"name": "unit tests", "labels": ["self-hosted", "linux"]}}), error, received, received + 900),
        )

    jobs = [row[0] for row in db.execute("SELECT id FROM workflow_jobs WHERE status = 'completed' ORDER BY id DESC LIMIT 3")]
    runs = [
        ("succeeded", "verifying", "pull_request", "Pin the flaky snapshot test's clock", "The test read the wall clock; it now uses the fixed clock the other tests use.", "https://github.com/acme/app/pull/812", 812, "gridops/fix-snapshot-clock", None),
        ("failed", "reproducing", None, None, None, None, None, None, "The sandbox could not reach the package registry after 3 attempts (connection reset by peer)."),
        ("running", "editing", None, None, None, None, None, None, None),
    ]
    for index, (job_id, (status, stage, outcome, title, summary, url, number, branch, error)) in enumerate(zip(jobs, runs)):
        created = now - (index + 1) * 2 * HOUR
        done = status in ("succeeded", "failed")
        db.execute(
            "INSERT INTO agent_runs (id,job_id,run_id,repository_id,trigger,status,stage,outcome,title,summary,pull_request_url,pull_request_number,branch,error,provider,model,turns,tool_calls,requested_by,created_at,started_at,completed_at,updated_at)"
            " SELECT ?,j.id,j.run_id,r.repository_id,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,? FROM workflow_jobs j JOIN workflow_runs r ON r.id = j.run_id WHERE j.id = ?",
            (f"a11y-agent-{index}", "manual" if index == 0 else "automatic", status, stage, outcome, title, summary, url, number, branch, error, "anthropic", "claude-sonnet-5-5", 14 + index, 37 + index, "a11y-admin", created, created + MINUTE, created + 40 * MINUTE if done else None, created + 40 * MINUTE, job_id),
        )

    tokens = [("deploy bot", ["fleet:read", "fleet:write"], None), ("dashboard reader", ["fleet:read"], None), ("old laptop", ["fleet:read"], now - 3 * 24 * HOUR)]
    for index, (name, scopes, revoked) in enumerate(tokens):
        db.execute(
            "INSERT INTO api_tokens (id,user_id,name,verifier,display_prefix,scopes_json,created_at,expires_at,last_used_at,revoked_at) VALUES (?,?,?,?,?,?,?,?,?,?)",
            (f"a11y-token-{index + 1}", "a11y-admin", name, b64(secrets.token_bytes(32)), f"gop_{secrets.token_hex(3)}", json.dumps(scopes), now - (index + 2) * 24 * HOUR, now + 60 * 24 * HOUR, now - (index + 1) * HOUR, revoked),
        )

    db.commit()
    db.close()
    fd = os.open(env_file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as out:
        out.write(f"GRIDOPS_SESSION_SECRET='{session_secret}'\n")
        out.write(f"GRIDOPS_ENCRYPTION_KEY='{encryption_key}'\n")
        out.write(f"GRIDOPS_A11Y_ADMIN_SESSION='{cookies['admin']}'\n")
        out.write(f"GRIDOPS_A11Y_MEMBER_SESSION='{cookies['member']}'\n")
    print(f"seeded the screens' data into {database}; secrets in {env_file}")


if __name__ == "__main__":
    main()
