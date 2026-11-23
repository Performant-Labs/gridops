//! Contract tests for #192 (scores and windowed concerns), written by #197.
//!
//! #192 removes the `#[ignore = "contract: #192"]` markers and makes these
//! pass; it changes nothing else here (`Fleet health guard`). The weights,
//! targets and thresholds are #192's two score tables and concern table, as
//! `docs/design/fleet-health.md` records them:
//! - linear between the "100 at" and "0 at" values, clamped to 0..=100;
//! - a component with too little data has `score: None`, a "not enough data"
//!   reason, weight 0, and its weight spread over the others in proportion;
//! - a deduction sentence ends with the points lost, `: −N.`, where N is
//!   `round(weight × (100 − score) / 100)`; a component at 100 has none.
//!
//! Mounted from `fleet_health/mod.rs` (#197-owned).

use super::{
    DiskHeadroom, HISTOGRAM_BUCKETS, HealthTargets, HostInfo, ScoreComponent, ScoredHost, Severity,
    SpeedKey, SpeedResult, concerns, fake, score,
};

const SECOND: f64 = 1_000.0;
const MINUTE: f64 = 60.0 * SECOND;
const GIB: u64 = 1024 * 1024 * 1024;

fn targets() -> HealthTargets {
    HealthTargets::default()
}

/// (input, expected score) rows: at 100, at 0, the midpoint, beyond both ends.
fn check_table(
    name: &str,
    rows: &[(Option<f64>, Option<u8>)],
    component: impl Fn(Option<f64>) -> ScoreComponent,
) {
    for (input, expected) in rows {
        let got = component(*input);
        assert_eq!(got.score, *expected, "{name}({input:?})");
        match got.score {
            Some(100) => assert_eq!(got.deduction, None, "{name}({input:?}) at 100"),
            Some(_) => assert!(
                got.deduction
                    .as_deref()
                    .is_some_and(|sentence| sentence.contains('−') && sentence.ends_with('.')),
                "{name}({input:?}) needs a deduction sentence: {:?}",
                got.deduction
            ),
            None => assert!(
                got.deduction
                    .as_deref()
                    .is_some_and(|reason| reason.to_lowercase().contains("not enough data")),
                "{name}({input:?}) must say not enough data: {:?}",
                got.deduction
            ),
        }
    }
}

fn component(key: &str, weight: f64, score: Option<u8>) -> ScoreComponent {
    ScoreComponent {
        key: key.to_owned(),
        label: key.to_owned(),
        weight,
        score,
        deduction: None,
    }
}

fn host(id: &str, score: Option<u8>, components: Vec<ScoreComponent>) -> ScoredHost {
    ScoredHost {
        host_id: id.to_owned(),
        name: id.to_uppercase(),
        score,
        components,
        busy_share: None,
    }
}

#[test]
#[ignore = "contract: #192"]
fn fleet_components_have_their_keys_labels_and_weights() {
    let t = targets();
    let fleet = [
        score::fleet_queue_wait(Some(MINUTE), &t),
        score::fleet_start_reliability(Some(1.0), &t),
        score::fleet_infra_failures(Some(0.0), &t),
        score::fleet_capacity_fit(Some(0.0), &t),
        score::fleet_availability(Some(1.0), &t),
    ];
    let shape: Vec<(&str, &str, f64)> = fleet
        .iter()
        .map(|c| (c.key.as_str(), c.label.as_str(), c.weight))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("queue-wait", "Queue wait", 40.0),
            ("start-reliability", "Start reliability", 20.0),
            ("infra-failures", "Infrastructure failures", 15.0),
            ("capacity-fit", "Capacity fit", 15.0),
            ("availability", "Availability", 10.0),
        ]
    );
    let disk = DiskHeadroom {
        total_bytes: 100 * GIB,
        min_free_bytes: 50 * GIB,
    };
    let machine = [
        score::host_availability(Some(1.0), &t),
        score::host_start_reliability(Some(1.0), &t),
        score::host_relative_speed(Some(1.0), &t),
        score::host_headroom(Some(disk), Some(0.5), &t),
        score::host_busy_share(Some(0.6), Some(80), &t),
    ];
    let shape: Vec<(&str, &str, f64)> = machine
        .iter()
        .map(|c| (c.key.as_str(), c.label.as_str(), c.weight))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("availability", "Availability", 25.0),
            ("start-reliability", "Start reliability", 25.0),
            ("relative-speed", "Relative speed", 25.0),
            ("headroom", "Headroom", 15.0),
            ("busy-share", "Busy share", 10.0),
        ]
    );
    assert!(
        fleet
            .iter()
            .chain(machine.iter())
            .all(|c| c.score == Some(100))
    );
}

