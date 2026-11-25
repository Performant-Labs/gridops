//! The windowed areas of concern (#192): pure rules over the scores, the
//! speed results and the window's job mix. The live concerns (offline,
//! start gate, disk, unmatched, oldest queued, uncertain) are #201's
//! `collect_live`. No I/O here, ever (`the_scoring_engine_does_no_io`).

use super::score::{duration, whole_percent};
use super::{
    Concern, ConcernSubject, FleetHealthInputs, HostInfo, HourlyJobRow, JobCategory,
    ScoreComponent, ScoredFleet, ScoredHost, Severity, SpeedResult,
};

/// A speed index at or above this makes a machine slow (`host-slow`).
const SLOW_INDEX: f64 = 1.3;
/// A busy share under this, while the fleet queues, is idleness
/// (`host-idle-while-queued`).
const IDLE_BUSY_SHARE: f64 = 0.30;
/// The fleet's queue-wait component under this means jobs are waiting.
const QUEUING: u8 = 70;
/// More than this share of the window's jobs in `other` is a concern.
const OTHER_SHARE: f64 = 0.20;

/// A score's band: critical under 50, warning under 70, none from 70 up.
fn band(score: u8) -> Option<Severity> {
    if score < 50 {
        Some(Severity::Critical)
    } else if score < 70 {
        Some(Severity::Warning)
    } else {
        None
    }
}

/// `fleet-component:<key>`: critical under 50, warning under 70, linking to
/// the component's row (`/fleet/health#fleet-component-<key>`).
#[must_use]
pub fn fleet_component(component: &ScoreComponent) -> Option<Concern> {
    let severity = band(component.score?)?;
    let detail = component.deduction.clone().unwrap_or_else(|| {
        format!(
            "{} scores {}.",
            component.label,
            component.score.unwrap_or_default()
        )
    });
    Some(Concern {
        key: format!("fleet-component:{}", component.key),
        severity,
        title: match component.key.as_str() {
            "queue-wait" => "Jobs wait too long to start".to_owned(),
            "start-reliability" => "Runner starts fail too often".to_owned(),
            "infra-failures" => "Jobs fail on the infrastructure".to_owned(),
            "capacity-fit" => "The fleet runs idle while jobs wait".to_owned(),
            "availability" => "Too much of the fleet is offline".to_owned(),
            _ => format!(
                "{} scores {}",
                component.label,
                component.score.unwrap_or_default()
            ),
        },
        detail,
        subject: ConcernSubject::Fleet,
        // The component's row on the fleet score card, not the page itself
        // (#253): `fleet-score-card.tsx` anchors each row by its key.
        link: Some(format!("/fleet/health#fleet-component-{}", component.key)),
        since: None,
    })
}

/// `host-score:<host>`: the same bands, naming the worst component.
#[must_use]
pub fn host_score(host: &ScoredHost) -> Option<Concern> {
    let score = host.score?;
    let severity = band(score)?;
    let (worst_score, worst) = host
        .components
        .iter()
        .filter_map(|component| component.score.map(|score| (score, component)))
        .min_by_key(|&(score, _)| score)?;
    let detail = match &worst.deduction {
        Some(deduction) => format!(
            "{}'s weakest part is {} ({}): {deduction}",
            host.name, worst.label, worst_score
        ),
        None => format!(
            "{}'s weakest part is {} ({}).",
            host.name, worst.label, worst_score
        ),
    };
    Some(Concern {
        key: format!("host-score:{}", host.host_id),
        severity,
        title: format!("{} scores {score}", host.name),
        detail,
        subject: ConcernSubject::Host(host.host_id.clone()),
        link: Some(format!("/fleet/health#machine-{}", host.host_id)),
        since: None,
    })
}

