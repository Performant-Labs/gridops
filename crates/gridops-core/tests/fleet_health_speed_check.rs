//! #197 section 7: the database speed check, in its own test binary (A warn
//! 8) so its deliberately slow statements can't fail another test, and no
//! other test's statements can fail these.
//!
//! `speed_check::check(db, work)` runs `work` and fails when any statement
//! takes over 100 ms (the #179 slow-statement log, `db.rs`) or any write
//! transaction holds the lock over 50 ms. The synthetic dataset builder
//! (`testdata::build_week`) gives the wave-1 contract tests 7 days of the
//! five fixture machines at realistic volume.
//!
//! This binary needs core's `test-support` feature under a plain
//! `cargo test -p gridops-core` (for example a self dev-dependency with the
//! feature); it must not be skipped silently.

use std::time::{Duration, Instant};

use anyhow::Result;
use gridops_core::{
    connect_database_path,
    fleet_health::{
        speed_check,
        testdata::{self, FLEET_HOSTS},
    },
};
use sqlx::SqlitePool;

async fn database() -> Result<(std::path::PathBuf, SqlitePool)> {
    let directory =
        std::env::temp_dir().join(format!("gridops-speed-check-{}", uuid::Uuid::new_v4()));
    let pool = connect_database_path(&directory.join("db.sqlite")).await?;
    Ok((directory, pool))
}

async fn close(directory: std::path::PathBuf, pool: SqlitePool) -> Result<()> {
    pool.close().await;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

/// A read that takes well over 100 ms on any machine.
const SLOW_QUERY: &str = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<3000000) SELECT COUNT(*) FROM c";

#[tokio::test]
async fn a_deliberately_slow_statement_fails_the_check() -> Result<()> {
    let (directory, pool) = database().await?;
    let outcome = speed_check::check(&pool, async {
        let count: i64 = sqlx::query_scalar(SLOW_QUERY).fetch_one(&pool).await?;
        Ok(count)
    })
    .await;
    let error = match outcome {
        Ok(count) => anyhow::bail!("a slow statement passed the check (returned {count})"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        error.to_lowercase().contains("slow statement"),
        "the failure names the slow statement: {error}"
    );
    close(directory, pool).await
}

#[tokio::test]
async fn a_long_write_transaction_fails_the_check() -> Result<()> {
    let (directory, pool) = database().await?;
    let outcome = speed_check::check(&pool, async {
        let mut tx = pool.begin().await?;
        sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES ('speed-check','1',1)")
            .execute(&mut *tx)
            .await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        tx.commit().await?;
        Ok(())
    })
    .await;
    let error = match outcome {
        Ok(()) => anyhow::bail!("a 150 ms write transaction passed the check"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        error.to_lowercase().contains("write transaction"),
        "the failure names the write transaction: {error}"
    );
    close(directory, pool).await
}

#[tokio::test]
async fn fast_work_passes_and_its_value_and_errors_come_through() -> Result<()> {
    let (directory, pool) = database().await?;
    let value = speed_check::check(&pool, async {
        for index in 0..20 {
            let mut tx = pool.begin().await?;
            sqlx::query("INSERT INTO settings (key,value,updated_at) VALUES (?,'1',1)")
                .bind(format!("fast-{index}"))
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(41 + 1)
    })
    .await?;
    assert_eq!(value, 42);
    let fail = value == 42;
    let failed = speed_check::check(&pool, async move {
        if fail {
            anyhow::bail!("work failed");
        }
        Ok(())
    })
    .await;
    assert!(
        failed.is_err_and(|error| format!("{error:#}").contains("work failed")),
        "the work's own error is returned"
    );
    close(directory, pool).await
}

/// The synthetic week (#197 section 7): the five fixture machines, about
/// 50,000 jobs and a sample every 30 s per machine, written in batches fast
/// enough that the whole speed-check set stays under 60 s in CI, and itself
/// within the lock rules it is used to enforce.
#[tokio::test]
async fn the_synthetic_week_has_the_fixture_machines_at_realistic_volume() -> Result<()> {
    let (directory, pool) = database().await?;
    let end = 1_800_000_000_000_i64; // a whole hour: 500,000 hours since the epoch
    // The budget is for the work, not for a busy machine (#247): the time
    // the threads waited for a CPU is not the builder's.
    let started = Instant::now();
    let (week, took) = speed_check::work_time(
        &pool,
        speed_check::check(&pool, testdata::build_week(&pool, end)),
    )
    .await;
    let week = week?;
    assert!(
        took < Duration::from_secs(20),
        "building the week took {took:?} less waits for a CPU ({:?} by the clock)",
        started.elapsed()
    );

    let hosts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT h.name,d.cpu_millis FROM fleet_hosts h JOIN host_resource_domains d ON d.host_id=h.id AND d.parent_domain_id IS NULL WHERE h.deleted_at IS NULL ORDER BY h.name",
    )
    .fetch_all(&pool)
    .await?;
    let mut expected: Vec<(String, i64)> = FLEET_HOSTS
        .iter()
        .map(|(name, cpu)| ((*name).to_owned(), i64::from(*cpu)))
        .collect();
    expected.sort();
    assert_eq!(hosts, expected);

    let week_start = end - 7 * 86_400_000;
    let jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM workflow_jobs WHERE completed_at>=? AND completed_at<?",
    )
    .bind(week_start)
    .bind(end)
    .fetch_one(&pool)
    .await?;
    assert!(
        (45_000..=55_000).contains(&jobs),
        "about 50,000 jobs in the week, got {jobs}"
    );
    assert_eq!(usize::try_from(jobs)?, week.jobs);
    let samples: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM host_samples WHERE observed_at>=? AND observed_at<?",
    )
    .bind(week_start)
    .bind(end)
    .fetch_one(&pool)
    .await?;
    // One every 30 s for 7 days, for each machine (MacBookPro included).
    assert_eq!(samples, 5 * 7 * 24 * 120);
    assert_eq!(usize::try_from(samples)?, week.samples);
    let placed: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT p.host_id) FROM workflow_jobs j JOIN runners r ON r.github_runner_id=j.runner_id JOIN workload_placements p ON p.runner_id=r.id WHERE j.completed_at>=?",
    )
    .bind(week_start)
    .fetch_one(&pool)
    .await?;
    assert_eq!(placed, 4, "jobs run on every machine with a CPU budget");
    close(directory, pool).await
}

