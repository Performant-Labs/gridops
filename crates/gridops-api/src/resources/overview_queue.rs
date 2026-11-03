//! How the Overview splits the queue: every job waiting to start is waiting
//! for capacity (an autoscaling pool will start or free a runner for it), on a
//! manual pool (only a manual-mode pool can run it, and nothing starts a
//! runner for it), or has no matching pool (none will run it). The three
//! always add up to the Queued tile, and the pool rows add up to the first two.
//!
//! Which pool a job is under is decided once, by the same rule the reconciler
//! scales from and a run's page uses ([`waiting_job_pools`]); nothing here
//! matches labels itself.

use std::collections::{BTreeMap, HashMap, HashSet};

use gridops_core::{
    LINUX_RUNNER_LABEL, MACOS_RUNNER_LABEL, WaitingJobOwner, is_runner_system_label,
    pool_platform_labels, waiting_job_pools,
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

/// A pool the viewer can see, with the labels a job can ask it for.
#[derive(Debug, Clone)]
pub(super) struct VisiblePool {
    /// Labels it carries: its configured labels and its name.
    pub labels: Vec<String>,
    /// Platform labels its platforms offer (OS, architecture,
    /// `self-hosted-linux`, `self-hosted-macos`); see
    /// [`gridops_core::pool_platform_labels`].
    pub platform_labels: Vec<String>,
}

/// Every label some pool the viewer can see offers, split the way the scaler
/// matches labels: a platform label (`linux`, `macos`, `windows`, `x64`,
/// `arm64`, `arm`, `x86`, `self-hosted-linux`, `self-hosted-macos`) is offered
/// only by a pool running that platform, never by a configured label, and any
/// other label only by a pool carrying it.
#[derive(Debug, Default)]
pub(super) struct OfferedLabels {
    carried: HashSet<String>,
    platform: HashSet<String>,
}

impl OfferedLabels {
    pub fn new(pools: &[VisiblePool]) -> Self {
        let lower = |labels: &[String]| {
            labels
                .iter()
                .map(|label| label.to_lowercase())
                .collect::<Vec<_>>()
        };
        Self {
            carried: pools.iter().flat_map(|pool| lower(&pool.labels)).collect(),
            platform: pools
                .iter()
                .flat_map(|pool| lower(&pool.platform_labels))
                .collect(),
        }
    }

    /// The labels of `requested` no pool offers, as the job spelled them,
    /// sorted and without repeats, ignoring case. `self-hosted` is never
    /// missing. Empty when each label is offered by some pool, even if no
    /// single pool offers them all.
    pub fn missing(&self, requested: &[String]) -> Vec<String> {
        let mut missing = requested
            .iter()
            .filter(|label| {
                let lower = label.to_lowercase();
                if lower == "self-hosted" {
                    false
                } else if is_runner_system_label(&lower)
                    || lower == LINUX_RUNNER_LABEL
                    || lower == MACOS_RUNNER_LABEL
                {
                    !self.platform.contains(&lower)
                } else {
                    !self.carried.contains(&lower)
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        missing.sort_by_key(|label| label.to_lowercase());
        missing.dedup_by_key(|label| label.to_lowercase());
        missing
    }
}

/// Jobs that ask for the same labels no pool has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MissingLabels {
    pub labels: Vec<String>,
    pub jobs: i64,
}

#[derive(Debug, Default)]
pub(super) struct QueueBreakdown {
    /// Jobs an autoscaling pool will start or free a runner for.
    pub waiting: i64,
    /// Jobs only a manual-mode pool can run: nothing starts a runner for them.
    pub manual: i64,
    pub unmatched: i64,
    /// Waiting and manual jobs by the pool they are under.
    pub by_pool: HashMap<String, i64>,
    pub oldest_waiting_millis: Option<i64>,
    /// The job that has waited `oldest_waiting_millis`.
    pub oldest_waiting_job: Option<i64>,
    /// Unmatched jobs asking for labels no pool has, most jobs first.
    pub missing_labels: Vec<MissingLabels>,
    /// Unmatched jobs that ask for no missing label: every pool that could run
    /// them is paused, tripped or on another platform.
    pub unavailable: i64,
}

impl QueueBreakdown {
    pub fn total(&self) -> i64 {
        self.waiting + self.manual + self.unmatched
    }

    /// Waiting and manual jobs under pools other than the listed ones.
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
                "detail": "Every pool that could run them is paused, has an open provisioning circuit or runs another platform.",
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

/// Splits `jobs` by whether a pool serves them. `serving` says which
/// autoscaling pool serves each job and `manual` which manual-mode pool a job
/// no autoscaling pool serves is under; a job in neither has no matching
/// pool. A job under a pool the viewer can't see counts all the same (the
/// pool exists; it is just not one of the viewer's rows), so it is in
/// [`QueueBreakdown::served_outside`]. `pools` are the pools the viewer can
/// see, whose labels tell which labels exist.
pub(super) fn split_queue(
    jobs: &[WaitingJob],
    serving: &HashMap<i64, String>,
    manual: &HashMap<i64, String>,
    pools: &[VisiblePool],
    now: i64,
) -> QueueBreakdown {
    let offered = OfferedLabels::new(pools);
    let mut breakdown = QueueBreakdown::default();
    let mut missing = BTreeMap::<Vec<String>, (Vec<String>, i64)>::new();
    for job in jobs {
        if let Some(pool) = serving.get(&job.id) {
            breakdown.waiting += 1;
            *breakdown.by_pool.entry(pool.clone()).or_default() += 1;
            let age = now.saturating_sub(job.created_at);
            if breakdown.oldest_waiting_millis < Some(age) {
                breakdown.oldest_waiting_millis = Some(age);
                breakdown.oldest_waiting_job = Some(job.id);
            }
            continue;
        }
        if let Some(pool) = manual.get(&job.id) {
            breakdown.manual += 1;
            *breakdown.by_pool.entry(pool.clone()).or_default() += 1;
            continue;
        }
        breakdown.unmatched += 1;
        let labels = offered.missing(&job.labels);
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

/// The pools `user_id` may see (every installation a pool covers is one of
/// theirs), with the labels each offers.
pub(super) async fn visible_pools(state: &AppState, user_id: &str) -> ApiResult<Vec<VisiblePool>> {
    let pool_rows = sqlx::query(
        r#"SELECT p.id,p.name,p.labels FROM runner_pools p
          WHERE EXISTS (SELECT 1 FROM runner_pool_installations mapped WHERE mapped.pool_id=p.id)
            AND NOT EXISTS (
              SELECT 1 FROM runner_pool_installations mapped WHERE mapped.pool_id=p.id
                AND NOT EXISTS (SELECT 1 FROM user_installations access
                  WHERE access.user_id=? AND access.installation_id=mapped.installation_id)
            )"#,
    )
    .bind(user_id)
    .fetch_all(&state.database)
    .await?;
    let mut platforms = if pool_rows.is_empty() {
        HashMap::new()
    } else {
        pool_platform_labels(&state.database).await?
    };
    Ok(pool_rows
        .iter()
        .map(|row| {
            // A pool's name is a label of it too.
            let mut labels = json_array(row.get::<&str, _>("labels"));
            labels.push(row.get::<String, _>("name"));
            VisiblePool {
                labels,
                platform_labels: platforms
                    .remove(&row.get::<String, _>("id"))
                    .unwrap_or_default(),
            }
        })
        .collect())
}

/// The queue as `user` may see it: the jobs of their installations waiting to
/// start in a run that hasn't finished, split by whether a pool serves them.
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
    let pools = visible_pools(state, &user.id).await?;
    let mut serving = HashMap::new();
    let mut manual = HashMap::new();
    if !jobs.is_empty() {
        for owner in waiting_job_pools(&state.database)
            .await?
            .into_iter()
            .filter_map(|job| job.owner)
        {
            match owner {
                WaitingJobOwner::Scaling(owner) => serving.insert(owner.job_id, owner.pool_id),
                WaitingJobOwner::Manual { job_id, pool_id } => manual.insert(job_id, pool_id),
            };
        }
    }
    Ok(split_queue(&jobs, &serving, &manual, &pools, now))
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

    /// A pool carrying `labels` that runs Linux on x64, as a Docker pool on
    /// an x64 host does.
    fn pool(labels: &[&str]) -> VisiblePool {
        platform_pool(labels, &["linux", "x64", "self-hosted-linux"])
    }

    fn platform_pool(labels: &[&str], platform_labels: &[&str]) -> VisiblePool {
        VisiblePool {
            labels: labels.iter().map(|label| (*label).to_owned()).collect(),
            platform_labels: platform_labels
                .iter()
                .map(|label| (*label).to_owned())
                .collect(),
        }
    }

    #[test]
    fn a_platform_no_pool_runs_is_a_missing_label() {
        // Issue #32: with only Linux pools, a macOS or Windows job can never
        // run, and the label it is missing is named.
        let jobs = [
            job(1, &["self-hosted", "self-hosted-macos"], 10),
            job(2, &["self-hosted", "macOS", "ARM64"], 20),
            job(3, &["self-hosted", "Windows", "gpu"], 30),
            // Each label exists on some pool, just not together: no pool is
            // free for it, but nothing is missing.
            job(4, &["self-hosted", "linux", "x64", "mac-only"], 40),
        ];
        // A configured `macos` label does not make a pool run macOS.
        let pools = [
            pool(&["linux-pool", "macos"]),
            platform_pool(&["mac-only"], &[]),
        ];

        let breakdown = split_queue(&jobs, &HashMap::new(), &HashMap::new(), &pools, 100);

        assert_eq!(breakdown.unmatched, 4);
        assert_eq!(
            breakdown.missing_labels,
            vec![
                MissingLabels {
                    labels: vec!["ARM64".into(), "macOS".into()],
                    jobs: 1
                },
                MissingLabels {
                    labels: vec!["gpu".into(), "Windows".into()],
                    jobs: 1
                },
                MissingLabels {
                    labels: vec!["self-hosted-macos".into()],
                    jobs: 1
                },
            ]
        );
        assert_eq!(breakdown.unavailable, 1);

        // Once a macOS pool exists, the macOS jobs are missing nothing.
        let offered =
            OfferedLabels::new(&[platform_pool(&[], &["macos", "arm64", "self-hosted-macos"])]);
        assert!(offered.missing(&jobs[0].labels).is_empty());
        assert!(offered.missing(&jobs[1].labels).is_empty());
        assert_eq!(offered.missing(&jobs[2].labels), vec!["gpu", "Windows"]);
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

        let breakdown = split_queue(&jobs, &serving, &HashMap::new(), &pools, 100);

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
    fn a_job_only_a_manual_pool_can_run_is_its_own_part_and_in_that_pools_row() {
        let jobs = [
            job(1, &["fast"], 10),
            job(2, &["manual-only"], 20),
            job(3, &["manual-only"], 30),
            job(4, &["nowhere"], 40),
        ];
        let serving = HashMap::from([(1, "auto".to_owned())]);
        let manual = HashMap::from([(2, "fixed".to_owned()), (3, "hidden".to_owned())]);
        let pools = [pool(&["fast"]), pool(&["manual-only"])];

        let breakdown = split_queue(&jobs, &serving, &manual, &pools, 100);

        assert_eq!(
            (breakdown.waiting, breakdown.manual, breakdown.unmatched),
            (1, 2, 1)
        );
        assert_eq!(breakdown.total(), 4);
        assert_eq!(breakdown.by_pool.get("fixed"), Some(&1));
        assert_eq!(
            breakdown.served_outside(&["auto".into(), "fixed".into()]),
            1
        );
        assert_eq!(
            breakdown.by_pool.values().sum::<i64>(),
            breakdown.waiting + breakdown.manual
        );
        // Manual jobs are not waiting for capacity, so they don't age the queue.
        assert_eq!(breakdown.oldest_waiting_millis, Some(90));
        // The manual pool's label exists, so nothing is reported missing.
        assert_eq!(breakdown.missing_labels.len(), 1);
        assert_eq!(breakdown.unavailable, 0);
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