/// A CPU budget in millis as the sentences read it: `2`, `1.5`.
fn cpus(cpu_millis: u32) -> String {
    let value = f64::from(cpu_millis) / 1000.0;
    if (value - value.round()).abs() < 1e-9 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// `host-slow:<host>:<cpu millis>`: a speed index of 1.3 or more at one
/// runner size, naming the keys behind it.
#[must_use]
pub fn host_slow(host: &HostInfo, speed: &SpeedResult) -> Option<Concern> {
    let index = speed.index?;
    if index < SLOW_INDEX {
        return None;
    }
    let behind = speed
        .top_keys
        .first()
        .map(|top| {
            format!(
                "; most behind it: {} ({} against {})",
                top.key,
                duration(top.host_median_ms as f64),
                duration(top.fleet_median_ms as f64)
            )
        })
        .unwrap_or_default();
    let detail = format!(
        "{} takes {index:.2}\u{d7} as long as the fleet at {} CPUs{behind}.",
        host.name,
        cpus(speed.cpu_millis)
    );
    Some(Concern {
        key: format!("host-slow:{}:{}", host.id, speed.cpu_millis),
        severity: Severity::Warning,
        title: format!("{} is slow at {} CPUs", host.name, cpus(speed.cpu_millis)),
        detail,
        subject: ConcernSubject::Host(host.id.clone()),
        link: Some(format!("/fleet/health#machine-{}", host.id)),
        since: None,
    })
}

/// `host-idle-while-queued:<host>`: busy under 30% while the fleet's
/// queue-wait component is under 70.
#[must_use]
pub fn host_idle_while_queued(host: &ScoredHost, fleet_queue: Option<u8>) -> Option<Concern> {
    let busy = host.busy_share?;
    let queue = fleet_queue?;
    if busy >= IDLE_BUSY_SHARE || queue >= QUEUING {
        return None;
    }
    Some(Concern {
        key: format!("host-idle-while-queued:{}", host.host_id),
        severity: Severity::Warning,
        title: format!("{} is idle while jobs wait", host.name),
        detail: format!(
            "{} was busy {} of its CPU budget while the fleet's queue wait scored {queue}.",
            host.name,
            whole_percent(busy)
        ),
        subject: ConcernSubject::Host(host.host_id.clone()),
        link: Some(format!("/fleet/health#machine-{}", host.host_id)),
        since: None,
    })
}

/// `category-other`: more than 20% of the window's jobs are `other`.
#[must_use]
pub fn category_other(other_share: Option<f64>) -> Option<Concern> {
    let share = other_share?;
    if share <= OTHER_SHARE {
        return None;
    }
    Some(Concern {
        key: "category-other".to_owned(),
        severity: Severity::Info,
        title: "Many jobs have no category".to_owned(),
        detail: format!(
            "{} of the window's jobs are `other`: a job category rule may be missing.",
            whole_percent(share)
        ),
        subject: ConcernSubject::Fleet,
        link: Some("/settings/general".to_owned()),
        since: None,
    })
}

/// Every windowed concern, in rule order: the fleet's components, each
/// machine's score, its slow runner sizes, its idleness while jobs queue,
/// then the window's job mix.
#[must_use]
pub fn windowed(inputs: &FleetHealthInputs, scored: &ScoredFleet) -> Vec<Concern> {
    let mut concerns = Vec::new();
    for component in &scored.components {
        concerns.extend(fleet_component(component));
    }
    for host in &scored.hosts {
        concerns.extend(host_score(host));
    }
    for host in &inputs.hosts {
        for result in inputs
            .speed
            .iter()
            .filter(|result| result.host_id == host.id)
        {
            concerns.extend(host_slow(host, result));
        }
    }
    let fleet_queue = scored
        .components
        .iter()
        .find(|component| component.key == "queue-wait")
        .and_then(|component| component.score);
    for host in &scored.hosts {
        concerns.extend(host_idle_while_queued(host, fleet_queue));
    }
    let start = inputs.now - inputs.window.millis();
    let window: Vec<&HourlyJobRow> = inputs
        .hourly
        .iter()
        .filter(|row| row.hour >= start && row.hour < inputs.now)
        .collect();
    let total: u64 = window.iter().map(|row| row.job_count).sum();
    let other: u64 = window
        .iter()
        .filter(|row| row.category == JobCategory::Other)
        .map(|row| row.job_count)
        .sum();
    concerns.extend(category_other(
        (total > 0).then(|| other as f64 / total as f64),
    ));
    concerns
}

#[cfg(test)]
mod tests {
    use crate::fleet_health::{
        FleetLiveStats, HOUR_MILLIS, HealthTargets, HealthWindow, Histogram, HourlyJobRow,
        RollupHost, SpeedKey,
    };

    use super::*;

    fn component(key: &str, label: &str, score: Option<u8>) -> ScoreComponent {
        ScoreComponent {
            key: key.to_owned(),
            label: label.to_owned(),
            weight: 10.0,
            score,
            deduction: score.map(|score| format!("{label} lost points: \u{2212}{score}.")),
        }
    }

    fn host(id: &str, name: &str, score: Option<u8>, busy: Option<f64>) -> ScoredHost {
        ScoredHost {
            host_id: id.to_owned(),
            name: name.to_owned(),
            score,
            components: vec![component("start-reliability", "Start reliability", score)],
            busy_share: busy,
        }
    }

    fn info(id: &str, name: &str) -> HostInfo {
        HostInfo {
            id: id.to_owned(),
            name: name.to_owned(),
            cpu_budget_millis: 10_000,
            memory_budget_mib: 1_000,
            disk_total_bytes: None,
            created_at: None,
            retired_at: None,
        }
    }

    fn job_row(category: JobCategory, count: u64) -> HourlyJobRow {
        job_row_at(0, category, count)
    }

    fn job_row_at(hour: i64, category: JobCategory, count: u64) -> HourlyJobRow {
        HourlyJobRow {
            hour,
            host: RollupHost::Unmanaged,
            pool_id: "unmanaged".to_owned(),
            category,
            job_count: count,
            completed_count: count,
            infra_failure_count: 0,
            duration_ms_sum: 0,
            cpu_seconds_sum: 0.0,
            wait_histogram: Histogram::default(),
            duration_histogram: Histogram::default(),
        }
    }

    fn inputs(speed: Vec<SpeedResult>, hourly: Vec<HourlyJobRow>) -> FleetHealthInputs {
        FleetHealthInputs {
            window: HealthWindow::Day,
            now: HOUR_MILLIS,
            hosts: vec![info("h-1", "io")],
            hourly,
            host_hourly: Vec::new(),
            speed,
            live: FleetLiveStats::default(),
            host_live: Vec::new(),
            targets: HealthTargets::default(),
        }
    }

    #[test]
    fn windowed_runs_every_rule_in_order() {
        let speed = SpeedResult {
            host_id: "h-1".to_owned(),
            cpu_millis: 2_000,
            index: Some(1.5),
            top_keys: vec![SpeedKey {
                key: "acme/app / CI / e2e".to_owned(),
                host_median_ms: 870_000,
                fleet_median_ms: 600_000,
                host_runs: 14,
                fleet_runs: 98,
            }],
        };
        let inputs = inputs(
            vec![speed],
            vec![
                job_row(JobCategory::Other, 50),
                job_row(JobCategory::Test, 50),
                // Outside the window, at its end and before its start: their
                // 9,999 test jobs must not dilute the window's `other` share.
                job_row_at(HOUR_MILLIS, JobCategory::Test, 9_999),
                job_row_at(-24 * HOUR_MILLIS, JobCategory::Test, 9_999),
            ],
        );
        let scored = ScoredFleet {
            score: Some(40),
            components: vec![component("queue-wait", "Queue wait", Some(40))],
            hosts: vec![host("h-1", "io", Some(60), Some(0.1))],
        };
        let concerns = windowed(&inputs, &scored);
        let summary: Vec<(String, Severity)> = concerns
            .iter()
            .map(|concern| (concern.key.clone(), concern.severity))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("fleet-component:queue-wait".to_owned(), Severity::Critical),
                ("host-score:h-1".to_owned(), Severity::Warning),
                ("host-slow:h-1:2000".to_owned(), Severity::Warning),
                ("host-idle-while-queued:h-1".to_owned(), Severity::Warning),
                ("category-other".to_owned(), Severity::Info),
            ]
        );
        assert!(
            concerns[0].detail.starts_with("Queue wait lost points"),
            "the fleet component's detail is its deduction: {}",
            concerns[0].detail
        );
        assert!(
            concerns[1]
                .detail
                .contains("Start reliability (60): Start reliability lost points"),
            "the host concern names its worst component: {}",
            concerns[1].detail
        );
        assert!(
            concerns[2]
                .detail
                .contains("acme/app / CI / e2e (14 min 30 s against 10 min)"),
            "the slow concern names the slowest job key: {}",
            concerns[2].detail
        );
        assert_eq!(concerns[2].title, "io is slow at 2 CPUs");
        assert!(
            concerns[3]
                .detail
                .contains("busy 10% of its CPU budget while the fleet's queue wait scored 40"),
            "{}",
            concerns[3].detail
        );
        assert!(
            concerns[4]
                .detail
                .starts_with("50% of the window's jobs are `other`"),
            "{}",
            concerns[4].detail
        );
    }

    #[test]
    fn the_idle_rule_reads_the_queue_component_not_the_fleet_score() {
        // The fleet scores 80 while its queue-wait component scores 60: the
        // idle rule must still fire.
        let inputs = inputs(Vec::new(), vec![job_row(JobCategory::Test, 40)]);
        let scored = ScoredFleet {
            score: Some(80),
            components: vec![
                component("queue-wait", "Queue wait", Some(60)),
                component("availability", "Availability", Some(100)),
            ],
            hosts: vec![host("h-1", "io", Some(80), Some(0.1))],
        };
        assert!(
            windowed(&inputs, &scored)
                .iter()
                .any(|concern| concern.key == "host-idle-while-queued:h-1")
        );
    }

    #[test]
    fn nothing_without_data_raises_a_concern() {
        let inputs = inputs(Vec::new(), vec![job_row(JobCategory::Test, 40)]);
        let scored = ScoredFleet {
            score: None,
            components: vec![component("queue-wait", "Queue wait", None)],
            hosts: vec![host("h-1", "io", None, None)],
        };
        assert_eq!(windowed(&inputs, &scored), Vec::new());
    }

    #[test]
    fn slow_runner_sizes_format_their_cpu_key() {
        let info = info("h-1", "io");
        let at = |cpu_millis: u32| SpeedResult {
            host_id: "h-1".to_owned(),
            cpu_millis,
            index: Some(1.42),
            top_keys: Vec::new(),
        };
        let result = at(1_500);
        assert_eq!(
            host_slow(&info, &result).map(|concern| concern.key),
            Some("host-slow:h-1:1500".to_owned())
        );
        assert_eq!(
            host_slow(&info, &result).map(|concern| concern.title),
            Some("io is slow at 1.5 CPUs".to_owned())
        );
    }

    /// #253 item 7: a fleet component's concern links to that component's
    /// row on the fleet score card (`#fleet-component-<key>`), not to the
    /// page it is already on; a machine's concerns keep their machine's row.
    #[test]
    fn a_fleet_component_concern_links_to_its_component_row() {
        for key in [
            "queue-wait",
            "start-reliability",
            "infra-failures",
            "capacity-fit",
            "availability",
        ] {
            let concern = fleet_component(&component(key, "Label", Some(10)));
            assert_eq!(
                concern.and_then(|concern| concern.link),
                Some(format!("/fleet/health#fleet-component-{key}")),
                "{key}"
            );
        }
        let inputs = inputs(Vec::new(), vec![job_row(JobCategory::Test, 40)]);
        let scored = ScoredFleet {
            score: Some(30),
            components: vec![component("queue-wait", "Queue wait", Some(30))],
            hosts: vec![host("h-1", "io", Some(30), None)],
        };
        for concern in windowed(&inputs, &scored) {
            let link = concern.link.unwrap_or_default();
            match concern.subject {
                ConcernSubject::Fleet => {
                    assert!(link.starts_with("/fleet/health#fleet-component-"), "{link}");
                }
                ConcernSubject::Host(id) => {
                    assert_eq!(link, format!("/fleet/health#machine-{id}"));
                }
                ConcernSubject::Pool(_) => {}
            }
        }
    }
}
