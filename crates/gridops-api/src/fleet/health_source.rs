//! The database source of the fleet health page (#204, wave 2): the
//! windowed half from the hourly rollups, the speed indexes and the scoring
//! engine (`gridops_core::fleet_health::{score, concerns, speed}`), and the
//! live half from `health_live::collect_live`.
//!
//! #197 only creates the stub. Until #204 implements it and selects it in
//! `health.rs`, the handler serves `fleet_health::fake`.

use async_trait::async_trait;
use gridops_core::fleet_health::{FleetHealthSource, HealthWindow, LiveHealth, WindowedHealth};
use sqlx::SqlitePool;

/// Reads the fleet's health from the database.
#[allow(dead_code, reason = "#204 builds it and selects it in health.rs")]
#[derive(Debug, Clone)]
pub(crate) struct DbFleetHealthSource {
    db: SqlitePool,
}

#[allow(dead_code, reason = "#204 builds it and selects it in health.rs")]
impl DbFleetHealthSource {
    pub(crate) const fn new(db: SqlitePool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl FleetHealthSource for DbFleetHealthSource {
    async fn windowed(&self, window: HealthWindow, now: i64) -> anyhow::Result<WindowedHealth> {
        let _ = (&self.db, window, now);
        anyhow::bail!("the database fleet health source arrives with #204")
    }

    async fn live(&self, now: i64) -> anyhow::Result<LiveHealth> {
        let _ = (&self.db, now);
        anyhow::bail!("the database fleet health source arrives with #204")
    }
}
