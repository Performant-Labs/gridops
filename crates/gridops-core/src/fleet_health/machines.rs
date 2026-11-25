//! The fleet machines (#262): the one rule for which `fleet_hosts` rows the
//! fleet health page, its snapshots and its rollups count, and for which part
//! of a window.
//!
//! A **fleet machine** is approved, not deleted and not retired. That is the
//! live half's machine set ([`fleet_machine_now!`]: the live machine list and every live
//! concern, the disk alarm's read included).
//!
//! Over a window, a machine counts for the part of its life inside the
//! window: from its `created_at` until its `retired_at` (only while its
//! lifecycle is `retired`; a re-enrolled machine is a new row with its own
//! `created_at`, and its retired row stays behind). So the window's machines
//! ([`overlaps`]) are the approved, non-deleted ones whose life overlaps the
//! window's covered time: one retired before the window starts isn't on the
//! page at all, one retired during it counts only for the time before
//! `retired_at`, and one created during it only from `created_at`
//! ([`HostInfo::span`]). A retired row without a `retired_at` counts as
//! retired since before any window. The machine rollup writes a row for every
//! machine that existed during the hour ([`fleet_machine_overlaps!`] on the hour, any
//! enrollment state: a revoked or deleted machine's starts before it went
//! still count for the fleet), and the speed index reads a job only when its
//! machine is approved, not deleted, and existed when the job completed
//! ([`fleet_machine_existed_at!`]).

//!
//! The SQL rules are macros expanding to string literals, so a query keeps
//! them in one `concat!` and stays a static string: each takes the
//! `fleet_hosts` alias, and [`fleet_machine_overlaps!`] and
//! [`fleet_machine_existed_at!`] SQL expressions (binds or columns) for the
//! time.

use super::{HOUR_MILLIS, HostInfo};

/// SQL: `fleet_hosts` row `$alias` is approved and not deleted.
#[macro_export]
macro_rules! fleet_machine_approved {
    ($alias:literal) => {
        concat!(
            $alias,
            ".enrollment_state='approved' AND ",
            $alias,
            ".deleted_at IS NULL"
        )
    };
}

/// SQL: `fleet_hosts` row `$alias` is a fleet machine now: approved, not
/// deleted and not retired.
#[macro_export]
macro_rules! fleet_machine_now {
    ($alias:literal) => {
        concat!(
            $crate::fleet_machine_approved!($alias),
            " AND ",
            $alias,
            ".lifecycle_state!='retired'"
        )
    };
}

/// SQL: the life of `fleet_hosts` row `$alias` overlaps `[$from, $to)`:
/// created before `$to`, and not retired, or retired after `$from`. A
/// retired row without a `retired_at` never overlaps.
#[macro_export]
macro_rules! fleet_machine_overlaps {
    ($alias:literal, $from:literal, $to:literal) => {
        concat!(
            "(",
            $alias,
            ".created_at<",
            $to,
            " AND (",
            $alias,
            ".lifecycle_state!='retired' OR ",
            $alias,
            ".retired_at>",
            $from,
            "))"
        )
    };
}

/// SQL: `fleet_hosts` row `$alias` existed at `$at`: created at or before
/// it, and not retired, or retired after it.
#[macro_export]
macro_rules! fleet_machine_existed_at {
    ($alias:literal, $at:literal) => {
        concat!(
            "(",
            $alias,
            ".created_at<=",
            $at,
            " AND (",
            $alias,
            ".lifecycle_state!='retired' OR ",
            $alias,
            ".retired_at>",
            $at,
            "))"
        )
    };
}

impl HostInfo {
    /// The part of `[from, to)` this machine counts for: from its creation
    /// until its retirement. Empty (`start >= end`) when it did not exist
    /// then.
    #[must_use]
    pub fn span(&self, from: i64, to: i64) -> (i64, i64) {
        (
            self.created_at.map_or(from, |created| from.max(created)),
            self.retired_at.map_or(to, |retired| to.min(retired)),
        )
    }