#[test]
#[ignore = "contract: #192"]
fn each_fleet_component_at_100_at_0_at_the_midpoint_and_beyond_both_ends() {
    let t = targets();
    check_table(
        "queue wait p90 (ms)",
        &[
            (Some(MINUTE), Some(100)),
            (Some(10.0 * MINUTE), Some(0)),
            (Some(5.5 * MINUTE), Some(50)),
            (Some(30.0 * SECOND), Some(100)),
            (Some(20.0 * MINUTE), Some(0)),
            (None, None),
        ],
        |value| score::fleet_queue_wait(value, &t),
    );
    check_table(
        "start reliability (share of starts that succeeded)",
        &[
            (Some(0.99), Some(100)),
            (Some(0.80), Some(0)),
            (Some(0.895), Some(50)),
            (Some(1.0), Some(100)),
            (Some(0.5), Some(0)),
            (None, None),
        ],
        |value| score::fleet_start_reliability(value, &t),
    );
    check_table(
        "infrastructure failures (share of completed jobs)",
        &[
            (Some(0.0), Some(100)),
            (Some(0.05), Some(0)),
            (Some(0.025), Some(50)),
            (Some(-0.01), Some(100)),
            (Some(0.10), Some(0)),
            (None, None),
        ],
        |value| score::fleet_infra_failures(value, &t),
    );
    check_table(
        "capacity fit (share of hours waiting with idle CPU)",
        &[
            (Some(0.0), Some(100)),
            (Some(0.25), Some(0)),
            (Some(0.125), Some(50)),
            (Some(-0.1), Some(100)),
            (Some(0.5), Some(0)),
            (None, None),
        ],
        |value| score::fleet_capacity_fit(value, &t),
    );
    check_table(
        "fleet availability (budget-weighted online share)",
        &[
            (Some(1.0), Some(100)),
            (Some(0.80), Some(0)),
            (Some(0.90), Some(50)),
            (Some(1.1), Some(100)),
            (Some(0.5), Some(0)),
            (None, None),
        ],
        |value| score::fleet_availability(value, &t),
    );
}

