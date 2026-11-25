//! The window's usage by job category (#204), pure: folds the window's
//! `fleet_health_hourly` rows ([`HourlyJobRow`], rolled or live) into
//! [`CategoryUsage`] (jobs, CPU-hours, median duration and p90 queue wait
//! per category) and [`HourlyUsage`] (CPU-hours per category per hour, the
//! machine's usage chart). The fleet's usage folds every row, unmanaged
//! runners' included; a machine's folds its own.
//!
//! The median and the p90 are estimated from the merged histograms with
//! [`percentile`], as the queue-wait score is, and rounded to the
//! millisecond; CPU-hours are rounded to the hundredth. No I/O here.

use std::collections::BTreeMap;

use super::{
    CategoryUsage, HISTOGRAM_BUCKETS, Histogram, HourlyJobRow, HourlyUsage, JobCategory,
    score::percentile,
};

/// One category's sums while folding.
struct Sums {
    jobs: u64,
    cpu_seconds: f64,
    wait: Histogram,
    duration: Histogram,
}

impl Default for Sums {
    fn default() -> Self {
        Self {
            jobs: 0,
            cpu_seconds: 0.0,
            wait: [0; HISTOGRAM_BUCKETS],
            duration: [0; HISTOGRAM_BUCKETS],
        }
    }
}

/// Seconds in an hour: CPU-seconds to CPU-hours.
const SECONDS_PER_HOUR: f64 = 3_600.0;

/// CPU-seconds as CPU-hours to the hundredth, as the page shows them.
fn cpu_hours(cpu_seconds: f64) -> f64 {
    (cpu_seconds / SECONDS_PER_HOUR * 100.0).round() / 100.0
}

/// The usage of every category with jobs in `rows`, in category order:
/// final-attempt jobs, CPU-hours reserved, the median duration (p50 of the
/// duration histograms) and the p90 queue wait (of the wait histograms).
#[must_use]
pub fn categories<'a>(rows: impl IntoIterator<Item = &'a HourlyJobRow>) -> Vec<CategoryUsage> {
    let mut sums: BTreeMap<JobCategory, Sums> = BTreeMap::new();
    for row in rows {
        let entry = sums.entry(row.category).or_default();
        entry.jobs += row.job_count;
        entry.cpu_seconds += row.cpu_seconds_sum;
        for bucket in 0..HISTOGRAM_BUCKETS {
            entry.wait[bucket] += row.wait_histogram[bucket];
            entry.duration[bucket] += row.duration_histogram[bucket];
        }
    }
    sums.into_iter()
        .filter(|(_, sums)| sums.jobs > 0)
        .map(|(category, sums)| CategoryUsage {
            category,
            jobs: u32::try_from(sums.jobs).unwrap_or(u32::MAX),
            cpu_hours: cpu_hours(sums.cpu_seconds),
            median_duration_ms: percentile(&sums.duration, 0.5).map(|ms| ms.round() as i64),
            p90_queue_wait_ms: percentile(&sums.wait, 0.9).map(|ms| ms.round() as i64),
        })
        .collect()
}