    /// Whether the hour starting at `hour` holds some of this machine's life,
    /// so its rows of that hour count.
    #[must_use]
    pub fn counts_hour(&self, hour: i64) -> bool {
        let (start, end) = self.span(hour, hour + HOUR_MILLIS);
        start < end
    }
}

#[cfg(test)]
mod tests {
    //! #262: the rule on its own, against real rows (the SQL) and in memory
    //! (the span).

    use std::path::PathBuf;

    use anyhow::Result;

    use super::{HOUR_MILLIS, HostInfo};
    use crate::connect_database_path;

    const T: i64 = 1_800_000_000_000;

    fn info(created_at: Option<i64>, retired_at: Option<i64>) -> HostInfo {
        HostInfo {
            id: "a".to_owned(),
            name: "a".to_owned(),
            cpu_budget_millis: 1_000,
            memory_budget_mib: 1_024,
            disk_total_bytes: None,
            created_at,
            retired_at,
        }
    }

    #[test]
    fn a_span_runs_from_creation_to_retirement_inside_the_range() {
        let range = (T, T + 4 * HOUR_MILLIS);
        assert_eq!(info(None, None).span(range.0, range.1), range);
        assert_eq!(info(Some(1), None).span(range.0, range.1), range);
        assert_eq!(
            info(Some(T + HOUR_MILLIS), Some(T + 2 * HOUR_MILLIS)).span(range.0, range.1),
            (T + HOUR_MILLIS, T + 2 * HOUR_MILLIS)
        );
        let (start, end) = info(Some(1), Some(T - 1)).span(range.0, range.1);
        assert!(start >= end, "retired before the range: empty");
    }

    #[test]
    fn an_hour_counts_when_it_holds_some_of_the_machine_s_life() {
        let created = T + 20 * 60_000;
        let retired = T + 2 * HOUR_MILLIS + 60_000;
        let host = info(Some(created), Some(retired));
        assert!(!host.counts_hour(T - HOUR_MILLIS), "before it was created");
        assert!(host.counts_hour(T), "created 20 minutes in");
        assert!(host.counts_hour(T + 2 * HOUR_MILLIS), "retired a minute in");
        assert!(
            !host.counts_hour(T + 3 * HOUR_MILLIS),
            "after it was retired"
        );
    }

