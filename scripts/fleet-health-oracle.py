#!/usr/bin/env python3
"""The fleet health oracle (#204 item 4): an independent implementation of
docs/design/fleet-health.md over a GridOps SQLite database.

It reads the raw tables directly (jobs, runs, runners, placements, samples,
start commands and failure events, budgets, category rules, snapshots) and
computes what GET /api/v1/fleet/health should answer for one window at one
time: every score and component, the speed indexes, the usage by category
(fleet and per machine) and the hourly usage, the trend, the live counts and
every concern. It never reads the hourly rollup tables and uses no GridOps
code: two implementations of the design that agree are the check that the
formulas were built as designed. Standard library only.

It also classifies every job by the stored category rules, in order, and
reports where a stored category differs from what the rules give.

    python3 scripts/fleet-health-oracle.py DB --now MS --window 24h

prints one JSON object. The end-to-end test
(crates/gridops-api/src/fleet/health_e2e_tests.rs) runs it and compares.

Simplifications, each one the seeded test data stays inside:
- The live queue split counts a waiting GitHub job as served when some pool
  that covers its repository offers all its labels (the pool's labels and its
  name); platform labels, pool capacity and pausing are not modelled for it.
  Waiting GitLab jobs follow #253's rule (see `gitlab_owner`), except which of
  several autoscaling pools with room a job goes to.
"""

import argparse
import json
import math
import re
import sqlite3
import sys

HOUR = 3_600_000
MINUTE = 60_000
GRACE = 15 * MINUTE
GIB = 1024 ** 3
WINDOWS = {"1h": HOUR, "24h": 24 * HOUR, "7d": 7 * 24 * HOUR}
UNMANAGED = "unmanaged"
CATEGORIES = ["test", "build", "check", "deploy", "fix-agent", "other"]
SEVERITY_ORDER = {"critical": 0, "warning": 1, "info": 2}
# The histogram buckets' lower bounds: 0-10 s, 10-30 s, 30-60 s, 1-2, 2-5,
# 5-10, 10-20, 20-40 and 40+ min.
BOUNDS = [0, 10_000, 30_000, 60_000, 120_000, 300_000, 600_000, 1_200_000, 2_400_000]
# Start failures that are the machine's fault (the design doc's
# `is_start_failure`): every agent-reported code but these, and these reasons.
MACHINE_FAULT_REASONS = {
    "agent_outcome_unknown",
    "prepare_deadline_expired",
    "start_deadline_expired",
    "agent_session_lost",
    "host_epoch_changed",
}


def rnd(value):
    """Rounds half away from zero, as Rust's f64::round does."""
    return int(math.floor(value + 0.5)) if value >= 0 else -int(math.floor(-value + 0.5))


def duration_text(ms):
    """A duration as the sentences read it: 45 s, 1 min, 7 min 12 s, 1 h 5 min."""
    total = rnd(max(ms, 0.0) / 1000.0)
    minutes, seconds = divmod(total, 60)
    hours, minutes = divmod(minutes, 60)
    if hours == 0 and minutes == 0:
        return f"{seconds} s"
    if hours == 0:
        return f"{minutes} min" if seconds == 0 else f"{minutes} min {seconds} s"
    return f"{hours} h" if minutes == 0 else f"{hours} h {minutes} min"


def percent(share):
    """A share to one decimal: 97.4%."""
    return f"{share * 100.0:.1f}%"


def whole_percent(share):
    """A share without a trailing .0: 45%, 29.9%."""
    value = share * 100.0
    return f"{rnd(value)}%" if abs(value - round(value)) < 1e-9 else f"{value:.1f}%"


def gib_text(size):
    """A byte count in GiB: 15 GiB, 10.3 GiB."""
    value = size / GIB
    return f"{value:.0f} GiB" if abs(value - round(value)) < 0.05 else f"{value:.1f} GiB"


def floor_hour(ms):
    return ms - ms % HOUR


def ceil_hour(ms):
    return ms if ms % HOUR == 0 else ms + HOUR - ms % HOUR


def bucket(ms):
    index = 0
    for position, lower in enumerate(BOUNDS):
        if ms >= lower:
            index = position
    return index


def percentile(histogram, quantile):
    """The quantile of a bucket histogram, interpolated inside its bucket; the
    40+ min bucket counts as 40 min."""
    total = sum(histogram)
    if total == 0:
        return None
    target = min(max(quantile, 0.0), 1.0) * total
    before = 0
    for index, count in enumerate(histogram):
        if count > 0 and before + count >= target:
            lower = BOUNDS[index]
            upper = BOUNDS[index + 1] if index + 1 < len(BOUNDS) else lower
            position = min(max((target - before) / count, 0.0), 1.0)
            return lower + position * (upper - lower)
        before += count
    return None