#[test]
#[ignore = "contract: #192"]
fn each_machine_component_at_100_at_0_at_the_midpoint_and_beyond_both_ends() {
    let t = targets();
    check_table(
        "machine availability",
        &[
            (Some(1.0), Some(100)),
            (Some(0.80), Some(0)),
            (Some(0.90), Some(50)),
            (Some(1.2), Some(100)),
            (Some(0.0), Some(0)),
            (None, None),
        ],
        |value| score::host_availability(value, &t),
    );
    check_table(
        "machine start reliability",
        &[
            (Some(0.99), Some(100)),
            (Some(0.80), Some(0)),
            (Some(0.895), Some(50)),
            (Some(1.0), Some(100)),
            (Some(0.1), Some(0)),
            (None, None),
        ],
        |value| score::host_start_reliability(value, &t),
    );
    check_table(
        "relative speed (speed index)",
        &[
            (Some(1.0), Some(100)),
            (Some(2.0), Some(0)),
            (Some(1.5), Some(50)),
            (Some(0.7), Some(100)),
            (Some(3.0), Some(0)),
            (None, None),
        ],
        |value| score::host_relative_speed(value, &t),
    );
    // Busy share deducts only while the fleet queue component is under 100.
    check_table(
        "busy share, fleet queue at 80",
        &[
            (Some(0.5), Some(100)),
            (Some(0.0), Some(0)),
            (Some(0.25), Some(50)),
            (Some(0.9), Some(100)),
            (Some(-0.1), Some(0)),
            (None, None),
        ],
        |value| score::host_busy_share(value, Some(80), &t),
    );
    for share in [0.0, 0.25, 0.5] {
        assert_eq!(
            score::host_busy_share(Some(share), Some(100), &t).score,
            Some(100),
            "a fleet queue at 100 never deducts busy share ({share})"
        );
    }
    // Headroom: the lower of disk (100 at the warning floor, 0 at the
    // critical floor, as `disk::alarm_for` sets them: 10 and 5 GiB of a
    // 100 GiB disk) and memory (100 at 80% used, 0 at 100%).
    let disk = |free: f64| {
        Some(DiskHeadroom {
            total_bytes: 100 * GIB,
            min_free_bytes: (free * GIB as f64) as u64,
        })
    };
    for (free_gib, memory, expected) in [
        (Some(20.0), Some(0.5), Some(100)),
        (Some(10.0), Some(0.8), Some(100)),
        (Some(5.0), Some(0.5), Some(0)),
        (Some(20.0), Some(1.0), Some(0)),
        (Some(7.5), Some(0.5), Some(50)),
        (Some(20.0), Some(0.9), Some(50)),
        (Some(1.0), Some(0.5), Some(0)),
        (Some(20.0), Some(1.3), Some(0)),
        (Some(7.5), Some(0.85), Some(50)),
        (Some(20.0), Some(0.85), Some(75)),
        (None, Some(0.9), Some(50)),
        (Some(7.5), None, Some(50)),
        (None, None, None),
    ] {
        let got = score::host_headroom(free_gib.and_then(disk), memory, &t);
        assert_eq!(
            got.score, expected,
            "headroom(disk {free_gib:?} GiB, memory {memory:?})"
        );
    }
}

#[test]
#[ignore = "contract: #192"]
fn deduction_sentences_name_the_value_the_target_and_the_points_lost() {
    let t = targets();
    // #192's own example: 7 min 12 s scores 31.1; 40 × 0.689 = 27.6 points.
    let queue = score::fleet_queue_wait(Some(7.0 * MINUTE + 12.0 * SECOND), &t);
    assert_eq!(queue.score, Some(31));
    assert_eq!(
        queue.deduction.as_deref(),
        Some("Queue p90 was 7 min 12 s against a 1 min target: −28.")
    );
    for (component, lost) in [
        (score::fleet_queue_wait(Some(5.5 * MINUTE), &t), 20),
        (score::fleet_start_reliability(Some(0.895), &t), 10),
        (score::fleet_availability(Some(0.90), &t), 5),
        (score::host_busy_share(Some(0.25), Some(80), &t), 5),
    ] {
        let sentence = component.deduction.unwrap_or_default();
        assert!(
            sentence.ends_with(&format!(": −{lost}.")),
            "{}: {sentence}",
            component.key
        );
    }
}