    /// Each SQL rule against the kinds of row a re-enrolled fleet has:
    /// active, new, retired (with and without a `retired_at`), pending and
    /// deleted.
    #[tokio::test]
    async fn the_sql_rules_pick_the_rows_the_design_names() -> Result<()> {
        let directory: PathBuf =
            std::env::temp_dir().join(format!("gridops-machines-{}", uuid::Uuid::new_v4()));
        let db = connect_database_path(&directory.join("db.sqlite")).await?;
        for (id, enrollment, lifecycle, created, retired, deleted) in [
            ("active", "approved", "active", 1, None, None),
            ("new", "approved", "active", T, None, None),
            ("retired", "approved", "retired", 1, Some(T), None),
            ("retired-unknown", "approved", "retired", 1, None, None),
            ("pending", "pending", "paused", 1, None, None),
            ("deleted", "revoked", "retired", 1, Some(T), Some(T)),
        ] {
            sqlx::query(
                "INSERT INTO fleet_hosts (id,name,host_os,architecture,enrollment_state,lifecycle_state,integrity_state,created_at,updated_at,retired_at,deleted_at)
                 VALUES (?,?,'linux','x64',?,?,'verified',?,1,?,?)",
            )
            .bind(id)
            .bind(id)
            .bind(enrollment)
            .bind(lifecycle)
            .bind(created)
            .bind(retired)
            .bind(deleted)
            .execute(&db)
            .await?;
        }
        let now: Vec<String> = sqlx::query_scalar(concat!(
            "SELECT id FROM fleet_hosts fh WHERE ",
            fleet_machine_now!("fh"),
            " ORDER BY id"
        ))
        .fetch_all(&db)
        .await?;
        assert_eq!(now, ["active", "new"], "approved, not deleted, not retired");
        let approved: Vec<String> = sqlx::query_scalar(concat!(
            "SELECT id FROM fleet_hosts fh WHERE ",
            fleet_machine_approved!("fh"),
            " ORDER BY id"
        ))
        .fetch_all(&db)
        .await?;
        assert_eq!(approved, ["active", "new", "retired", "retired-unknown"]);
        let window = |from: i64, to: i64| {
            sqlx::query_scalar::<_, String>(concat!(
                "SELECT id FROM fleet_hosts fh WHERE ",
                fleet_machine_approved!("fh"),
                " AND ",
                fleet_machine_overlaps!("fh", "?1", "?2"),
                " ORDER BY id"
            ))
            .bind(from)
            .bind(to)
            .fetch_all(&db)
        };
        // A window ending at T: the retired row still lived in it, the row
        // created at T did not exist yet.
        assert_eq!(window(T - HOUR_MILLIS, T).await?, ["active", "retired"]);
        // A window starting at T: the retired row is gone, the new one is in.
        assert_eq!(window(T, T + HOUR_MILLIS).await?, ["active", "new"]);
        // The rollup's rows: every row that existed in the hour, any state.
        let rolled: Vec<String> = sqlx::query_scalar(concat!(
            "SELECT id FROM fleet_hosts fh WHERE ",
            fleet_machine_overlaps!("fh", "?1", "?2"),
            " ORDER BY id"
        ))
        .bind(T - HOUR_MILLIS)
        .bind(T)
        .fetch_all(&db)
        .await?;
        assert_eq!(rolled, ["active", "deleted", "pending", "retired"]);
        let at_t: Vec<String> = sqlx::query_scalar(concat!(
            "SELECT id FROM fleet_hosts fh WHERE ",
            fleet_machine_existed_at!("fh", "?1"),
            " ORDER BY id"
        ))
        .bind(T)
        .fetch_all(&db)
        .await?;
        assert_eq!(
            at_t,
            ["active", "new", "pending"],
            "at T: retired at T is gone, created at T is there"
        );
        db.close().await;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// The live half's machine list (`host_live_reads`, which every live
    /// concern iterates) leaves out a re-enrolled machine's retired row and
    /// a revoked machine, and keeps the active row of the same name.
    #[tokio::test]
    async fn the_live_machine_list_leaves_out_retired_and_revoked_rows() -> Result<()> {
        let directory: PathBuf =
            std::env::temp_dir().join(format!("gridops-machines-live-{}", uuid::Uuid::new_v4()));
        let db = connect_database_path(&directory.join("db.sqlite")).await?;
        let fleet = crate::fleet_health::testdata::seed_fleet(&db).await?;
        let callisto = fleet.host("callisto")?.id.clone();
        let europa = fleet.host("europa")?.id.clone();
        // callisto's row was retired when it re-enrolled; europa was revoked.
        sqlx::query("UPDATE fleet_hosts SET lifecycle_state='retired',retired_at=? WHERE id=?")
            .bind(T)
            .bind(&callisto)
            .execute(&db)
            .await?;
        sqlx::query(
            "UPDATE fleet_hosts SET enrollment_state='revoked',lifecycle_state='retired',
                    retired_at=?1 WHERE id=?2",
        )
        .bind(T)
        .bind(&europa)
        .execute(&db)
        .await?;
        let names: Vec<String> = crate::fleet::registry::host_live_reads(&db, T + HOUR_MILLIS)
            .await?
            .into_iter()
            .map(|read| read.name.as_str().to_owned())
            .collect();
        let mut expected: Vec<String> = fleet
            .hosts
            .iter()
            .filter(|host| host.id != callisto && host.id != europa)
            .map(|host| host.name.clone())
            .collect();
        expected.sort_by_key(|name| {
            fleet
                .hosts
                .iter()
                .find(|host| &host.name == name)
                .map(|host| host.id.clone())
        });
        assert_eq!(names, expected);
        db.close().await;
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
}