def nearest_rank(values, percent):
    values = sorted(values)
    if not values:
        return None
    rank = min(max(-(-percent * len(values) // 100), 1), len(values))
    return values[rank - 1]


def linear_lower(value, good, bad):
    """100 at `good`, 0 at `bad`, less being better, clamped."""
    return min(max((bad - value) / (bad - good) * 100.0, 0.0), 100.0)


def linear_higher(value, good, bad):
    """100 at `good`, 0 at `bad`, more being better, clamped."""
    return min(max((value - bad) / (good - bad) * 100.0, 0.0), 100.0)


def disk_alarm(total, free):
    if total == 0:
        return None
    warn = max(10 * GIB, total // 10)
    critical = max(5 * GIB, total // 20)
    if free < critical:
        return "critical"
    if free < warn:
        return "warning"
    return None


def is_start_failure(detail_json):
    try:
        detail = json.loads(detail_json)
    except ValueError:
        detail = {}
    reason = detail.get("reason") or ""
    if reason == "agent_reported":
        # The codes an agent reports that are not the machine's fault.
        return detail.get("agent_code") not in NOT_THE_MACHINES_FAULT
    return reason in MACHINE_FAULT_REASONS


# Agent-reported codes that are not the machine's fault (start_health.rs's
# list, which the design doc names but does not spell out); the seeded data
# reports none of them.
NOT_THE_MACHINES_FAULT = set()


class Oracle:
    def __init__(self, path, now, window):
        self.db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
        self.db.row_factory = sqlite3.Row
        self.now = now
        self.window = window
        self.length = WINDOWS[window]
        self.first = ceil_hour(now - self.length)

    def q(self, sql, *args):
        return self.db.execute(sql, args).fetchall()

    # -- Machines --------------------------------------------------------

    def approved_hosts(self):
        """Approved, non-deleted machines in id order, with the root budget's
        CPU and memory, the observed root disk's size, and their life: from
        `created_at` until `retired_at` while retired (#262; a retired row
        without a `retired_at` retired before any window, -inf)."""
        rows = self.q(
            """SELECT fh.id,fh.name,fh.last_heartbeat_at,fh.lifecycle_state,fh.created_at,fh.retired_at
                 FROM fleet_hosts fh
                WHERE fh.enrollment_state='approved' AND fh.deleted_at IS NULL ORDER BY fh.id"""
        )
        hosts = []
        for row in rows:
            retired = None
            if row["lifecycle_state"] == "retired":
                retired = row["retired_at"] if row["retired_at"] is not None else -math.inf
            budget = self.q(
                "SELECT cpu_millis,memory_mib FROM host_resource_domains WHERE host_id=? AND parent_domain_id IS NULL",
                row["id"],
            )
            disk = self.q(
                "SELECT disk_bytes FROM fleet_observed_domains WHERE host_id=? AND local_key='root'",
                row["id"],
            )
            hosts.append(
                {
                    "id": row["id"],
                    "name": row["name"],
                    "cpu": budget[0]["cpu_millis"] if budget else 0,
                    "memory": budget[0]["memory_mib"] if budget else 0,
                    "disk": disk[0]["disk_bytes"] if disk else None,
                    "heartbeat": row["last_heartbeat_at"],
                    "lifecycle": row["lifecycle_state"],
                    "created": row["created_at"],
                    "retired": retired,
                }
            )
        return hosts

    def hosts(self):
        """The window's machines (#262): the approved, non-deleted ones whose
        life overlaps the covered time [first, now). One retired before the
        window isn't on the page; one created or retired during it is."""
        return [host for host in self.approved_hosts() if self.lives(host, self.first, self.now)]

    def live_hosts(self):
        """The fleet's machines now (#262): approved, not deleted, not retired."""
        return [host for host in self.approved_hosts() if host["lifecycle"] != "retired"]

    @staticmethod
    def span(host, start, end):
        """The part of [start, end) `host` lived in: from its creation until
        its retirement."""
        retired = host["retired"] if host["retired"] is not None else math.inf
        return max(start, host["created"]), min(end, retired)

    @staticmethod
    def lives(host, start, end):
        begin, until = Oracle.span(host, start, end)
        return begin < until

    def lives_in_hour(self, host, hour):
        return self.lives(host, hour, hour + HOUR)

    # -- Jobs ------------------------------------------------------------

    def placement_of_runner(self, runner_row_id):
        if runner_row_id is None:
            return None
        rows = self.q(
            """SELECT p.host_id,p.cpu_millis,p.state,i.pool_id FROM workload_placements p
                 LEFT JOIN workload_intents i ON i.id=p.intent_id
                WHERE p.runner_id=? ORDER BY p.generation DESC,p.updated_at DESC,p.id DESC LIMIT 1""",
            runner_row_id,
        )
        return rows[0] if rows else None

    def runner_row(self, column, external_id):
        if external_id is None:
            return None
        rows = self.q(
            f"SELECT id FROM runners WHERE {column}=? ORDER BY created_at DESC,id DESC LIMIT 1",
            external_id,
        )
        return rows[0]["id"] if rows else None

    def github_jobs(self):
        """Every completed GitHub job with what the rules need."""
        rows = self.q(
            """SELECT wj.id,wj.run_id,wj.name,wj.conclusion,wj.runner_id,wj.labels,wj.category,
                      wj.created_at,wj.started_at,wj.completed_at,wj.status,
                      sync.run_attempt AS attempt,run.run_attempt AS run_attempt,
                      run.workflow_name,repo.full_name AS repository
                 FROM workflow_jobs wj
                 LEFT JOIN workflow_job_sync sync ON sync.job_id=wj.id
                 LEFT JOIN workflow_runs run ON run.id=wj.run_id
                 LEFT JOIN repositories repo ON repo.id=run.repository_id
                WHERE wj.completed_at IS NOT NULL"""
        )
        return [dict(row) for row in rows]

    def final_attempt(self, job, jobs_by_run):
        """A job counts unless a later attempt of its run ran a job of the same
        name: a higher attempt, or, when either attempt is unknown and the run
        knows a re-run, a higher job id."""
        for other in jobs_by_run.get(job["run_id"], []):
            if other["id"] == job["id"] or other["name"] != job["name"]:
                continue
            if other["attempt"] is not None and job["attempt"] is not None:
                if other["attempt"] > job["attempt"]:
                    return False
            elif other["id"] > job["id"] and (job["run_attempt"] or 0) > 1:
                return False
        return True

    def all_run_jobs(self):
        rows = self.q(
            """SELECT wj.id,wj.run_id,wj.name,sync.run_attempt AS attempt
                 FROM workflow_jobs wj LEFT JOIN workflow_job_sync sync ON sync.job_id=wj.id"""
        )
        by_run = {}
        for row in rows:
            by_run.setdefault(row["run_id"], []).append(dict(row))
        return by_run

    def gitlab_jobs(self):
        rows = self.q(
            """SELECT id,connection_id,project_id,job_name,tag_list,project_path,category,status,
                      gitlab_runner_id,created_at,started_at,completed_at
                 FROM gitlab_jobs WHERE completed_at IS NOT NULL"""
        )
        return [dict(row) for row in rows]

    def counted_jobs(self, start, end):
        """The final-attempt jobs completed in [start, end), each with its
        machine, pool, CPU size, category and outcome."""
        by_run = self.all_run_jobs()
        jobs = []
        for job in self.github_jobs():
            if not (start <= job["completed_at"] < end):
                continue
            if not self.final_attempt(job, by_run):
                continue
            placement = self.placement_of_runner(self.runner_row("github_runner_id", job["runner_id"]))
            jobs.append(self.fact(job, placement, job["conclusion"] == "success"))
        for job in self.gitlab_jobs():
            if not (start <= job["completed_at"] < end):
                continue
            placement = self.placement_of_runner(self.runner_row("gitlab_runner_id", job["gitlab_runner_id"]))
            jobs.append(self.fact(job, placement, job["status"] == "success"))
        return jobs

    @staticmethod
    def fact(job, placement, success):
        managed = placement is not None and placement["host_id"] is not None and placement["pool_id"] is not None
        started = job["started_at"] if job["started_at"] is not None else job["completed_at"]
        duration = max(job["completed_at"] - started, 0)
        cpu = placement["cpu_millis"] if managed else 0
        return {
            "hour": floor_hour(job["completed_at"]),
            "host": placement["host_id"] if managed else UNMANAGED,
            "category": job["category"] if job["category"] in CATEGORIES else "other",
            "success": success,
            "infra": placement is not None and placement["state"] in ("failed", "uncertain"),
            "duration": duration,
            "wait": started - job["created_at"],
            "cpu_seconds": cpu * duration / 1_000_000.0,
        }

    # -- Machine hours ---------------------------------------------------

    def host_hour(self, host_id, start, end):
        samples = self.q(
            """SELECT observed_at,cpu_used_millis,memory_used_mib,disk_free_bytes FROM host_samples
                WHERE host_id=? AND observed_at>=? AND observed_at<?""",
            host_id,
            start,
            end,
        )
        minutes = {(row["observed_at"] - start) // MINUTE for row in samples}
        cpu = [row["cpu_used_millis"] for row in samples if row["cpu_used_millis"] is not None]
        memory = [row["memory_used_mib"] for row in samples if row["memory_used_mib"] is not None]
        disk = [row["disk_free_bytes"] for row in samples if row["disk_free_bytes"] is not None]
        attempts = self.q(
            """SELECT COUNT(DISTINCT placement_id) AS n FROM host_commands
                WHERE host_id=? AND kind='start_runner' AND issued_at>=? AND issued_at<?""",
            host_id,
            start,
            end,
        )[0]["n"]
        failures = sum(
            1
            for row in self.q(
                """SELECT detail_json FROM fleet_events
                    WHERE host_id=? AND kind='placement_failed' AND created_at>=? AND created_at<?""",
                host_id,
                start,
                end,
            )
            if is_start_failure(row["detail_json"])
        )
        return {
            "hour": start,
            "host": host_id,
            "online": len(minutes),
            "cpu_p50": nearest_rank(cpu, 50),
            "memory_p95": nearest_rank(memory, 95),
            "min_disk": min(disk) if disk else None,
            "attempts": attempts,
            "failures": failures,
        }

    def host_hours(self):
        """Every machine's row for every hour of the window it lived in, every
        machine that ever enrolled included (a revoked one's starts still
        happened): an hour with no samples still has one (0 minutes). #262:
        none for an hour before its creation or after its retirement."""
        machines = []
        for row in self.q("SELECT id,created_at,lifecycle_state,retired_at FROM fleet_hosts ORDER BY id"):
            retired = None
            if row["lifecycle_state"] == "retired":
                retired = row["retired_at"] if row["retired_at"] is not None else -math.inf
            machines.append({"id": row["id"], "created": row["created_at"], "retired": retired})
        rows = []
        hour = self.first
        while hour < self.now:
            end = min(hour + HOUR, self.now)
            for machine in machines:
                if self.lives(machine, hour, end):
                    rows.append(self.host_hour(machine["id"], hour, end))
            hour += HOUR
        return rows

    def speed_machine(self, host_id, at):
        """#262: a job counts for the speed index only on a machine that is
        approved, not deleted, and lived when the job completed."""
        if not hasattr(self, "_speed_machines"):
            self._speed_machines = {host["id"]: host for host in self.approved_hosts()}
        host = self._speed_machines.get(host_id)
        return host is not None and host["created"] <= at and (host["retired"] is None or at < host["retired"])

    # -- The speed index -------------------------------------------------

    @staticmethod
    def gitlab_key(base_url, project_path, name):
        """A GitLab job's key: `gitlab:<instance>/<project path> / <job name>`,
        the instance its connection's URL without the scheme or a trailing
        slash, the name's shard counter stripped as a GitHub job's (#253)."""
        instance = base_url.split("://", 1)[1] if "://" in base_url else base_url
        github = Oracle.job_key("", "", name)
        return f"gitlab:{instance.rstrip('/')}/{project_path} / {github[len(' /  / '):]}"

    @staticmethod
    def job_key(repository, workflow, name):
        trimmed = name.strip()
        stripped = trimmed
        if trimmed:
            last = trimmed[-1]
            opened_by = {")": "(", "]": "["}.get(last, " ")
            body = trimmed if opened_by == " " else trimmed[:-1]
            at = body.rfind(opened_by)
            if at >= 0:
                counter = body[at + 1 :]
                shard, _, of = counter.partition("/")
                if _ and shard.isdigit() and of.isdigit() and shard.isascii() and of.isascii():
                    stripped = body[:at].strip()
        return f"{repository} / {workflow} / {stripped}"

    @staticmethod
    def median(values):
        values = sorted(values)
        middle = len(values) // 2
        if len(values) % 2 == 1:
            return values[middle]
        return rnd((values[middle - 1] + values[middle]) / 2.0)

    def speed(self):
        """Per (machine, runner size): the run-count-weighted geometric mean of
        own median / fleet median over keys with 5+ runs there and 5+ on the
        rest of the fleet; fewer than 3 keys is no index."""
        start = self.now - self.length
        by_run = {}
        for row in self.q(
            """SELECT wj.id,wj.run_id,sync.run_attempt AS attempt FROM workflow_jobs wj
                 LEFT JOIN workflow_job_sync sync ON sync.job_id=wj.id"""
        ):
            by_run.setdefault(row["run_id"], []).append(dict(row))
        machines = {}
        fleet = {}
        for job in self.github_jobs():
            if job["status"] != "completed" or job["conclusion"] != "success":
                continue
            if not (start <= job["completed_at"] < self.now) or job["started_at"] is None:
                continue
            if job["repository"] is None or job["workflow_name"] is None:
                continue
            # The stricter per-run rule: left out when any later attempt of
            # its run is known, whatever the job's name.
            if job["attempt"] is not None:
                if job["attempt"] < (job["run_attempt"] or 1):
                    continue
                if any(
                    other["attempt"] is not None and other["attempt"] > job["attempt"]
                    for other in by_run.get(job["run_id"], [])
                ):
                    continue
            runner = self.runner_row("github_runner_id", job["runner_id"])
            placement = self.placement_of_runner(runner)
            if placement is None or not self.speed_machine(placement["host_id"], job["completed_at"]):
                continue
            key = self.job_key(job["repository"], job["workflow_name"], job["name"])
            duration = job["completed_at"] - job["started_at"]
            machines.setdefault((placement["host_id"], placement["cpu_millis"]), {}).setdefault(key, []).append(duration)
            fleet.setdefault(key, []).append(duration)
        # GitLab jobs (#253): each its own only attempt; one with no name,
        # path or start has no key; a job recorded under several connections
        # of one instance counts once (its lowest row id); its machine is the
        # newest runner with its runner id on the same instance.
        for job in self.q(
            """SELECT gj.id,gj.job_id,gj.job_name,gj.project_path,gj.gitlab_runner_id,
                      gj.started_at,gj.completed_at,c.base_url
                 FROM gitlab_jobs gj JOIN gitlab_connections c ON c.id=gj.connection_id
                WHERE gj.status='success' AND gj.completed_at>=? AND gj.completed_at<?
                  AND gj.started_at IS NOT NULL AND gj.job_name IS NOT NULL AND gj.project_path IS NOT NULL""",
            start,
            self.now,
        ):
            twin = self.q(
                """SELECT 1 FROM gitlab_jobs other JOIN gitlab_connections oc ON oc.id=other.connection_id
                    WHERE other.job_id=? AND oc.base_url=? AND other.id<?""",
                job["job_id"],
                job["base_url"],
                job["id"],
            )
            if twin:
                continue
            runner = self.q(
                """SELECT r.id FROM runners r JOIN gitlab_connections rc ON rc.id=r.gitlab_connection_id
                    WHERE r.gitlab_runner_id=? AND rc.base_url=? ORDER BY r.created_at DESC,r.id DESC LIMIT 1""",
                job["gitlab_runner_id"],
                job["base_url"],
            )
            placement = self.placement_of_runner(runner[0]["id"] if runner else None)
            if placement is None or not self.speed_machine(placement["host_id"], job["completed_at"]):
                continue
            key = self.gitlab_key(job["base_url"], job["project_path"], job["job_name"])
            duration = job["completed_at"] - job["started_at"]
            machines.setdefault((placement["host_id"], placement["cpu_millis"]), {}).setdefault(key, []).append(duration)
            fleet.setdefault(key, []).append(duration)
        results = []
        for (host, cpu), keys in sorted(machines.items()):
            qualifying = []
            for key, durations in sorted(keys.items()):
                host_runs = len(durations)
                fleet_runs = len(fleet[key])
                if host_runs < 5 or fleet_runs - host_runs < 5:
                    continue
                host_median = self.median(durations)
                fleet_median = self.median(fleet[key])
                if host_median <= 0 or fleet_median <= 0:
                    continue
                qualifying.append(
                    (
                        host_median / fleet_median,
                        {
                            "key": key,
                            "hostMedianMs": host_median,
                            "fleetMedianMs": fleet_median,
                            "hostRuns": host_runs,
                            "fleetRuns": fleet_runs,
                        },
                    )
                )
            if len(qualifying) < 3:
                index, top = None, []
            else:
                weight = sum(entry["hostRuns"] for _, entry in qualifying)
                index = math.exp(sum(math.log(ratio) * entry["hostRuns"] for ratio, entry in qualifying) / weight)
                slower = [(ratio, entry) for ratio, entry in qualifying if ratio > 1.0]
                slower.sort(key=lambda pair: (-pair[0], pair[1]["key"]))
                top = [entry for _, entry in slower[:5]]
            results.append({"host": host, "cpuMillis": cpu, "index": index, "topKeys": top})
        return results

    # -- Scores ----------------------------------------------------------

    def covered(self):
        return self.now - self.first

    @staticmethod
    def component(key, weight, value, score_of, sentence, no_data):
        """One component: its unrounded score, the sentence that explains a
        deduction (before its points), and what "not enough data" says."""
        if value is None:
            return {"key": key, "weight": weight, "raw": None, "body": None, "no_data": no_data}
        return {"key": key, "weight": weight, "raw": score_of(value), "body": sentence(value), "no_data": no_data}

    @staticmethod
    def combine(parts):
        scored = sum(part["weight"] for part in parts if part["raw"] is not None)
        out = []
        for part in parts:
            if part["raw"] is not None and scored > 0:
                weight = part["weight"] * 100.0 / scored
            elif part["raw"] is None and scored == 0:
                weight = part["weight"]
            else:
                weight = 0.0
            # The deduction: what "not enough data" says without a score,
            # none at 100, else the sentence and the points lost from the
            # weight after reweighting.
            if part["raw"] is None:
                deduction = f"Not enough data: {part['no_data']}."
            elif rnd(part["raw"]) >= 100:
                deduction = None
            else:
                deduction = f"{part['body']}: \u2212{rnd(weight * (100.0 - part['raw']) / 100.0)}."
            out.append(
                {
                    "key": part["key"],
                    "weight": weight,
                    "score": None if part["raw"] is None else rnd(part["raw"]),
                    "raw": part["raw"],
                    "deduction": deduction,
                }
            )
        total = None
        if scored > 0:
            total = rnd(sum(part["raw"] * entry["weight"] for part, entry in zip(parts, out) if part["raw"] is not None) / 100.0)
        return total, out

    def host_covered(self, host):
        """#262: the whole minutes and the seconds of the covered time `host`
        lived in, from the minute it was created in to the minute it was
        retired in (the minute in progress, as `now`'s, not counted)."""
        begin, until = self.span(host, self.first, self.now)
        if begin >= until:
            return 0, 0.0
        return until // MINUTE - begin // MINUTE, (until - begin) / 1000.0

    def score(self, hosts, jobs, host_rows, speed):
        total_jobs = len(jobs)
        enough = total_jobs >= 20
        wait = [0] * 9
        for job in jobs:
            wait[bucket(job["wait"])] += 1
        attempts = sum(row["attempts"] for row in host_rows)
        failures = sum(row["failures"] for row in host_rows)
        infra = sum(1 for job in jobs if job["infra"])

        def starts(attempts, failures):
            return max(attempts - failures, 0) / attempts if attempts >= 20 else None

        # Capacity fit: of the ended hours, the share in which a job waited
        # a minute or more while the sampled machines' CPU p50 was under half
        # their budget.
        misfit = None
        if enough:
            hours = self.covered() // HOUR
            count = 0
            for index in range(hours):
                hour = self.first + index * HOUR
                waiting = sum(1 for job in jobs if job["hour"] == hour and job["wait"] >= 60_000)
                if waiting == 0:
                    continue
                used = budget = 0.0
                for row in host_rows:
                    if row["hour"] != hour or row["cpu_p50"] is None:
                        continue
                    host = next((h for h in hosts if h["id"] == row["host"]), None)
                    if host is None or not self.lives_in_hour(host, hour):
                        continue
                    used += row["cpu_p50"]
                    budget += host["cpu"]
                if budget > 0 and used < budget / 2.0:
                    count += 1
            misfit = count / hours if hours > 0 else None

        # Each machine's online minutes over its own covered minutes, weighted
        # by its budget (#262).
        online = 0.0
        budget = 0.0
        for host in hosts:
            covered_minutes, _ = self.host_covered(host)
            minutes = sum(
                row["online"] for row in host_rows if row["host"] == host["id"] and self.lives_in_hour(host, row["hour"])
            )
            online += min(minutes, covered_minutes) * host["cpu"]
            budget += covered_minutes * host["cpu"]
        availability = online / budget if budget > 0 else None

        few_jobs = "fewer than 20 jobs completed in the window"
        few_starts = "fewer than 20 runner starts in the window"
        p90 = percentile(wait, 0.9) if enough else None
        queue_part = self.component(
            "queue-wait",
            40.0,
            p90,
            lambda v: linear_lower(v, 60_000.0, 600_000.0),
            lambda v: f"Queue p90 was {duration_text(v)} against a 1 min target",
            few_jobs,
        )
        fleet_queue = None if queue_part["raw"] is None else rnd(queue_part["raw"])

        def starts_sentence(share):
            return f"{percent(share)} of runner starts succeeded against a 99% target"

        fleet_parts = [
            queue_part,
            self.component(
                "start-reliability",
                20.0,
                starts(attempts, failures),
                lambda v: linear_higher(v, 0.99, 0.80),
                starts_sentence,
                few_starts,
            ),
            self.component(
                "infra-failures",
                15.0,
                infra / total_jobs if enough and total_jobs > 0 else None,
                lambda v: linear_lower(v, 0.0, 0.05),
                lambda v: f"{percent(v)} of completed jobs failed on the infrastructure against a 0% target",
                few_jobs,
            ),
            self.component(
                "capacity-fit",
                15.0,
                misfit,
                lambda v: linear_lower(v, 0.0, 0.25),
                lambda v: f"{percent(v)} of the window's hours waited over a minute while the fleet sat under half its CPU budget, against a 0% target",
                few_jobs,
            ),
            self.component(
                "availability",
                10.0,
                availability,
                lambda v: linear_higher(v, 1.0, 0.80),
                lambda v: f"The fleet's CPU budget was online {percent(v)} of the window against a 100% target",
                "no machine samples in the window",
            ),
        ]
        fleet_score, fleet_components = self.combine(fleet_parts)

        scored_hosts = []
        for host in hosts:
            covered_minutes, covered_seconds = self.host_covered(host)
            rows = [row for row in host_rows if row["host"] == host["id"] and self.lives_in_hour(host, row["hour"])]
            own_jobs = [job for job in jobs if job["host"] == host["id"] and self.lives_in_hour(host, job["hour"])]
            minutes = sum(row["online"] for row in rows)
            h_attempts = sum(row["attempts"] for row in rows)
            h_failures = sum(row["failures"] for row in rows)
            indexes = [entry["index"] for entry in speed if entry["host"] == host["id"] and entry["index"] is not None]
            index = max(indexes) if indexes else None
            disks = [row["min_disk"] for row in rows if row["min_disk"] is not None]
            disk = None
            if disks and host["disk"]:
                total = host["disk"]
                warn = max(10 * GIB, total // 10)
                critical = max(5 * GIB, total // 20)
                # The floors are the frees that clear each alarm: a free at or
                # above `warn` raises none, under `critical` the critical one.
                disk = (
                    linear_higher(float(min(disks)), float(warn), float(critical)),
                    f"The least free disk was {gib_text(min(disks))}, between the {gib_text(warn)} warning and {gib_text(critical)} critical floors",
                )
            memories = [row["memory_p95"] for row in rows if row["memory_p95"] is not None]
            memory = None
            if memories and host["memory"] > 0:
                used = max(memories) / host["memory"]
                memory = (
                    linear_lower(used, 0.80, 1.0),
                    f"Memory p95 was {whole_percent(used)} of budget against a 80% target",
                )
            # The lower of the two, the disk on a tie.
            headroom = None
            if disk is not None and memory is not None:
                headroom = memory if memory[0] < disk[0] else disk
            elif disk is not None:
                headroom = disk
            elif memory is not None:
                headroom = memory
            budgeted = host["cpu"] > 0
            busy = None
            if budgeted and covered_seconds > 0:
                cpu_seconds = sum(job["cpu_seconds"] for job in own_jobs)
                busy = cpu_seconds / (host["cpu"] / 1000.0 * covered_seconds)
            avail = minutes / covered_minutes if rows and covered_minutes > 0 else None
            deducts = fleet_queue is not None and fleet_queue < 100
            parts = [
                self.component(
                    "availability",
                    25.0,
                    avail,
                    lambda v: linear_higher(v, 1.0, 0.80),
                    lambda v: f"Online {percent(v)} of the window against a 100% target",
                    "no samples from this machine in the window",
                ),
                self.component(
                    "start-reliability",
                    25.0,
                    starts(h_attempts, h_failures),
                    lambda v: linear_higher(v, 0.99, 0.80),
                    starts_sentence,
                    few_starts,
                ),
                self.component(
                    "relative-speed",
                    25.0,
                    index,
                    lambda v: linear_lower(v, 1.0, 2.0),
                    lambda v: f"Speed index {v:.2f} against a 1.00 target",
                    "fewer than 3 job keys qualify",
                ),
                self.component(
                    "headroom",
                    15.0,
                    headroom,
                    lambda v: v[0],
                    lambda v: v[1],
                    "no disk or memory samples in the window",
                ),
                self.component(
                    "busy-share",
                    10.0,
                    busy,
                    lambda v: linear_higher(v, 0.50, 0.0) if deducts else 100.0,
                    lambda v: f"Busy {whole_percent(v)} of its CPU budget against a 50% target while jobs waited",
                    "the machine has no CPU budget",
                ),
            ]
            if not budgeted:
                # No CPU budget, no score: a component that had data says so.
                parts = [
                    dict(
                        part,
                        raw=None,
                        no_data="the machine has no CPU budget" if part["raw"] is not None else part["no_data"],
                    )
                    for part in parts
                ]
            total, components = self.combine(parts)
            scored_hosts.append(
                {"id": host["id"], "name": host["name"], "score": total, "components": components, "busy": busy}
            )
        return fleet_score, fleet_components, fleet_queue, scored_hosts

    # -- Usage -----------------------------------------------------------

    @staticmethod
    def categories(jobs):
        out = []
        for category in CATEGORIES:
            mine = [job for job in jobs if job["category"] == category]
            if not mine:
                continue
            durations = [0] * 9
            waits = [0] * 9
            for job in mine:
                durations[bucket(job["duration"])] += 1
                waits[bucket(job["wait"])] += 1
            median = percentile(durations, 0.5)
            p90 = percentile(waits, 0.9)
            out.append(
                {
                    "category": category,
                    "jobs": len(mine),
                    "cpuHours": rnd(sum(job["cpu_seconds"] for job in mine) / 3600.0 * 100.0) / 100.0,
                    "medianDurationMs": None if median is None else rnd(median),
                    "p90QueueWaitMs": None if p90 is None else rnd(p90),
                }
            )
        return out

    @staticmethod
    def hourly(jobs):
        hours = {}
        for job in jobs:
            if job["cpu_seconds"] > 0:
                entry = hours.setdefault(job["hour"], {})
                entry[job["category"]] = entry.get(job["category"], 0.0) + job["cpu_seconds"]
        return [
            {"hour": hour, "cpuHours": {c: rnd(s / 3600.0 * 100.0) / 100.0 for c, s in hours[hour].items()}}
            for hour in sorted(hours)
        ]

    def trend(self):
        rows = self.q(
            """SELECT recorded_at,fleet_score FROM fleet_health_snapshots
                WHERE recorded_at>=? AND recorded_at<=? AND "window"=? ORDER BY recorded_at""",
            self.now - self.length,
            self.now,
            self.window,
        )
        return [{"at": row["recorded_at"], "score": row["fleet_score"]} for row in rows]

    # -- The live half ---------------------------------------------------

    def live(self, hosts):
        now = self.now
        concerns = []
        host_live = []
        fleet_running = {}
        for host in hosts:
            recent = self.q(
                "SELECT MAX(observed_at) AS at FROM host_samples WHERE host_id=? AND observed_at>=?",
                host["id"],
                now - 5 * MINUTE,
            )[0]["at"]
            last_seen = recent
            if last_seen is None:
                last_seen = self.q("SELECT MAX(observed_at) AS at FROM host_samples WHERE host_id=?", host["id"])[0]["at"]
            running = {}
            for row in self.q(
                """SELECT r.current_job_id,r.ci_platform,r.status,r.busy,r.gitlab_runner_id,r.gitlab_connection_id
                     FROM workload_placements p
                     JOIN workload_intents i ON i.id=p.intent_id
                     JOIN runners r ON r.id=p.runner_id
                    WHERE p.host_id=? AND p.state='running' AND i.kind='ci_runner'""",
                host["id"],
            ):
                if row["current_job_id"] is None and not (row["status"] == "busy" and row["busy"] == 1):
                    continue
                category = "other"
                if row["ci_platform"] == "github" and row["current_job_id"] is not None:
                    job = self.q("SELECT category FROM workflow_jobs WHERE id=?", row["current_job_id"])
                    if job and job[0]["category"] in CATEGORIES:
                        category = job[0]["category"]
                if row["ci_platform"] == "gitlab":
                    # The GitLab job running on it (#253): its runner id on
                    # the runner's instance, newest first.
                    job = self.q(
                        """SELECT gj.category FROM gitlab_jobs gj JOIN gitlab_connections c ON c.id=gj.connection_id
                            WHERE gj.status='running' AND gj.gitlab_runner_id=?
                              AND c.base_url=(SELECT base_url FROM gitlab_connections WHERE id=?)
                            ORDER BY gj.status_changed_at DESC,gj.id DESC LIMIT 1""",
                        row["gitlab_runner_id"],
                        row["gitlab_connection_id"],
                    )
                    if job and job[0]["category"] in CATEGORIES:
                        category = job[0]["category"]
                running[category] = running.get(category, 0) + 1
            for category, count in running.items():
                fleet_running[category] = fleet_running.get(category, 0) + count
            newest = self.q(
                "SELECT cpu_used_millis,memory_used_mib,disk_free_bytes FROM host_samples WHERE host_id=? ORDER BY id DESC LIMIT 1",
                host["id"],
            )
            # Reserved: the capacity charged on the machine's root domain,
            # every allocation not yet released.
            reserved = self.q(
                """SELECT COALESCE(SUM(a.cpu_millis),0) AS cpu,COALESCE(SUM(a.memory_mib),0) AS memory
                     FROM capacity_allocations a
                     JOIN host_resource_domains d ON d.id=a.domain_id AND d.host_id=a.host_id
                    WHERE a.host_id=? AND d.parent_domain_id IS NULL
                      AND a.state IN ('reserved','committed','releasing','uncertain')""",
                host["id"],
            )[0]
            failures = sorted(
                row["created_at"]
                for row in self.q(
                    """SELECT created_at,detail_json FROM fleet_events
                        WHERE host_id=? AND kind='placement_failed' AND created_at>=? AND created_at<=?""",
                    host["id"],
                    now - HOUR,
                    now,
                )
                if is_start_failure(row["detail_json"])
            )
            host_live.append(
                {
                    "hostId": host["id"],
                    "startFailuresLastHour": len(failures),
                    "cpuReservedMillis": reserved["cpu"],
                    "memoryReservedMib": reserved["memory"],
                    "online": recent is not None,
                    "lastSeenAt": last_seen,
                    "runningByCategory": running,
                    "cpuUsedMillis": newest[0]["cpu_used_millis"] if newest else None,
                    "memoryUsedMib": newest[0]["memory_used_mib"] if newest else None,
                    "diskFreeBytes": newest[0]["disk_free_bytes"] if newest else None,
                }
            )
            # start-failures (#253): 3 or more in the hour, at least a quarter
            # of its starts (the placements issued a start command in it,
            # never fewer than the failures), critical from a half.
            if len(failures) >= 3:
                starts = self.q(
                    """SELECT COUNT(DISTINCT placement_id) AS n FROM host_commands
                        WHERE host_id=? AND kind='start_runner' AND issued_at>=? AND issued_at<=?""",
                    host["id"],
                    now - HOUR,
                    now,
                )[0]["n"]
                starts = max(starts, len(failures))
                share = len(failures) * 100
                severity = "critical" if share >= 50 * starts else "warning" if share >= 25 * starts else None
                if severity:
                    concerns.append({"key": f"start-failures:{host['id']}", "severity": severity, "since": failures[0]})
            if host["cpu"] > 0 and recent is None:
                concerns.append({"key": f"host-offline:{host['id']}", "severity": "critical", "since": last_seen})
            gate = self.start_gate(host["id"])
            if gate is not None:
                concerns.append({"key": f"start-gate:{host['id']}", "severity": "warning", "since": gate})
            # The disk alarm: a heartbeat in the last 120 s, the newest sample's
            # free space against the observed root disk.
            if host["heartbeat"] is not None and host["heartbeat"] >= now - 120_000 and host["lifecycle"] != "retired":
                free = self.q(
                    """SELECT disk_free_bytes FROM host_samples WHERE host_id=? AND disk_free_bytes IS NOT NULL
                        ORDER BY observed_at DESC LIMIT 1""",
                    host["id"],
                )
                if free and host["disk"] is not None:
                    alarm = disk_alarm(host["disk"], free[0]["disk_free_bytes"])
                    if alarm:
                        concerns.append({"key": f"disk:{host['id']}", "severity": alarm, "since": None})
            uncertain = self.q(
                "SELECT COUNT(*) AS n FROM workload_placements WHERE host_id=? AND state='uncertain'", host["id"]
            )[0]["n"]
            if uncertain > 0:
                concerns.append({"key": f"uncertain:{host['id']}", "severity": "warning", "since": None})

        pools = self.q("SELECT id,name,labels,repository_id FROM runner_pools")
        queued = self.q(
            """SELECT wj.id,wj.labels,wj.created_at,wj.category,run.repository_id FROM workflow_jobs wj
                 JOIN workflow_runs run ON run.id=wj.run_id
                WHERE wj.status IN ('queued','waiting','pending','requested')"""
        )
        by_category = {}
        by_pool = {}
        unmatched = 0
        oldest = None
        for job in queued:
            category = job["category"] if job["category"] in CATEGORIES else "other"
            by_category[category] = by_category.get(category, 0) + 1
            labels = {label.lower() for label in json.loads(job["labels"] or "[]")}
            pool = next(
                (
                    pool
                    for pool in pools
                    if pool["repository_id"] in (None, job["repository_id"])
                    and labels <= {label.lower() for label in json.loads(pool["labels"] or "[]")} | {pool["name"].lower()}
                ),
                None,
            )
            if pool is None:
                unmatched += 1
                continue
            by_pool[pool["id"]] = by_pool.get(pool["id"], 0) + 1
            wait = now - job["created_at"]
            if oldest is None or wait > oldest["waitMs"]:
                oldest = {"jobId": job["id"], "waitMs": wait, "queuedAt": job["created_at"]}
        # GitLab jobs waiting for a runner (#253): a job is one per instance
        # and job id over its active rows, waiting while its best row is
        # `pending` (none `running`); it is shown under a pool when a pool
        # linked to one of its connections, running on some platform, offers
        # all its tags (or takes untagged jobs when it has none): an
        # autoscaling one first, else a manual one. A job an autoscaling pool
        # runs can be the oldest, when it has waited longer than the GitHub
        # one.
        gitlab_oldest = None
        for job in self.waiting_gitlab_jobs():
            category = job["category"] if job["category"] in CATEGORIES else "other"
            by_category[category] = by_category.get(category, 0) + 1
            owner = self.gitlab_owner(job)
            if owner is None:
                unmatched += 1
                continue
            by_pool[owner["id"]] = by_pool.get(owner["id"], 0) + 1
            if not owner["manual"] and (gitlab_oldest is None or job["queued_at"] < gitlab_oldest["queued_at"]):
                gitlab_oldest = job
        waiting_gitlab = len(self.waiting_gitlab_jobs())
        if gitlab_oldest is not None and (oldest is None or now - gitlab_oldest["queued_at"] > oldest["waitMs"]):
            oldest = {
                "jobId": gitlab_oldest["job_id"],
                "waitMs": now - gitlab_oldest["queued_at"],
                "queuedAt": gitlab_oldest["queued_at"],
            }
        if unmatched > 0:
            concerns.append({"key": "unmatched-jobs", "severity": "critical", "since": None})
        if oldest is not None and oldest["waitMs"] > 5 * MINUTE:
            severity = "critical" if oldest["waitMs"] > 10 * MINUTE else "warning"
            concerns.append({"key": "oldest-queued", "severity": severity, "since": oldest["queuedAt"]})
        fleet = {
            "queuedTotal": len(queued) + waiting_gitlab,
            "queuedByCategory": by_category,
            "queuedByPool": sorted(by_pool.items(), key=lambda pair: (-pair[1], pair[0])),
            "queuedUnmatched": unmatched,
            "oldestQueued": None if oldest is None else {"jobId": oldest["jobId"], "waitMs": oldest["waitMs"]},
            "runningTotal": sum(fleet_running.values()),
            "runningByCategory": fleet_running,
        }
        return fleet, host_live, concerns

    def waiting_gitlab_jobs(self):
        """The GitLab jobs waiting for a runner, one per (instance, job id)."""
        groups = {}
        for row in self.q(
            """SELECT gj.*,c.base_url FROM gitlab_jobs gj JOIN gitlab_connections c ON c.id=gj.connection_id
                WHERE gj.status IN ('created','waiting_for_resource','preparing','pending','running')"""
        ):
            groups.setdefault((row["base_url"], row["job_id"]), []).append(dict(row))
        jobs = []
        for (base_url, job_id), rows in sorted(groups.items()):
            tier = max(2 if row["status"] == "running" else 1 if row["status"] == "pending" else 0 for row in rows)
            if tier != 1:
                continue
            categories = [row["category"] for row in rows if row["category"] is not None]
            tags = [row["tag_list"] for row in rows if row["tag_list"] != "[]"]
            jobs.append(
                {
                    "base_url": base_url,
                    "job_id": job_id,
                    "queued_at": min(row["created_at"] for row in rows),
                    "category": max(categories) if categories else None,
                    "tags": json.loads(max(tags)) if tags else [],
                    "connections": {row["connection_id"] for row in rows},
                }
            )
        return jobs

    def gitlab_owner(self, job):
        """The pool a waiting GitLab job is shown under: the first pool (a
        repository-scoped one first, then the oldest) linked to one of its
        connections, running on some platform, and offering all its tags (or
        taking untagged jobs when it has none), autoscaling (not paused, its
        circuit closed) before manual (autoscaling off); `None` when none
        will run it. Among several autoscaling pools the API picks the first
        with room, which the seeded fleet (one pool) never has to choose."""
        wanted = {str(tag).lower() for tag in job["tags"]}
        scaling, manual = [], []
        for pool in self.q(
            """SELECT * FROM runner_pools ORDER BY CASE WHEN scope='repository' THEN 0 ELSE 1 END,created_at,id"""
        ):
            linked = self.q("SELECT connection_id FROM runner_pool_gitlab_connections WHERE pool_id=?", pool["id"])
            if not {row["connection_id"] for row in linked} & job["connections"]:
                continue
            providers = [str(provider).lower() for provider in json.loads(pool["providers"] or "[]")]
            profiles = self.q(
                "SELECT 1 FROM pool_execution_profiles WHERE pool_id=? AND enabled=1 AND archived_at IS NULL",
                pool["id"],
            )
            if not profiles and not any(provider in ("docker", "tart") for provider in providers):
                continue
            offered = {str(label).lower() for label in json.loads(pool["labels"] or "[]")}
            if (not wanted and not pool["run_untagged"]) or not wanted <= offered:
                continue
            if pool["paused"]:
                continue
            if not pool["autoscaling_enabled"]:
                manual.append(pool)
            elif not pool["provision_circuit_open"]:
                scaling.append(pool)
        if scaling:
            return {"id": scaling[0]["id"], "manual": False}
        if manual:
            return {"id": manual[0]["id"], "manual": True}
        return None

    def start_gate(self, host_id):
        """Three or more machine-fault start failures in the last 15 minutes
        since the last successful start set a machine aside; `since` is the
        oldest of them."""
        window_start = self.now - 15 * MINUTE
        failures = sorted(
            row["created_at"]
            for row in self.q(
                """SELECT created_at,detail_json FROM fleet_events
                    WHERE host_id=? AND kind='placement_failed' AND created_at>=? AND created_at<=?""",
                host_id,
                window_start,
                self.now,
            )
            if is_start_failure(row["detail_json"])
        )
        return failures[0] if len(failures) >= 3 else None

    # -- Concerns --------------------------------------------------------

    def windowed_concerns(self, hosts, fleet_components, fleet_queue, scored_hosts, speed, jobs):
        concerns = []

        def band(score):
            if score < 50:
                return "critical"
            if score < 70:
                return "warning"
            return None

        for component in fleet_components:
            if component["score"] is not None and band(component["score"]):
                concerns.append({"key": f"fleet-component:{component['key']}", "severity": band(component["score"]), "since": None})
        for host in scored_hosts:
            if host["score"] is not None and band(host["score"]) and any(c["score"] is not None for c in host["components"]):
                concerns.append({"key": f"host-score:{host['id']}", "severity": band(host["score"]), "since": None})
        for host in hosts:
            for entry in speed:
                if entry["host"] == host["id"] and entry["index"] is not None and entry["index"] >= 1.3:
                    concerns.append({"key": f"host-slow:{host['id']}:{entry['cpuMillis']}", "severity": "warning", "since": None})
        for host in scored_hosts:
            if host["busy"] is not None and fleet_queue is not None and host["busy"] < 0.30 and fleet_queue < 70:
                concerns.append({"key": f"host-idle-while-queued:{host['id']}", "severity": "warning", "since": None})
        total = len(jobs)
        other = sum(1 for job in jobs if job["category"] == "other")
        if total > 0 and other / total > 0.20:
            concerns.append({"key": "category-other", "severity": "info", "since": None})
        return concerns

    @staticmethod
    def merge(windowed, live):
        live_keys = {concern["key"] for concern in live}
        merged = [concern for concern in windowed if concern["key"] not in live_keys] + live
        merged.sort(
            key=lambda concern: (
                SEVERITY_ORDER[concern["severity"]],
                concern["since"] is None,
                concern["since"] or 0,
            )
        )
        return merged

    # -- Classification --------------------------------------------------

    def classification(self):
        """Every job classified by the stored rules, in order (case-insensitive,
        the first match decides, none is `other`), against its stored
        category."""
        rules = [
            (row["category"], row["field"], re.compile(row["pattern"], re.IGNORECASE))
            for row in self.q("SELECT category,field,pattern FROM job_category_rules ORDER BY position")
        ]

        def classify(name, workflow, repository, labels):
            for category, field, pattern in rules:
                if field == "job_name" and name is not None and pattern.search(name):
                    return category
                if field == "workflow_name" and workflow is not None and pattern.search(workflow):
                    return category
                if field == "repository" and repository is not None and pattern.search(repository):
                    return category
                if field == "labels" and any(pattern.search(label) for label in labels):
                    return category
            return "other"

        jobs = []
        for row in self.q(
            """SELECT wj.id,wj.name,wj.labels,wj.category,run.workflow_name,repo.full_name AS repository
                 FROM workflow_jobs wj
                 LEFT JOIN workflow_runs run ON run.id=wj.run_id
                 LEFT JOIN repositories repo ON repo.id=run.repository_id"""
        ):
            ruled = classify(row["name"], row["workflow_name"], row["repository"], json.loads(row["labels"] or "[]"))
            jobs.append({"job": f"github:{row['id']}", "stored": row["category"], "rules": ruled})
        for row in self.q(
            "SELECT id,connection_id,project_id,job_name,tag_list,project_path,category FROM gitlab_jobs"
        ):
            ruled = classify(
                row["job_name"] or "", None, self.gitlab_path(row), json.loads(row["tag_list"] or "[]")
            )
            jobs.append({"job": f"gitlab:{row['id']}", "stored": row["category"], "rules": ruled})
        return jobs

    def gitlab_path(self, row):
        """A GitLab job's project path: its own, else its connection's when the
        connection is scoped to that project, else another of the project's
        rows' path."""
        if row["project_path"] is not None:
            return row["project_path"]
        scoped = self.q(
            "SELECT scope_path FROM gitlab_connections WHERE id=? AND scope_kind='project' AND scope_id=?",
            row["connection_id"],
            row["project_id"],
        )
        if scoped:
            return scoped[0]["scope_path"]
        sibling = self.q(
            """SELECT project_path FROM gitlab_jobs WHERE connection_id=? AND project_id=?
                AND project_path IS NOT NULL ORDER BY job_id LIMIT 1""",
            row["connection_id"],
            row["project_id"],
        )
        return sibling[0]["project_path"] if sibling else None

    # -- Everything ------------------------------------------------------

    def run(self):
        hosts = self.hosts()
        jobs = self.counted_jobs(self.first, self.now)
        host_rows = self.host_hours()
        speed = self.speed()
        fleet_score, fleet_components, fleet_queue, scored_hosts = self.score(hosts, jobs, host_rows, speed)
        live_fleet, host_live, live_concerns = self.live(self.live_hosts())
        windowed = self.windowed_concerns(hosts, fleet_components, fleet_queue, scored_hosts, speed, jobs)
        concerns = self.merge(windowed, live_concerns)
        out_hosts = []
        for host, scored in zip(hosts, scored_hosts):
            own = [job for job in jobs if job["host"] == host["id"] and self.lives_in_hour(host, job["hour"])]
            out_hosts.append(
                {
                    "id": host["id"],
                    "name": host["name"],
                    "cpuBudgetMillis": host["cpu"],
                    "memoryBudgetMib": host["memory"],
                    "diskTotalBytes": host["disk"],
                    "score": scored["score"],
                    "components": [
                        {"key": c["key"], "weight": c["weight"], "score": c["score"], "deduction": c["deduction"]} for c in scored["components"]
                    ],
                    "speed": [
                        {"cpuMillis": entry["cpuMillis"], "index": entry["index"], "topKeys": entry["topKeys"]}
                        for entry in sorted(speed, key=lambda entry: entry["cpuMillis"])
                        if entry["host"] == host["id"]
                    ],
                    "categories": self.categories(own),
                    "usage": self.hourly(own),
                    "live": next((entry for entry in host_live if entry["hostId"] == host["id"]), None),
                }
            )
        return {
            "window": self.window,
            "now": self.now,
            "fleet": {
                "score": fleet_score,
                "components": [{"key": c["key"], "weight": c["weight"], "score": c["score"], "deduction": c["deduction"]} for c in fleet_components],
                "trend": self.trend(),
                "categories": self.categories(jobs),
            },
            "live": live_fleet,
            "hosts": out_hosts,
            "concerns": [{"key": c["key"], "severity": c["severity"], "since": c["since"]} for c in concerns],
            "classification": self.classification(),
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("database")
    parser.add_argument("--now", type=int, required=True, help="epoch milliseconds")
    parser.add_argument("--window", choices=sorted(WINDOWS), default="24h")
    args = parser.parse_args()
    json.dump(Oracle(args.database, args.now, args.window).run(), sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