#[test]
#[ignore = "contract: #192"]
fn a_component_without_enough_data_spreads_its_weight_and_the_weights_still_sum_to_100() {
    let all = [
        component("queue-wait", 40.0, Some(100)),
        component("start-reliability", 20.0, Some(50)),
        component("infra-failures", 15.0, Some(0)),
        component("capacity-fit", 15.0, Some(100)),
        component("availability", 10.0, Some(100)),
    ];
    let combined = score::combine(&all);
    assert_eq!(combined.score, Some(75), "40 + 10 + 0 + 15 + 10");
    let weights: Vec<f64> = combined.components.iter().map(|c| c.weight).collect();
    assert_eq!(weights, [40.0, 20.0, 15.0, 15.0, 10.0]);

    let mut missing = all.clone();
    missing[3] = component("capacity-fit", 15.0, None);
    let combined = score::combine(&missing);
    let sum: f64 = combined.components.iter().map(|c| c.weight).sum();
    assert!((sum - 100.0).abs() < 1e-6, "weights sum to {sum}");
    assert!(
        combined.components[3].weight.abs() < 1e-9,
        "a missing component takes no weight"
    );
    for (index, original) in [(0, 40.0), (1, 20.0), (2, 15.0), (4, 10.0)] {
        let spread = combined.components[index].weight;
        assert!(
            (spread - original * 100.0 / 85.0).abs() < 1e-6,
            "component {index}: {spread}"
        );
    }
    // (100 × 40 + 50 × 20 + 0 × 15 + 100 × 10) / 85 = 70.6.
    assert_eq!(combined.score, Some(71));
    assert_eq!(
        combined
            .components
            .iter()
            .map(|c| c.key.as_str())
            .collect::<Vec<_>>(),
        [
            "queue-wait",
            "start-reliability",
            "infra-failures",
            "capacity-fit",
            "availability"
        ],
        "the order is kept"
    );

    let none: Vec<ScoreComponent> = all
        .iter()
        .map(|c| ScoreComponent {
            score: None,
            ..c.clone()
        })
        .collect();
    assert_eq!(score::combine(&none).score, None);
}

#[test]
#[ignore = "contract: #192"]
fn histogram_percentiles_interpolate_inside_the_crossing_bucket() {
    let mut single = [0_u64; HISTOGRAM_BUCKETS];
    single[3] = 10; // all ten jobs waited 1 to 2 minutes
    assert_eq!(score::percentile(&single, 0.9), Some(114.0 * SECOND));
    assert_eq!(score::percentile(&single, 0.5), Some(90.0 * SECOND));

    let uniform = [1_u64; HISTOGRAM_BUCKETS];
    // Target 4.5 of 9: halfway through the 2–5 min bucket.
    assert_eq!(score::percentile(&uniform, 0.5), Some(210.0 * SECOND));
    // Target 8.1 of 9: inside 40+, which counts as 40 min.
    assert_eq!(score::percentile(&uniform, 0.9), Some(40.0 * MINUTE));

    let mut slow = [0_u64; HISTOGRAM_BUCKETS];
    slow[8] = 7;
    assert_eq!(score::percentile(&slow, 0.9), Some(40.0 * MINUTE));
    assert_eq!(score::percentile(&slow, 0.1), Some(40.0 * MINUTE));

    assert_eq!(score::percentile(&[0; HISTOGRAM_BUCKETS], 0.9), None);
}

#[test]
#[ignore = "contract: #192"]
fn on_the_fixture_the_weak_machines_score_lowest_and_its_concerns_come_out() {
    let inputs = fake::fixture_inputs();
    let scored = score::score_fleet(&inputs);
    let lowest = |key: &str| {
        scored
            .hosts
            .iter()
            .filter_map(|host| {
                host.components
                    .iter()
                    .find(|c| c.key == key)
                    .and_then(|c| c.score)
                    .map(|score| (score, host.host_id.as_str()))
            })
            .min()
            .map(|(_, id)| id)
    };
    assert_eq!(lowest("relative-speed"), Some(fake::SLOW_HOST));
    assert_eq!(lowest("start-reliability"), Some(fake::FAILING_STARTS_HOST));

    let windowed_prefixes = [
        "fleet-component:",
        "host-score:",
        "host-slow:",
        "host-idle-while-queued:",
        "category-other",
    ];
    let mut expected: Vec<(String, Severity)> = fake::fixture(super::HealthWindow::Day)
        .concerns
        .into_iter()
        .filter(|c| {
            windowed_prefixes
                .iter()
                .any(|prefix| c.key.starts_with(prefix))
        })
        .map(|c| (c.key, c.severity))
        .collect();
    let mut got: Vec<(String, Severity)> = concerns::windowed(&inputs, &scored)
        .into_iter()
        .map(|c| (c.key, c.severity))
        .collect();
    expected.sort();
    got.sort();
    assert!(!expected.is_empty(), "the fixture has windowed concerns");
    assert_eq!(got, expected);
}

