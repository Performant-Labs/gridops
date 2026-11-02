//! How the Overview splits the queue: every job waiting to start is either
//! waiting for capacity (some pool the viewer can see serves it) or has no
//! matching pool (none does), so the two always add up to the Queued tile, and
//! the pool rows add up to the first part.
//!
//! Which pool serves a job is decided once, by the same ownership the
//! reconciler scales from ([`waiting_job_ownership`]); nothing here
//! matches labels itself.

use std::collections::{BTreeMap, HashMap, HashSet};

use gridops_core::{
    LINUX_RUNNER_LABEL, MACOS_RUNNER_LABEL, is_runner_system_label, waiting_job_ownership,
};
use serde_json::{Value, json};
use sqlx::Row as _;

use super::json_array;
use crate::{auth::AuthUser, error::ApiResult, state::AppState};

/// Longest list of distinct missing-label sets reported, most jobs first.
const MAX_MISSING_LABEL_SETS: usize = 20;
/// How many of them get their own alert.
const MAX_MISSING_LABEL_ALERTS: usize = 3;

/// A job waiting to start, as the Overview sees it.
#[derive(Debug, Clone)]
pub(super) struct WaitingJob {
    pub id: i64,
    pub labels: Vec<String>,
    pub created_at: i64,
}

/// A pool the viewer can see: every label a job can ask it for.
#[derive(Debug, Clone)]
pub(super) struct VisiblePool {
    pub labels: Vec<String>,
}

/// Jobs that ask for the same labels no pool has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MissingLabels {
    pub labels: Vec<String>,
    pub jobs: i64,
}

#[derive(Debug, Default)]
pub(super) struct QueueBreakdown {
    pub waiting: i64,
    pub unmatched: i64,
    /// Waiting jobs by the pool that serves them.
    pub by_pool: HashMap<String, i64>,
    pub oldest_waiting_millis: Option<i64>,
    /// Unmatched jobs asking for labels no pool has, most jobs first.
    pub missing_labels: Vec<MissingLabels>,
    /// Unmatched jobs that ask for no missing label: every pool that could run
    /// them is paused, tripped, on manual scaling or on another platform.
    pub unavailable: i64,
}

impl QueueBreakdown {
    pub fn total(&self) -> i64 {
        self.waiting + self.unmatched
    }

    /// Waiting jobs served by pools other than the listed ones.
    pub fn served_outside(&self, listed: &[String]) -> i64 {
        let listed = listed.iter().collect::<HashSet<_>>();
        self.by_pool
            .iter()
            .filter(|(pool, _)| !listed.contains(pool))
            .map(|(_, jobs)| *jobs)
            .sum()
    }