/// #247: the check times the work, not the machine. Under load the same
/// synthetic week failed with "a write transaction held the write lock 71
/// ms", because a thread kept off the CPU by other processes still counted
/// as holding the lock. Here a transaction that does about 15 ms of work
/// shares one CPU with 11 spinning threads, so it takes far longer than
/// either limit by the wall clock, and the check passes it; on the same
/// busy CPU, a transaction that does 150 ms of work, a 300 ms read, or a
/// lock held while the caller sleeps, still fails.
///
/// The work runs in a child process of this test binary, pinned to one CPU
/// with `taskset` (util-linux), so the load costs one CPU whatever the
/// machine. The timings come from Linux's `/proc/<thread>/schedstat`:
/// elsewhere the check keeps wall-clock time and this test does not apply.
#[cfg(target_os = "linux")]
#[test]
fn work_slowed_by_a_busy_cpu_passes_and_real_work_still_fails() -> Result<()> {
    busy_cpu::run_pinned("busy_cpu::on_one_busy_cpu")
}

#[cfg(target_os = "linux")]
mod busy_cpu {
    use std::{
        future::Future,
        process::Command,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::JoinHandle,
        time::{Duration, Instant},
    };

    use anyhow::{Context as _, Result, bail, ensure};
    use gridops_core::fleet_health::speed_check::{self, SLOW_STATEMENT_LIMIT, WRITE_LOCK_LIMIT};
    use sqlx::SqlitePool;

    use super::{close, database};

    /// Set for the child, which is pinned to one CPU.
    const PINNED: &str = "GRIDOPS_SPEED_CHECK_PINNED";

    /// Threads spinning on the child's one CPU: the work gets about a
    /// twelfth of it.
    const SPINNERS: usize = 11;

    /// What the light transaction does, by the clock of an idle CPU: well
    /// under [`WRITE_LOCK_LIMIT`].
    const LIGHT_WORK: Duration = Duration::from_millis(15);