#[test]
#[ignore = "contract: #192"]
fn each_concern_rule_fires_at_its_threshold_and_not_just_below_it() {
    // fleet-component:<key>: critical under 50, warning under 70.
    for (score, expected) in [
        (Some(49), Some(Severity::Critical)),
        (Some(50), Some(Severity::Warning)),
        (Some(69), Some(Severity::Warning)),
        (Some(70), None),
        (None, None),
    ] {
        let got = concerns::fleet_component(&component("capacity-fit", 15.0, score));
        assert_eq!(
            got.as_ref().map(|c| c.severity),
            expected,
            "fleet {score:?}"
        );
        if let Some(concern) = got {
            assert_eq!(concern.key, "fleet-component:capacity-fit");
            assert_eq!(concern.subject, super::ConcernSubject::Fleet);
        }
    }

    // host-score:<host>: the same bands, naming the worst component.
    let parts = vec![
        component("availability", 25.0, Some(90)),
        ScoreComponent {
            label: "Start reliability".to_owned(),
            ..component("start-reliability", 25.0, Some(10))
        },
        component("relative-speed", 25.0, None),
    ];
    for (score, expected) in [
        (Some(49), Some(Severity::Critical)),
        (Some(50), Some(Severity::Warning)),
        (Some(69), Some(Severity::Warning)),
        (Some(70), None),
        (None, None),
    ] {
        let got = concerns::host_score(&host("h-1", score, parts.clone()));
        assert_eq!(got.as_ref().map(|c| c.severity), expected, "host {score:?}");
        if let Some(concern) = got {
            assert_eq!(concern.key, "host-score:h-1");
            assert_eq!(
                concern.subject,
                super::ConcernSubject::Host("h-1".to_owned())
            );
            assert!(
                concern.detail.contains("Start reliability"),
                "names the worst component: {}",
                concern.detail
            );
        }
    }

    // host-slow:<host>:<cpu>: a speed index of 1.3 or more at one size.
    let info = HostInfo {
        id: "h-1".to_owned(),
        name: "io".to_owned(),
        cpu_budget_millis: 22_800,
        memory_budget_mib: 65_536,
        disk_total_bytes: Some(500 * GIB),
    };
    let speed = |index: Option<f64>| SpeedResult {
        host_id: "h-1".to_owned(),
        cpu_millis: 1_500,
        index,
        top_keys: vec![SpeedKey {
            key: "acme/app / CI / e2e".to_owned(),
            host_median_ms: 300_000,
            fleet_median_ms: 200_000,
            host_runs: 12,
            fleet_runs: 40,
        }],
    };
    for (index, fires) in [
        (Some(1.3), true),
        (Some(1.299), false),
        (Some(2.5), true),
        (None, false),
    ] {
        let got = concerns::host_slow(&info, &speed(index));
        assert_eq!(got.is_some(), fires, "speed {index:?}");
        if let Some(concern) = got {
            assert_eq!(concern.key, "host-slow:h-1:1500");
            assert_eq!(concern.severity, Severity::Warning);
            assert!(
                concern.detail.contains("acme/app / CI / e2e"),
                "{}",
                concern.detail
            );
        }
    }

    // host-idle-while-queued:<host>: busy under 30% while the fleet queue
    // component is under 70.
    for (busy, queue, fires) in [
        (Some(0.299), Some(69), true),
        (Some(0.30), Some(69), false),
        (Some(0.10), Some(70), false),
        (Some(0.10), None, false),
        (None, Some(10), false),
    ] {
        let mut idle = host("h-1", Some(80), Vec::new());
        idle.busy_share = busy;
        let got = concerns::host_idle_while_queued(&idle, queue);
        assert_eq!(got.is_some(), fires, "busy {busy:?}, queue {queue:?}");
        if let Some(concern) = got {
            assert_eq!(concern.key, "host-idle-while-queued:h-1");
            assert_eq!(concern.severity, Severity::Warning);
        }
    }

    // category-other: more than 20% of the window's jobs.
    for (share, fires) in [
        (Some(0.2001), true),
        (Some(0.20), false),
        (Some(0.9), true),
        (None, false),
    ] {
        let got = concerns::category_other(share);
        assert_eq!(got.is_some(), fires, "other share {share:?}");
        if let Some(concern) = got {
            assert_eq!(concern.key, "category-other");
            assert_eq!(concern.severity, Severity::Info);
        }
    }
}