    pub fn alerts(&self) -> Vec<Value> {
        let mut alerts = self
            .missing_labels
            .iter()
            .take(MAX_MISSING_LABEL_ALERTS)
            .map(|missing| {
                let labels = missing
                    .labels
                    .iter()
                    .map(|label| format!("`{label}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let (needs, has) = match (missing.jobs, missing.labels.len()) {
                    (1, 1) => ("needs", "it"),
                    (1, _) => ("needs", "them"),
                    (_, 1) => ("need", "it"),
                    _ => ("need", "them"),
                };
                json!({
                    "level": "error",
                    "title": format!(
                        "{} {} {needs} {labels}; no pool has {has}",
                        missing.jobs,
                        if missing.jobs == 1 { "job" } else { "jobs" },
                    ),
                    "detail": "They stay queued until a runner pool is given the label.",
                    "href": "/queued-jobs",
                })
            })
            .collect::<Vec<_>>();
        if self.unavailable > 0 {
            alerts.push(json!({
                "level": "error",
                "title": format!(
                    "{} {} no available pool",
                    self.unavailable,
                    if self.unavailable == 1 { "job has" } else { "jobs have" },
                ),
                "detail": "Every pool that could run them is paused, has an open provisioning circuit, has autoscaling off or runs another platform.",
                "href": "/queued-jobs",
            }));
        }
        alerts
    }

    pub fn missing_labels_json(&self) -> Value {
        json!(
            self.missing_labels
                .iter()
                .map(|missing| json!({ "labels": missing.labels, "jobs": missing.jobs }))
                .collect::<Vec<_>>()
        )
    }
}

/// Splits `jobs` by whether a pool serves them. `serving` says which pool
/// serves each job; a job it has no pool for has no matching pool. A job
/// served by a pool the viewer can't see is waiting for capacity all the same
/// (the pool exists; it is just not one of the viewer's rows), so it is in
/// [`QueueBreakdown::served_outside`]. `pools` are the pools the viewer can
/// see, whose labels tell which labels exist.
pub(super) fn split_queue(
    jobs: &[WaitingJob],
    serving: &HashMap<i64, String>,
    pools: &[VisiblePool],
    now: i64,
) -> QueueBreakdown {
    // Labels some pool the viewer can see carries.
    let known = pools
        .iter()
        .flat_map(|pool| pool.labels.iter())
        .map(|label| label.to_lowercase())
        .collect::<HashSet<_>>();
    let mut breakdown = QueueBreakdown::default();
    let mut missing = BTreeMap::<Vec<String>, (Vec<String>, i64)>::new();
    for job in jobs {
        if let Some(pool) = serving.get(&job.id) {
            breakdown.waiting += 1;
            *breakdown.by_pool.entry(pool.clone()).or_default() += 1;
            let age = now.saturating_sub(job.created_at);
            breakdown.oldest_waiting_millis = breakdown.oldest_waiting_millis.max(Some(age));
            continue;
        }
        breakdown.unmatched += 1;
        let mut labels = job
            .labels
            .iter()
            .filter(|label| {
                !is_runner_system_label(label)
                    && !label.eq_ignore_ascii_case(LINUX_RUNNER_LABEL)
                    && !label.eq_ignore_ascii_case(MACOS_RUNNER_LABEL)
                    && !known.contains(&label.to_lowercase())
            })
            .cloned()
            .collect::<Vec<_>>();
        labels.sort_by_key(|label| label.to_lowercase());
        labels.dedup_by_key(|label| label.to_lowercase());
        if labels.is_empty() {
            breakdown.unavailable += 1;
        } else {
            let key = labels.iter().map(|label| label.to_lowercase()).collect();
            missing.entry(key).or_insert((labels, 0)).1 += 1;
        }
    }
    let mut missing = missing
        .into_values()
        .map(|(labels, jobs)| MissingLabels { labels, jobs })
        .collect::<Vec<_>>();
    missing.sort_by(|left, right| {
        right
            .jobs
            .cmp(&left.jobs)
            .then_with(|| left.labels.cmp(&right.labels))
    });
    missing.truncate(MAX_MISSING_LABEL_SETS);
    breakdown.missing_labels = missing;
    breakdown
}

/// The queue as `user` may see it: the jobs of their installations waiting to
/// start in a run that hasn't finished, split by whether a pool they can see
/// serves them.
pub(super) async fn queue_breakdown(
    state: &AppState,
    user: &AuthUser,
    now: i64,
) -> ApiResult<QueueBreakdown> {
    let job_rows = sqlx::query(concat!(
        "SELECT wj.id,wj.labels,wj.created_at ",
        visible_queued_jobs!()
    ))
    .bind(&user.id)
    .fetch_all(&state.database)
    .await?;
    let jobs = job_rows
        .iter()
        .map(|row| WaitingJob {
            id: row.get::<i64, _>("id"),
            labels: json_array(row.get::<&str, _>("labels")),
            created_at: row.get::<i64, _>("created_at"),
        })
        .collect::<Vec<_>>();
    let pool_rows = sqlx::query(
        r#"SELECT p.name,p.labels FROM runner_pools p
          WHERE EXISTS (SELECT 1 FROM runner_pool_installations mapped WHERE mapped.pool_id=p.id)
            AND NOT EXISTS (
              SELECT 1 FROM runner_pool_installations mapped WHERE mapped.pool_id=p.id
                AND NOT EXISTS (SELECT 1 FROM user_installations access
                  WHERE access.user_id=? AND access.installation_id=mapped.installation_id)
            )"#,
    )
    .bind(&user.id)
    .fetch_all(&state.database)
    .await?;
    let pools = pool_rows
        .iter()
        .map(|row| {
            // A pool's name is a label of it too.
            let mut labels = json_array(row.get::<&str, _>("labels"));
            labels.push(row.get::<String, _>("name"));
            VisiblePool { labels }
        })
        .collect::<Vec<_>>();
    let serving = if jobs.is_empty() {
        HashMap::new()
    } else {
        waiting_job_ownership(&state.database)
            .await?
            .into_iter()
            .map(|owner| (owner.job_id, owner.pool_id))
            .collect()
    };
    Ok(split_queue(&jobs, &serving, &pools, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: i64, labels: &[&str], created_at: i64) -> WaitingJob {
        WaitingJob {
            id,
            labels: labels.iter().map(|label| (*label).to_owned()).collect(),
            created_at,
        }
    }

    fn pool(labels: &[&str]) -> VisiblePool {
        VisiblePool {
            labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        }
    }

    #[test]
    fn jobs_split_into_served_and_unmatched_that_add_up() {
        let jobs = [
            job(1, &["self-hosted", "linux", "gridops-ci"], 10),
            job(2, &["self-hosted", "linux", "GridOps-CI"], 20),
            job(3, &["self-hosted-linux"], 30),
            job(4, &["paused-only"], 40),
            // Served by a pool the viewer can't see.
            job(5, &["elsewhere"], 50),
        ];
        let serving = HashMap::from([(3, "linux".to_owned()), (5, "hidden".to_owned())]);
        let pools = [pool(&["linux"]), pool(&["paused-only"])];

        let breakdown = split_queue(&jobs, &serving, &pools, 100);

        // Job 5 is served by a pool the viewer can't see: still waiting.
        assert_eq!((breakdown.waiting, breakdown.unmatched), (2, 3));
        assert_eq!(breakdown.total(), 5);
        assert_eq!(breakdown.served_outside(&["linux".into()]), 1);
        assert_eq!(breakdown.by_pool.values().sum::<i64>(), breakdown.waiting);
        assert_eq!(breakdown.oldest_waiting_millis, Some(70));
        // `gridops-ci` is missing whatever its case; `paused-only` exists on a
        // pool that doesn't serve it.
        assert_eq!(
            breakdown.missing_labels,
            vec![MissingLabels {
                labels: vec!["gridops-ci".into()],
                jobs: 2
            }]
        );
        assert_eq!(breakdown.unavailable, 1);
        assert_eq!(
            breakdown
                .missing_labels
                .iter()
                .map(|missing| missing.jobs)
                .sum::<i64>()
                + breakdown.unavailable,
            breakdown.unmatched
        );
    }

    #[test]
    fn alerts_name_the_missing_labels() {
        let breakdown = QueueBreakdown {
            unmatched: 7,
            missing_labels: vec![
                MissingLabels {
                    labels: vec!["gridops-ci".into()],
                    jobs: 4,
                },
                MissingLabels {
                    labels: vec!["gpu".into(), "big".into()],
                    jobs: 1,
                },
            ],
            unavailable: 2,
            ..QueueBreakdown::default()
        };
        let titles = breakdown
            .alerts()
            .iter()
            .map(|alert| alert["title"].as_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            titles,
            vec![
                "4 jobs need `gridops-ci`; no pool has it",
                "1 job needs `gpu`, `big`; no pool has them",
                "2 jobs have no available pool",
            ]
        );
    }

    #[test]
    fn listed_pool_rows_leave_the_rest_in_other_pools() {
        let breakdown = QueueBreakdown {
            waiting: 5,
            by_pool: HashMap::from([("a".into(), 2), ("b".into(), 3)]),
            ..QueueBreakdown::default()
        };
        assert_eq!(breakdown.served_outside(&["a".into()]), 3);
        assert_eq!(breakdown.served_outside(&["a".into(), "b".into()]), 0);
    }
}