    /// Runs `inner` in a child process of this test binary, pinned to the
    /// first CPU this process may use.
    pub(super) fn run_pinned(inner: &str) -> Result<()> {
        let status = std::fs::read_to_string("/proc/self/status")?;
        let allowed = status
            .lines()
            .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
            .context("Cpus_allowed_list in /proc/self/status")?
            .trim();
        let cpu: String = allowed.chars().take_while(char::is_ascii_digit).collect();
        ensure!(!cpu.is_empty(), "no CPU in {allowed:?}");
        // `cargo test -- --include-ignored` runs the child test unpinned: it
        // must pass there (as a no-op) rather than fail the run.
        let unpinned = Command::new(std::env::current_exe()?)
            .args(["--exact", inner, "--ignored", "--test-threads=1"])
            .env_remove(PINNED)
            .output()?;
        ensure!(
            unpinned.status.success(),
            "the child test failed when run unpinned by --include-ignored:\n{}{}",
            String::from_utf8_lossy(&unpinned.stdout),
            String::from_utf8_lossy(&unpinned.stderr)
        );
        let output = Command::new("taskset")
            .args(["-c", &cpu])
            .arg(std::env::current_exe()?)
            .args([
                "--exact",
                inner,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(PINNED, "1")
            .output()
            .context("taskset (util-linux) pins this test's child to one CPU")?;
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        ensure!(
            output.status.success() && printed.contains("1 passed"),
            "the pinned child failed:\n{printed}"
        );
        Ok(())
    }

    /// Threads that keep the CPU busy until dropped.
    struct Spinners {
        stop: Arc<AtomicBool>,
        threads: Vec<JoinHandle<()>>,
    }

    impl Spinners {
        fn start(count: usize) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let threads = (0..count)
                .map(|_| {
                    let stop = Arc::clone(&stop);
                    std::thread::spawn(move || {
                        while !stop.load(Ordering::Relaxed) {
                            std::hint::spin_loop();
                        }
                    })
                })
                .collect();
            Self { stop, threads }
        }
    }

    impl Drop for Spinners {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            for thread in self.threads.drain(..) {
                let _ = thread.join();
            }
        }
    }

    /// One statement that writes `rows` rows, in its own transaction: the
    /// time the statement took, by the wall clock.
    async fn write_rows(pool: &SqlitePool, rows: i64) -> Result<Duration> {
        let mut tx = pool.begin().await?;
        let started = Instant::now();
        sqlx::query(
            "INSERT INTO busy_work (x) WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) SELECT x FROM c",
        )
        .bind(rows)
        .execute(&mut *tx)
        .await?;
        let took = started.elapsed();
        tx.commit().await?;
        Ok(took)
    }