/// `SplitMix64`: deterministic inputs without a dependency (`proptest` is not
/// in `Cargo.lock`, so #192 takes the 1,000-seeded-inputs branch).
struct Inputs(u64);

impl Inputs {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `[low, high)`, or `None` one time in ten.
    fn value(&mut self, low: f64, high: f64) -> Option<f64> {
        let raw = self.next();
        (!raw.is_multiple_of(10))
            .then(|| low + (raw >> 11) as f64 / (1_u64 << 53) as f64 * (high - low))
    }
}

#[test]
#[ignore = "contract: #192"]
fn no_score_is_ever_outside_0_to_100_on_1000_seeded_inputs() {
    let t = targets();
    let mut inputs = Inputs(197);
    let in_range = |c: &ScoreComponent| c.score.is_none_or(|score| score <= 100);
    // The inputs run past both ends of every table, so a clamp must be hit at
    // both ends: without this, a component that never scores passes vacuously.
    let (mut at_0, mut at_100, mut combined_scores) = (0_usize, 0_usize, 0_usize);
    for case in 0..1_000 {
        let components = vec![
            score::fleet_queue_wait(inputs.value(-MINUTE, 120.0 * MINUTE), &t),
            score::fleet_start_reliability(inputs.value(-1.0, 2.0), &t),
            score::fleet_infra_failures(inputs.value(-1.0, 2.0), &t),
            score::fleet_capacity_fit(inputs.value(-1.0, 2.0), &t),
            score::fleet_availability(inputs.value(-1.0, 2.0), &t),
        ];
        let free = inputs.value(0.0, 2_000.0);
        let machine = vec![
            score::host_availability(inputs.value(-1.0, 2.0), &t),
            score::host_start_reliability(inputs.value(-1.0, 2.0), &t),
            score::host_relative_speed(inputs.value(0.0, 10.0), &t),
            score::host_headroom(
                free.map(|gib| DiskHeadroom {
                    total_bytes: 1_000 * GIB,
                    min_free_bytes: (gib * GIB as f64) as u64,
                }),
                inputs.value(-0.5, 3.0),
                &t,
            ),
            score::host_busy_share(
                inputs.value(-1.0, 3.0),
                inputs.value(0.0, 100.0).map(|q| q as u8),
                &t,
            ),
        ];
        for set in [&components, &machine] {
            assert!(set.iter().all(in_range), "case {case}: {set:?}");
            at_0 += set.iter().filter(|c| c.score == Some(0)).count();
            at_100 += set.iter().filter(|c| c.score == Some(100)).count();
            let combined = score::combine(set);
            combined_scores += usize::from(combined.score.is_some());
            assert!(
                combined.score.is_none_or(|score| score <= 100),
                "case {case}: {combined:?}"
            );
            if set.iter().any(|c| c.score.is_some()) {
                let sum: f64 = combined.components.iter().map(|c| c.weight).sum();
                assert!((sum - 100.0).abs() < 1e-6, "case {case}: weights {sum}");
            }
        }
    }
    assert!(
        at_0 > 0,
        "no component clamped to 0 on inputs past the 0 end"
    );
    assert!(
        at_100 > 0,
        "no component clamped to 100 on inputs past the 100 end"
    );
    assert!(combined_scores > 0, "no set combined into a score");
}

/// #192's rule: the scoring engine is pure. A guard, green by design.
#[test]
fn the_scoring_engine_does_no_io() {
    for (file, source) in [
        ("score.rs", include_str!("score.rs")),
        ("concerns.rs", include_str!("concerns.rs")),
    ] {
        for crate_name in ["sqlx", "tokio", "reqwest", "std::fs"] {
            assert!(
                !source.contains(crate_name),
                "{file} must not use {crate_name}"
            );
        }
    }
}
