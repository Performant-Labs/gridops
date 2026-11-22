//! #152: how many days of inventory snapshots the reconciler keeps is a
//! system setting, `inventorySnapshotRetentionDays`: 7 by default, 1 to 3,650,
//! and optional in a save, so a client that predates it leaves it as it is.

use super::*;

const KEY: &str = "inventorySnapshotRetentionDays";

/// Saves the settings with their defaults, and `days` when given.
async fn save(f: &Fixture, days: Option<i64>) -> Result<StatusCode> {
    let mut body = json!({
        "logRetentionDays": 30, "logStorageBudgetMb": 4096, "webhookRetentionDays": 90,
        "auditRetentionDays": 365, "reconcileIntervalSeconds": 30,
        "githubSyncIntervalSeconds": 60, "autoUpdateImages": false, "provisioningPaused": false,
    });
    if let Some(days) = days {
        body[KEY] = json!(days);
    }
    Ok(f.client
        .put(format!("{}/api/v1/settings", f.origin))
        .header(header::COOKIE, &f.cookie)
        .header(header::ORIGIN, &f.origin)
        .json(&body)
        .send()
        .await?
        .status())
}

/// The value the settings page is given.
async fn shown(f: &Fixture) -> Result<Value> {
    let settings: Value = f
        .client
        .get(format!("{}/api/v1/settings", f.origin))
        .header(header::COOKIE, &f.cookie)
        .send()
        .await?
        .json()
        .await?;
    Ok(settings["data"]["settings"][KEY].clone())
}

#[tokio::test]
async fn snapshot_retention_days_are_a_setting_left_alone_when_a_save_omits_them() -> Result<()> {
    let f = Fixture::new().await?;
    assert_eq!(shown(&f).await?, json!(7), "seven days by default");

    assert_eq!(save(&f, Some(30)).await?, StatusCode::OK);
    assert_eq!(shown(&f).await?, json!(30));
    // Stored where the reconciler reads it, as a JSON number.
    let stored: String = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
        .bind(KEY)
        .fetch_one(&f.database)
        .await?;
    assert_eq!(stored, "30");
    let audited: String = sqlx::query_scalar(
        "SELECT metadata FROM audit_events WHERE action='settings.updated'
          ORDER BY created_at DESC,rowid DESC LIMIT 1",
    )
    .fetch_one(&f.database)
    .await?;
    assert_eq!(
        serde_json::from_str::<Value>(&audited)?[KEY],
        json!(30),
        "{audited}"
    );

    // A client that predates the setting leaves it as it is.
    assert_eq!(save(&f, None).await?, StatusCode::OK);
    assert_eq!(shown(&f).await?, json!(30));

    // Outside 1..=3650 is refused and changes nothing.
    for refused in [0, -1, 3_651] {
        assert_eq!(
            save(&f, Some(refused)).await?,
            StatusCode::BAD_REQUEST,
            "{refused}"
        );
    }
    assert_eq!(
        shown(&f).await?,
        json!(30),
        "a refused value changes nothing"
    );

    // Both ends of the range are allowed.
    for allowed in [1, 3_650] {
        assert_eq!(save(&f, Some(allowed)).await?, StatusCode::OK, "{allowed}");
        assert_eq!(shown(&f).await?, json!(allowed));
    }
    Ok(())
}