/// CPU-hours by category per hour of `rows`, oldest first: one entry per
/// hour with any CPU-hours, and in it every category with some.
#[must_use]
pub fn hourly<'a>(rows: impl IntoIterator<Item = &'a HourlyJobRow>) -> Vec<HourlyUsage> {
    let mut hours: BTreeMap<i64, BTreeMap<JobCategory, f64>> = BTreeMap::new();
    for row in rows {
        if row.cpu_seconds_sum > 0.0 {
            *hours
                .entry(row.hour)
                .or_default()
                .entry(row.category)
                .or_insert(0.0) += row.cpu_seconds_sum;
        }
    }
    hours
        .into_iter()
        .map(|(hour, seconds)| HourlyUsage {
            hour,
            cpu_hours: seconds
                .into_iter()
                .map(|(category, seconds)| (category, cpu_hours(seconds)))
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet_health::{HOUR_MILLIS, RollupHost};

    const HOUR: i64 = 1_800_000_000_000;

    fn row(
        hour: i64,
        category: JobCategory,
        jobs: u64,
        cpu_seconds: f64,
        wait: Histogram,
        duration: Histogram,
    ) -> HourlyJobRow {
        HourlyJobRow {
            hour,
            host: RollupHost::Managed("io".to_owned()),
            pool_id: "pool".to_owned(),
            category,
            job_count: jobs,
            completed_count: jobs,
            infra_failure_count: 0,
            duration_ms_sum: 0,
            cpu_seconds_sum: cpu_seconds,
            wait_histogram: wait,
            duration_histogram: duration,
        }
    }

    /// Two hours of `test` merge into one entry: 10 jobs, 3 CPU-hours, the
    /// median and p90 interpolated in the merged histograms; `build` follows
    /// in category order; a category with no jobs is left out.
    #[test]
    fn categories_merge_the_rows_of_a_category() {
        // Durations: 4 in 2–5 min, 6 in 5–10 min. The median (5th of 10)
        // is the first of the 6 in 5–10 min: a sixth of the way, 350 s.
        // Waits: 9 in 0–10 s, 1 in 1–2 min. p90 = 9 of 10, the top of the
        // first bucket: 10 s.
        let rows = [
            row(
                HOUR,
                JobCategory::Test,
                4,
                3_600.0,
                [4, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 4, 0, 0, 0, 0],
            ),
            row(
                HOUR + HOUR_MILLIS,
                JobCategory::Test,
                6,
                7_200.0,
                [5, 0, 0, 1, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 6, 0, 0, 0],
            ),
            row(
                HOUR,
                JobCategory::Build,
                1,
                900.0,
                [0, 0, 0, 1, 0, 0, 0, 0, 0],
                [0, 0, 0, 1, 0, 0, 0, 0, 0],
            ),
            row(
                HOUR,
                JobCategory::Deploy,
                0,
                0.0,
                [0; HISTOGRAM_BUCKETS],
                [0; HISTOGRAM_BUCKETS],
            ),
        ];
        let usage = categories(&rows);
        assert_eq!(
            usage,
            [
                CategoryUsage {
                    category: JobCategory::Test,
                    jobs: 10,
                    cpu_hours: 3.0,
                    median_duration_ms: Some(350_000),
                    p90_queue_wait_ms: Some(10_000),
                },
                CategoryUsage {
                    category: JobCategory::Build,
                    jobs: 1,
                    cpu_hours: 0.25,
                    // One job in 1–2 min: the median is half way, 90 s; the
                    // p90 nine tenths of the way, 114 s.
                    median_duration_ms: Some(90_000),
                    p90_queue_wait_ms: Some(114_000),
                },
            ]
        );
    }

    #[test]
    fn hourly_usage_keeps_hours_and_categories_with_cpu_hours_oldest_first() {
        let empty = [0; HISTOGRAM_BUCKETS];
        let rows = [
            row(
                HOUR + HOUR_MILLIS,
                JobCategory::Check,
                2,
                1_800.0,
                empty,
                empty,
            ),
            row(HOUR, JobCategory::Test, 4, 3_600.0, empty, empty),
            row(HOUR, JobCategory::Other, 1, 0.0, empty, empty),
            row(
                HOUR + 2 * HOUR_MILLIS,
                JobCategory::Other,
                3,
                0.0,
                empty,
                empty,
            ),
            row(
                HOUR + HOUR_MILLIS,
                JobCategory::Check,
                1,
                1_800.0,
                empty,
                empty,
            ),
        ];
        assert_eq!(
            hourly(&rows),
            [
                HourlyUsage {
                    hour: HOUR,
                    cpu_hours: BTreeMap::from([(JobCategory::Test, 1.0)]),
                },
                HourlyUsage {
                    hour: HOUR + HOUR_MILLIS,
                    cpu_hours: BTreeMap::from([(JobCategory::Check, 1.0)]),
                },
            ]
        );
    }

    /// CPU-hours go on the wire to the hundredth, as the page shows them
    /// (the legend's tooltip, the usage chart): 100 CPU-seconds is 0.03 h,
    /// not 0.027777….
    #[test]
    fn cpu_hours_are_rounded_to_the_hundredth() {
        let empty = [0; HISTOGRAM_BUCKETS];
        let rows = [
            row(
                HOUR,
                JobCategory::Test,
                1,
                100.0,
                empty,
                [0, 0, 0, 0, 1, 0, 0, 0, 0],
            ),
            row(HOUR, JobCategory::Check, 1, 7_163.0, empty, empty),
        ];
        let usage = categories(&rows);
        let hours: Vec<f64> = usage.iter().map(|entry| entry.cpu_hours).collect();
        assert_eq!(hours, [0.03, 1.99], "100 s and 7,163 s (1.98972… h)");
        assert_eq!(
            hourly(&rows)[0].cpu_hours,
            BTreeMap::from([(JobCategory::Test, 0.03), (JobCategory::Check, 1.99)])
        );
    }

    #[test]
    fn no_rows_is_no_usage() {
        assert!(categories(&[]).is_empty());
        assert!(hourly(&[]).is_empty());
    }
}