    /// One statement that counts `rows` rows and writes nothing: the time
    /// it took, by the wall clock.
    async fn count_rows(pool: &SqlitePool, rows: i64) -> Result<Duration> {
        let started = Instant::now();
        let counted: i64 = sqlx::query_scalar(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x<?) SELECT COUNT(*) FROM c",
        )
        .bind(rows)
        .fetch_one(pool)
        .await?;
        ensure!(counted == rows);
        Ok(started.elapsed())
    }

    /// The CPU time every thread of this process has used, from Linux's
    /// `/proc/self/task/*/schedstat`.
    fn process_cpu() -> Result<Duration> {
        let mut total = Duration::ZERO;
        for task in std::fs::read_dir("/proc/self/task")? {
            // A thread can end between the listing and the read.
            let Ok(schedstat) = std::fs::read_to_string(task?.path().join("schedstat")) else {
                continue;
            };
            let nanos: u64 = schedstat
                .split_ascii_whitespace()
                .next()
                .context("schedstat")?
                .parse()?;
            total += Duration::from_nanos(nanos);
        }
        Ok(total)
    }

    /// How many rows `work` handles in `budget` of CPU time: measured by the
    /// CPU the process used, not the clock, so a CPU shared with others
    /// while it measures cannot make the work lighter (#247 review). The
    /// cheapest of a few tries.
    async fn rows_in<F: Future<Output = Result<Duration>>>(
        budget: Duration,
        work: impl Fn(i64) -> F,
    ) -> Result<i64> {
        const PROBE: i64 = 20_000;
        let mut cheapest = Duration::MAX;
        for _ in 0..5 {
            let before = process_cpu()?;
            work(PROBE).await?;
            cheapest = cheapest.min(process_cpu()?.saturating_sub(before));
        }
        let per_row = cheapest.as_secs_f64().max(1e-9) / 20_000.0;
        let rows = (budget.as_secs_f64() / per_row) as i64;
        Ok(rows.max(1_000))
    }

    #[tokio::test]
    #[ignore = "run pinned to one CPU by work_slowed_by_a_busy_cpu_passes_and_real_work_still_fails"]
    async fn on_one_busy_cpu() -> Result<()> {
        if std::env::var_os(PINNED).is_none() {
            // Run unpinned (`--include-ignored`): there is no busy CPU to
            // test on, and the parent test is the one that checks this.
            eprintln!(
                "skipped: work_slowed_by_a_busy_cpu_passes_and_real_work_still_fails runs this pinned to one CPU"
            );
            return Ok(());
        }
        let (directory, pool) = database().await?;
        sqlx::query("CREATE TABLE busy_work (x INTEGER NOT NULL)")
            .execute(&pool)
            .await?;
        let rows = rows_in(LIGHT_WORK, |rows| write_rows(&pool, rows)).await?;
        // A read that takes well over the statement limit on an idle CPU.
        let slow_read = rows_in(SLOW_STATEMENT_LIMIT * 3, |rows| count_rows(&pool, rows)).await?;

        // This process is fresh: after a first check (whose hooks create
        // its dispatcher), sqlx's slow-statement warning is first reached
        // outside a check. A check must still see the next slow statement;
        // it once stopped seeing them for good.
        speed_check::check(&pool, write_rows(&pool, 10)).await?;
        count_rows(&pool, slow_read).await?;
        let seen = speed_check::check(&pool, count_rows(&pool, slow_read)).await;
        ensure!(
            seen.is_err_and(|error| format!("{error:#}").contains("slow statement")),
            "a slow statement after one outside a check passed the check"
        );

        let spinners = Spinners::start(SPINNERS);
        // Light work, slowed by the busy CPU: it passes, and its work time
        // is the work's. Until the CPU slows one write past both limits by
        // the clock, which proves the timers took the wait off.
        let mut slowest = Duration::ZERO;
        for _ in 0..20 {
            let (took, work) =
                speed_check::work_time(&pool, speed_check::check(&pool, write_rows(&pool, rows)))
                    .await;
            let took = took.context("light work on a busy CPU failed the check")?;
            ensure!(
                work < WRITE_LOCK_LIMIT,
                "light work on a busy CPU took {work:?} of work time ({took:?} by the clock)"
            );
            slowest = slowest.max(took);
            if slowest > SLOW_STATEMENT_LIMIT {
                break;
            }
        }
        ensure!(
            slowest > SLOW_STATEMENT_LIMIT,
            "the busy CPU slowed {rows} rows only to {slowest:?}, so this proves nothing"
        );
        // Ten times the work, on the same busy CPU: it fails.
        let heavy = speed_check::check(&pool, write_rows(&pool, rows * 10)).await;
        let error = match heavy {
            Ok(took) => bail!("150 ms of work in a write transaction passed the check ({took:?})"),
            Err(error) => format!("{error:#}"),
        };
        ensure!(error.contains("write transaction"), "{error}");
        ensure!(error.contains("slow statement"), "{error}");
        // A slow read, on the same busy CPU: it fails, though its row comes
        // back before the worker, kept off the CPU, writes its log line.
        let read = speed_check::check(&pool, count_rows(&pool, slow_read)).await;
        ensure!(
            read.is_err_and(|error| format!("{error:#}").contains("slow statement")),
            "a 300 ms read on a busy CPU passed the check"
        );
        // The lock held while the caller sleeps, on the same busy CPU: it
        // fails, because a sleep is not a wait for the CPU.
        let held = speed_check::check(&pool, async {
            let mut tx = pool.begin().await?;
            sqlx::query("INSERT INTO busy_work (x) VALUES (1)")
                .execute(&mut *tx)
                .await?;
            tokio::time::sleep(Duration::from_millis(150)).await;
            tx.commit().await?;
            Ok(())
        })
        .await;
        ensure!(
            held.is_err_and(|error| format!("{error:#}").contains("write transaction")),
            "a write lock held through a 150 ms sleep on a busy CPU passed the check"
        );
        drop(spinners);
        close(directory, pool).await
    }
}
