use sqlx::Row as _;
use sqlx::SqlitePool;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryCapacity {
    pub repository_id: i64,
    pub installation_id: i64,
    pub owner: String,
    pub name: String,
    pub queued: i64,
    pub active: i64,
    pub busy: i64,
    /// Queued jobs that count as prioritised; see [`QueuedJob::prioritised`].
    pub prioritised: i64,
    /// When the oldest of those jobs was queued.
    pub prioritised_since: Option<i64>,
}

impl RepositoryCapacity {
    /// When this repository's oldest prioritised job was queued, if it has
    /// more prioritised jobs than runners waiting to take them.
    pub fn unmet_priority(&self, factor: i64) -> Option<i64> {
        (self.prioritised.saturating_mul(factor) > self.active.saturating_sub(self.busy))
            .then_some(self.prioritised_since)
            .flatten()
    }
}

/// A queued job the pool would run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedJob {
    pub repository_id: i64,
    pub labels: Vec<String>,
    pub queued_at: i64,
    /// Its repository is prioritised in the pool, or it has waited longer than
    /// the pool's priority window, so a busy prioritised repository can't
    /// starve the others.
    pub prioritised: bool,
}

/// Whether a queued job counts as prioritised in the pool. Binds the pool id,
/// the current time and the pool id again.
macro_rules! prioritised_job {
    () => {
        "(EXISTS (SELECT 1 FROM runner_pool_priority_repositories flagged
    WHERE flagged.pool_id=? AND flagged.repository_id=wr.repository_id)
  OR wj.created_at <= ? - 60000*COALESCE(
    (SELECT wait_minutes FROM runner_pool_priorities WHERE pool_id=?),10))"
    };
}

/// The platforms a pool's runners run on, one `os,arch` row each:
/// the built-in host's (Docker is Linux on the host's architecture, Tart is
/// macOS on arm64) unless the pool runs on machines only, then each enabled
/// fleet platform. Correlated on the pool as `candidate`; binds the built-in
/// host's architecture label ([`runner_arch_label`]). Queue ownership and
/// [`pool_platform_labels`] both read this, so a label counts as offered
/// exactly when some pool could run it.
macro_rules! pool_platforms {
    () => {
        "
                -- The built-in host's runners, unless the pool runs on machines only.
                SELECT CASE lower(CAST(provider.value AS TEXT)) WHEN 'tart' THEN 'macos' ELSE 'linux' END AS os,
                  CASE lower(CAST(provider.value AS TEXT)) WHEN 'tart' THEN 'arm64' ELSE lower(?) END AS arch
                FROM json_each(candidate.providers) provider
                WHERE lower(CAST(provider.value AS TEXT)) IN ('docker','tart')
                  AND NOT EXISTS (SELECT 1 FROM pool_execution_profiles machines_only
                    WHERE machines_only.pool_id=candidate.id AND machines_only.enabled=1
                      AND machines_only.archived_at IS NULL AND machines_only.placement='machines')
                UNION ALL
                -- Each fleet platform the pool runs on.
                SELECT profile.execution_os,profile.architecture FROM pool_execution_profiles profile
                WHERE profile.pool_id=candidate.id AND profile.enabled=1 AND profile.archived_at IS NULL
              "
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCapacity {
    pub provider: String,
    pub queued: i64,
    pub active: i64,
    pub busy: i64,
}

pub fn runner_arch_label() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        "arm" => "arm",
        architecture => architecture,
    }
}

/// The one label every Linux runner carries, whatever its pool configures, so
/// a workflow that asks for `self-hosted-linux` can run on any of them.
pub const LINUX_RUNNER_LABEL: &str = "self-hosted-linux";

/// The same for macOS runners.
pub const MACOS_RUNNER_LABEL: &str = "self-hosted-macos";

pub fn runner_system_labels(provider: &str) -> [&'static str; 3] {
    match provider {
        "tart" => ["self-hosted", "macOS", "ARM64"],
        _ => ["self-hosted", "linux", runner_arch_label()],
    }
}

pub fn effective_runner_labels(provider: &str, configured: &[String]) -> Vec<String> {
    let mut labels = runner_system_labels(provider)
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    labels.push(
        if provider == "tart" {
            MACOS_RUNNER_LABEL
        } else {
            LINUX_RUNNER_LABEL
        }
        .to_owned(),
    );
    for label in configured {
        if !is_runner_system_label(label)
            && !labels
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(label))
        {
            labels.push(label.clone());
        }
    }
    labels
}

pub fn runner_supports_system_label(provider: &str, label: &str) -> bool {
    runner_system_labels(provider)
        .iter()
        .any(|system| system.eq_ignore_ascii_case(label))
}

pub fn compatible_runner_provider(
    providers: &[String],
    requested: &[String],
    configured: &[String],
) -> Option<String> {
    providers
        .iter()
        .find(|provider| provider_supports_labels(provider, requested, configured))
        .cloned()
}

pub fn provider_supports_labels(
    provider: &str,
    requested: &[String],
    configured: &[String],
) -> bool {
    requested.iter().all(|requested| {
        if requested.eq_ignore_ascii_case("self-hosted") {
            return true;
        }
        if requested.eq_ignore_ascii_case(LINUX_RUNNER_LABEL) {
            return provider != "tart";
        }
        if requested.eq_ignore_ascii_case(MACOS_RUNNER_LABEL) {
            return provider == "tart";
        }
        if is_runner_system_label(requested) {
            return runner_supports_system_label(provider, requested);
        }
        configured
            .iter()
            .any(|label| label.eq_ignore_ascii_case(requested))
    })
}

pub fn is_runner_system_label(label: &str) -> bool {
    [
        "self-hosted",
        "linux",
        "macos",
        "windows",
        "x64",
        "arm64",
        "arm",
        "x86",
    ]
    .iter()
    .any(|system| system.eq_ignore_ascii_case(label))
}

pub fn scale_up_target(desired: i64, busy: i64, queued: i64, factor: i64, maximum: i64) -> i64 {
    maximum.min(desired.max(busy.saturating_add(queued.saturating_mul(factor))))
}

pub async fn assigned_queued_jobs(
    database: &SqlitePool,
    pool_id: &str,
) -> Result<i64, sqlx::Error> {
    let owned = owned_queued_jobs(database, pool_id).await?;
    Ok(i64::try_from(owned.len()).unwrap_or(i64::MAX))
}

/// Ids of the queued jobs this pool owns, oldest first; see
/// [`queued_job_owners`].
async fn owned_queued_jobs(database: &SqlitePool, pool_id: &str) -> Result<Vec<i64>, sqlx::Error> {
    Ok(queued_job_owners(database)
        .await?
        .into_iter()
        .filter_map(|owner| (owner.pool_id == pool_id).then_some(owner.job_id))
        .collect())
}

/// Which pool counts a queued job, as the scaler decides it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedJobOwner {
    pub job_id: i64,
    pub pool_id: String,
    /// Every pool that could run the job had used up its room when the job was
    /// handed out, so it stays with its first candidate until one frees up.
    pub over_capacity: bool,
}

/// The owner of every queued job that has one, oldest job first. A queued job
/// missing from the result has no unpaused autoscaling pool that serves its
/// repository and can run its labels. This is the ownership the scaler uses
/// ([`queued_job_owners`]); read it rather than matching labels again.
pub async fn queued_job_ownership(
    database: &SqlitePool,
) -> Result<Vec<QueuedJobOwner>, sqlx::Error> {
    queued_job_owners(database).await
}

/// The one pool that counts each queued job, oldest job first. Every view of
/// a pool's queue ([`assigned_queued_jobs`], [`queued_job_assignments`],
/// [`repository_capacities`]) is built from this, so no two of them disagree
/// about who owns a job and no job makes two pools scale.
///
/// A job's candidates are the unpaused autoscaling pools that serve its
/// repository and can run its labels, repository pools first, then the
/// oldest, then by id. The job goes to the first candidate with room left,
/// where a pool's room is `max_count` less its busy runners: an idle or
/// starting runner can take a queued job and the pool can grow to
/// `max_count`, but a busy runner can't. Busy runners are counted as the
/// reconciler counts them: not deleted and in an active status. Jobs are
/// handed out oldest first and each one uses up a unit of its owner's room. If
/// every candidate is full the job stays with the first one, as it did before
/// room was considered, so a queue with nowhere to go looks the same.
///
/// Owners depend only on the queued jobs, the pools and their busy counts,
/// and every ordering ends in an id, so the same state always gives the same
/// owners. Since older jobs claim room first, a newly queued job never moves
/// an older one; a pool near its limit only gains or loses jobs when its own
/// busy count, the queue or a pool's settings change, so ownership can't
/// alternate between ticks on its own.
async fn queued_job_owners(database: &SqlitePool) -> Result<Vec<QueuedJobOwner>, sqlx::Error> {
    let candidates = queued_job_candidates(database, false).await?;
    Ok(assign_queued_jobs(candidates.into_iter().map(
        |candidate| (candidate.job, candidate.pool, candidate.room),
    )))
}

/// The owner of every job waiting to start that has one, for showing the queue
/// to people: [`queued_job_ownership`] with two differences. Jobs GitHub lists
/// as `waiting`, `pending` or `requested` count as well as `queued` ones, so
/// the Overview counts the same statuses as the run list, and a pool whose
/// provisioning circuit is open is passed over, since it can't start a runner
/// until it is retried. The pools that did the scaling are not changed by
/// either difference. A job with no entry has no pool that can serve it: none
/// has its labels, or every one that does is paused, has autoscaling off or
/// has an open circuit.
pub async fn waiting_job_ownership(
    database: &SqlitePool,
) -> Result<Vec<QueuedJobOwner>, sqlx::Error> {
    let candidates = queued_job_candidates(database, true).await?;
    Ok(assign_queued_jobs(
        candidates
            .into_iter()
            .filter(|candidate| !candidate.circuit_open)
            .map(|candidate| (candidate.job, candidate.pool, candidate.room)),
    ))
}

/// A pool that could run a queued job, with the room it has left.
struct QueuedJobCandidate {
    job: i64,
    pool: String,
    room: i64,
    circuit_open: bool,
}

/// Every (job, pool) pair where the pool could run the job, each job's pools
/// together in the order [`queued_job_owners`] prefers them. With
/// `every_waiting_status`, jobs that are `waiting`, `pending` or `requested`
/// are included as well as `queued` ones.
async fn queued_job_candidates(
    database: &SqlitePool,
    every_waiting_status: bool,
) -> Result<Vec<QueuedJobCandidate>, sqlx::Error> {
    let rows = sqlx::query(concat!(
        r#"SELECT wj.id AS job_id,candidate.id AS pool_id,candidate.provision_circuit_open AS circuit_open,
          candidate.max_count-(SELECT COUNT(*) FROM runners runner
            WHERE runner.pool_id=candidate.id AND runner.deleted_at IS NULL AND runner.busy=1
              AND runner.status IN ('starting','online','idle','busy','paused','stopped')) AS room
          FROM workflow_jobs wj
          JOIN workflow_runs wr ON wr.id=wj.run_id
          JOIN repositories repo ON repo.id=wr.repository_id
          JOIN runner_pools candidate ON candidate.autoscaling_enabled=1 AND candidate.paused=0 AND (
              EXISTS (SELECT 1 FROM runner_pool_repositories candidate_repo
                WHERE candidate_repo.pool_id=candidate.id AND candidate_repo.repository_id=repo.id) OR
              candidate.repository_id=repo.id OR
              (candidate.scope='organization' AND candidate.installation_id=repo.installation_id)
            ) AND EXISTS (
              SELECT 1 FROM ("#,
        pool_platforms!(),
        r#") platform
              WHERE NOT EXISTS (
                SELECT 1 FROM json_each(wj.labels) requested
                WHERE NOT (
                  lower(CAST(requested.value AS TEXT))='self-hosted' OR
                  (lower(CAST(requested.value AS TEXT)) NOT IN ('linux','macos','windows','x64','arm64','arm','x86') AND EXISTS (
                    SELECT 1 FROM json_each(candidate.labels) assigned
                    WHERE lower(CAST(assigned.value AS TEXT))=lower(CAST(requested.value AS TEXT))
                  )) OR
                  lower(CAST(requested.value AS TEXT)) IN (platform.os,platform.arch) OR
                  -- Every Linux and macOS runner takes its platform label, configured or not.
                  (lower(CAST(requested.value AS TEXT))='self-hosted-linux' AND platform.os='linux') OR
                  (lower(CAST(requested.value AS TEXT))='self-hosted-macos' AND platform.os='macos')
                )
              )
            )
          WHERE (wj.status='queued' OR (? AND wj.status IN ('waiting','pending','requested')))
            AND wr.status!='completed'
          ORDER BY wj.created_at,wj.id,
            CASE WHEN candidate.scope='repository' THEN 0 ELSE 1 END,
            candidate.created_at,candidate.id"#
    ))
    .bind(runner_arch_label())
    .bind(every_waiting_status)
    .fetch_all(database)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| QueuedJobCandidate {
            job: row.get::<i64, _>("job_id"),
            pool: row.get::<String, _>("pool_id"),
            room: row.get::<i64, _>("room"),
            circuit_open: row.get::<bool, _>("circuit_open"),
        })
        .collect())
}

/// The platform labels each pool offers, by pool id, lowercase, sorted and
/// without repeats: the OS and architecture of each platform it runs on (the
/// same platforms queue ownership matches system labels against), plus
/// [`LINUX_RUNNER_LABEL`] or [`MACOS_RUNNER_LABEL`] for a Linux or macOS
/// platform. A pool's configured labels never offer these; a pool with no
/// platform maps to an empty list.
pub async fn pool_platform_labels(
    database: &SqlitePool,
) -> Result<std::collections::HashMap<String, Vec<String>>, sqlx::Error> {
    let rows = sqlx::query(concat!(
        r#"SELECT candidate.id AS pool_id,
          (SELECT json_group_array(json_array(platform.os,platform.arch)) FROM ("#,
        pool_platforms!(),
        r#") platform) AS platforms
          FROM runner_pools candidate"#
    ))
    .bind(runner_arch_label())
    .fetch_all(database)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let platforms =
                serde_json::from_str::<Vec<(String, String)>>(row.get::<&str, _>("platforms"))
                    .unwrap_or_default();
            let mut labels = platforms
                .into_iter()
                .flat_map(|(os, arch)| {
                    let os = os.to_lowercase();
                    let named = match os.as_str() {
                        "linux" => Some(LINUX_RUNNER_LABEL.to_owned()),
                        "macos" => Some(MACOS_RUNNER_LABEL.to_owned()),
                        _ => None,
                    };
                    [Some(os), Some(arch.to_lowercase()), named]
                })
                .flatten()
                .collect::<Vec<_>>();
            labels.sort();
            labels.dedup();
            (row.get::<String, _>("pool_id"), labels)
        })
        .collect())
}

/// Gives each job to its first candidate with room; see [`queued_job_owners`].
/// `candidates` holds `(job, pool, room)` with each job's rows together, jobs
/// oldest first and each job's pools in order.
fn assign_queued_jobs(
    candidates: impl IntoIterator<Item = (i64, String, i64)>,
) -> Vec<QueuedJobOwner> {
    let mut room = std::collections::HashMap::<String, i64>::new();
    let mut jobs = Vec::<(i64, Vec<String>)>::new();
    for (job, pool, pool_room) in candidates {
        room.entry(pool.clone()).or_insert(pool_room);
        match jobs.last_mut() {
            Some((last, pools)) if *last == job => pools.push(pool),
            _ => jobs.push((job, vec![pool])),
        }
    }
    jobs.into_iter()
        .filter_map(|(job, pools)| {
            let with_room = pools
                .iter()
                .find(|pool| room.get(*pool).is_some_and(|room| *room > 0));
            let over_capacity = with_room.is_none();
            let owner = with_room.or_else(|| pools.first())?.clone();
            if let Some(room) = room.get_mut(&owner) {
                *room -= 1;
            }
            Some(QueuedJobOwner {
                job_id: job,
                pool_id: owner,
                over_capacity,
            })
        })
        .collect()
}

pub async fn provider_capacities(
    database: &SqlitePool,
    pool_id: &str,
    providers: &[String],
    configured_labels: &[String],
) -> Result<Vec<ProviderCapacity>, sqlx::Error> {
    let mut capacities = providers
        .iter()
        .map(|provider| ProviderCapacity {
            provider: provider.clone(),
            queued: 0,
            active: 0,
            busy: 0,
        })
        .collect::<Vec<_>>();
    for job in queued_job_assignments(database, pool_id).await? {
        if let Some(provider) =
            compatible_runner_provider(providers, &job.labels, configured_labels)
            && let Some(capacity) = capacities
                .iter_mut()
                .find(|capacity| capacity.provider == provider)
        {
            capacity.queued = capacity.queued.saturating_add(1);
        }
    }
    let rows = sqlx::query(
        r#"SELECT provider,COUNT(*) AS active,
          COUNT(CASE WHEN busy=1 THEN 1 END) AS busy
          FROM runners WHERE pool_id=? AND deleted_at IS NULL
            AND status IN ('starting','online','idle','busy','paused','stopped')
          GROUP BY provider"#,
    )
    .bind(pool_id)
    .fetch_all(database)
    .await?;
    for row in rows {
        let provider = row.get::<String, _>("provider");
        if let Some(capacity) = capacities
            .iter_mut()
            .find(|capacity| capacity.provider == provider)
        {
            capacity.active = row.get("active");
            capacity.busy = row.get("busy");
        }
    }
    Ok(capacities)
}

pub async fn next_runner_provider(
    database: &SqlitePool,
    pool_id: &str,
    repository_id: Option<i64>,
    providers: &[String],
    configured_labels: &[String],
    factor: i64,
) -> Result<Option<String>, sqlx::Error> {
    let assignments = queued_job_assignments(database, pool_id).await?;
    let mut capacities = providers
        .iter()
        .map(|provider| ProviderCapacity {
            provider: provider.clone(),
            queued: 0,
            active: 0,
            busy: 0,
        })
        .collect::<Vec<_>>();
    for job in &assignments {
        if repository_id.is_none_or(|repository_id| repository_id == job.repository_id)
            && let Some(provider) =
                compatible_runner_provider(providers, &job.labels, configured_labels)
            && let Some(capacity) = capacities
                .iter_mut()
                .find(|capacity| capacity.provider == provider)
        {
            capacity.queued = capacity.queued.saturating_add(1);
        }
    }
    let rows = sqlx::query(
        r#"SELECT provider,COUNT(*) AS active,
          COUNT(CASE WHEN busy=1 THEN 1 END) AS busy
          FROM runners WHERE pool_id=? AND deleted_at IS NULL
            AND status IN ('starting','online','idle','busy','paused','stopped')
            AND (? IS NULL OR target_repository_id=?)
          GROUP BY provider"#,
    )
    .bind(pool_id)
    .bind(repository_id)
    .bind(repository_id)
    .fetch_all(database)
    .await?;
    for row in rows {
        let provider = row.get::<String, _>("provider");
        if let Some(capacity) = capacities
            .iter_mut()
            .find(|capacity| capacity.provider == provider)
        {
            capacity.active = row.get("active");
            capacity.busy = row.get("busy");
        }
    }
    if let Some(capacity) = capacities
        .iter()
        .enumerate()
        .filter(|(_, capacity)| provider_capacity_deficit(capacity, factor) > 0)
        .max_by(|(left_index, left), (right_index, right)| {
            provider_capacity_deficit(left, factor)
                .cmp(&provider_capacity_deficit(right, factor))
                .then_with(|| left.queued.cmp(&right.queued))
                .then_with(|| right_index.cmp(left_index))
        })
        .map(|(_, capacity)| capacity)
    {
        return Ok(Some(capacity.provider.clone()));
    }
    Ok(providers.first().cloned())
}

pub fn provider_capacity_deficit(capacity: &ProviderCapacity, factor: i64) -> i64 {
    capacity
        .busy
        .saturating_add(capacity.queued.saturating_mul(factor))
        .saturating_sub(capacity.active)
}

/// Labels of the queued jobs assigned to this pool, with their repository.
///
/// Here and wherever jobs count as queued, a job in a finished run doesn't:
/// GitHub sometimes leaves one listed as queued after its run completes, and
/// it never runs, so a runner for it would wait forever (and, once it's old
/// enough to be prioritised, take every new runner).
pub async fn queued_job_assignments(
    database: &SqlitePool,
    pool_id: &str,
) -> Result<Vec<QueuedJob>, sqlx::Error> {
    let owned = owned_queued_jobs(database, pool_id).await?;
    let sql = concat!(
        r#"SELECT repo.id AS repository_id,wj.labels,wj.created_at,
          "#,
        prioritised_job!(),
        r#" AS prioritised FROM workflow_jobs wj
          JOIN workflow_runs wr ON wr.id=wj.run_id
          JOIN repositories repo ON repo.id=wr.repository_id
          WHERE wj.status='queued' AND wr.status!='completed'
            AND wj.id IN (SELECT value FROM json_each(?))
          ORDER BY wj.created_at,wj.id"#
    );
    let rows = sqlx::query(sql)
        .bind(pool_id)
        .bind(crate::now_millis())
        .bind(pool_id)
        .bind(serde_json::to_string(&owned).unwrap_or_else(|_| "[]".into()))
        .fetch_all(database)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| QueuedJob {
            repository_id: row.get("repository_id"),
            labels: serde_json::from_str(row.get::<&str, _>("labels")).unwrap_or_default(),
            queued_at: row.get("created_at"),
            prioritised: row.get("prioritised"),
        })
        .collect())
}

pub async fn repository_capacities(
    database: &SqlitePool,
    pool_id: &str,
) -> Result<Vec<RepositoryCapacity>, sqlx::Error> {
    let sql = concat!(
        r#"SELECT repo.id,repo.installation_id,repo.owner,repo.name,
          COUNT(job.repository_id) AS queued,
          COALESCE(SUM(job.prioritised),0) AS prioritised,
          MIN(CASE WHEN job.prioritised THEN job.created_at END) AS prioritised_since,
          (SELECT COUNT(*) FROM runners runner
            WHERE runner.pool_id=? AND runner.target_repository_id=repo.id
              AND runner.deleted_at IS NULL
              AND runner.status IN ('starting','online','idle','busy','paused','stopped')) AS active,
          (SELECT COUNT(*) FROM runners runner
            WHERE runner.pool_id=? AND runner.target_repository_id=repo.id
              AND runner.deleted_at IS NULL AND runner.busy=1
              AND runner.status IN ('starting','online','idle','busy','paused','stopped')) AS busy
          FROM runner_pool_repositories membership
          JOIN repositories repo ON repo.id=membership.repository_id
          LEFT JOIN (
            SELECT wr.repository_id,wj.created_at,"#,
        prioritised_job!(),
        r#" AS prioritised
            FROM workflow_jobs wj
            JOIN workflow_runs wr ON wr.id=wj.run_id
            WHERE wj.status='queued' AND wr.status!='completed'
              AND wj.id IN (SELECT value FROM json_each(?))
          ) job ON job.repository_id=repo.id
          WHERE membership.pool_id=? AND repo.archived=0
          GROUP BY repo.id
          ORDER BY membership.created_at,repo.id"#
    );
    let owned = owned_queued_jobs(database, pool_id).await?;
    let rows = sqlx::query(sql)
        .bind(pool_id)
        .bind(pool_id)
        .bind(pool_id)
        .bind(crate::now_millis())
        .bind(pool_id)
        .bind(serde_json::to_string(&owned).unwrap_or_else(|_| "[]".into()))
        .bind(pool_id)
        .fetch_all(database)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| RepositoryCapacity {
            repository_id: row.get("id"),
            installation_id: row.get("installation_id"),
            owner: row.get("owner"),
            name: row.get("name"),
            queued: row.get("queued"),
            active: row.get("active"),
            busy: row.get("busy"),
            prioritised: row.get("prioritised"),
            prioritised_since: row.get("prioritised_since"),
        })
        .collect())
}

pub fn repository_capacity_deficit(capacity: &RepositoryCapacity, factor: i64) -> i64 {
    capacity
        .busy
        .saturating_add(capacity.queued.saturating_mul(factor))
        .saturating_sub(capacity.active)
}

pub async fn next_runner_repository(
    database: &SqlitePool,
    pool_id: &str,
    factor: i64,
) -> Result<Option<RepositoryCapacity>, sqlx::Error> {
    let mut capacities = repository_capacities(database, pool_id).await?;
    // Repositories with prioritised jobs still waiting for a runner go first,
    // the longest-waiting first.
    let urgency = |capacity: &RepositoryCapacity| {
        capacity
            .unmet_priority(factor)
            .map_or((1, 0), |since| (0, since))
    };
    capacities.sort_by(|left, right| {
        urgency(left)
            .cmp(&urgency(right))
            .then_with(|| {
                repository_capacity_deficit(right, factor)
                    .cmp(&repository_capacity_deficit(left, factor))
            })
            .then_with(|| right.queued.cmp(&left.queued))
            .then_with(|| left.active.cmp(&right.active))
            .then_with(|| left.repository_id.cmp(&right.repository_id))
    });
    Ok(capacities.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect_database_path;
    use std::fs;

    #[test]
    fn effective_labels_include_supported_runner_system_labels() {
        let labels = effective_runner_labels(
            "docker",
            &["gridops".into(), "SELF-HOSTED".into(), "pool-a".into()],
        );
        assert!(labels.iter().any(|label| label == "self-hosted"));
        assert!(labels.iter().any(|label| label == "linux"));
        assert!(labels.iter().any(|label| label == runner_arch_label()));
        assert!(labels.iter().any(|label| label == "gridops"));
        assert!(labels.iter().any(|label| label == "pool-a"));
        assert_eq!(
            labels
                .iter()
                .filter(|label| label.eq_ignore_ascii_case("self-hosted"))
                .count(),
            1
        );
    }

    #[test]
    fn unsupported_system_labels_are_not_treated_as_available() {
        assert!(runner_supports_system_label("docker", "self-hosted"));
        assert!(runner_supports_system_label("docker", "Linux"));
        assert!(runner_supports_system_label("docker", runner_arch_label()));
        assert!(!runner_supports_system_label("docker", "windows"));
        assert!(runner_supports_system_label("tart", "macOS"));
        assert!(runner_supports_system_label("tart", "arm64"));
        assert!(!runner_supports_system_label("tart", "linux"));
        assert!(
            !effective_runner_labels("docker", &["macOS".into()])
                .iter()
                .any(|label| label.eq_ignore_ascii_case("macos"))
        );
    }

    #[test]
    fn every_linux_runner_takes_the_shared_linux_label() {
        // Docker runners carry it whether or not the pool lists it, once.
        for configured in [vec![], vec!["Self-Hosted-Linux".to_owned()]] {
            let labels = effective_runner_labels("docker", &configured);
            assert_eq!(
                labels
                    .iter()
                    .filter(|label| label.eq_ignore_ascii_case(LINUX_RUNNER_LABEL))
                    .count(),
                1,
                "{labels:?}"
            );
        }
        assert!(provider_supports_labels(
            "docker",
            &["self-hosted-linux".into()],
            &[]
        ));
        // A macOS VM isn't a Linux runner, even if its pool lists the label.
        assert!(!provider_supports_labels(
            "tart",
            &["self-hosted-linux".into()],
            &["self-hosted-linux".into()]
        ));
        assert!(
            !effective_runner_labels("tart", &[])
                .iter()
                .any(|label| label.eq_ignore_ascii_case(LINUX_RUNNER_LABEL))
        );
        // And macOS runners carry theirs, which Linux ones don't.
        assert!(provider_supports_labels(
            "tart",
            &["self-hosted-macos".into()],
            &[]
        ));
        assert!(!provider_supports_labels(
            "docker",
            &["self-hosted-macos".into()],
            &["self-hosted-macos".into()]
        ));
        assert!(
            effective_runner_labels("tart", &[])
                .iter()
                .any(|label| label.eq_ignore_ascii_case(MACOS_RUNNER_LABEL))
        );
        assert!(
            !effective_runner_labels("docker", &[])
                .iter()
                .any(|label| label.eq_ignore_ascii_case(MACOS_RUNNER_LABEL))
        );
    }

    #[test]
    fn mixed_pools_choose_the_provider_that_matches_job_system_labels() {
        let providers = vec!["docker".into(), "tart".into()];
        let configured = vec!["gridops".into(), "release".into()];
        assert_eq!(
            compatible_runner_provider(
                &providers,
                &["self-hosted".into(), "macOS".into(), "release".into()],
                &configured,
            ),
            Some("tart".into())
        );
        assert_eq!(
            compatible_runner_provider(
                &providers,
                &["self-hosted".into(), "linux".into()],
                &configured,
            ),
            Some("docker".into())
        );
        assert_eq!(
            compatible_runner_provider(
                &providers,
                &["self-hosted".into(), "windows".into()],
                &configured,
            ),
            None
        );
        assert_eq!(
            compatible_runner_provider(&providers, &["self-hosted".into()], &configured),
            Some("docker".into())
        );
    }

    #[test]
    fn scale_target_is_idempotent_for_the_same_queued_work() {
        assert_eq!(scale_up_target(0, 0, 1, 1, 10), 1);
        assert_eq!(scale_up_target(1, 0, 1, 1, 10), 1);
        assert_eq!(scale_up_target(1, 1, 1, 1, 10), 2);
        assert_eq!(scale_up_target(2, 1, 1, 1, 10), 2);
    }

    #[tokio::test]
    async fn provisioning_chooses_the_provider_with_an_unfilled_queue() -> anyhow::Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-provider-placement-test-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1);
            INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,provider,providers,labels,image,docker_image,tart_image,autoscaling_enabled,created_at,updated_at)
              VALUES ('mixed',1,10,'mixed','repository','docker','["docker","tart"]','["mixed"]','runner:latest','runner:latest','macos-base',1,1,1);
            INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at)
              VALUES ('mixed',10,1);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES (20,10,'CI',1,'push','queued','abc123','https://github.com/octo-org/gridops/actions/runs/20',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES
              (30,20,'linux','queued','["self-hosted","linux"]','https://github.com/octo-org/gridops/actions/runs/20/job/30',1,1),
              (31,20,'macos','queued','["self-hosted","macOS"]','https://github.com/octo-org/gridops/actions/runs/20/job/31',2,2);
            INSERT INTO runners (id,pool_id,target_repository_id,name,provider,status,created_at,updated_at)
              VALUES ('linux-runner','mixed',10,'mixed-linux','docker','online',1,1);
            "#,
        )
        .execute(&database)
        .await?;

        let providers = vec!["docker".into(), "tart".into()];
        assert_eq!(
            next_runner_provider(
                &database,
                "mixed",
                Some(10),
                &providers,
                &["mixed".into()],
                1,
            )
            .await?,
            Some("tart".into())
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn a_platform_job_is_owned_by_any_pool_of_that_platform_not_just_ones_that_list_the_label()
    -> anyhow::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("gridops-linux-label-test-{}", uuid::Uuid::new_v4()));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        // 'plain' never lists self-hosted-linux and 'mac' is Tart only.
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1);
            INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,provider,providers,labels,image,docker_image,tart_image,autoscaling_enabled,created_at,updated_at)
              VALUES
              ('plain',1,10,'plain','repository','docker','["docker"]','["gridops"]','runner:latest','runner:latest','macos-base',1,1,1),
              ('mac',1,10,'mac','repository','tart','["tart"]','["self-hosted-linux"]','runner:latest','runner:latest','macos-base',1,2,2);
            INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at)
              VALUES ('plain',10,1),('mac',10,1);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES (20,10,'CI',1,'push','queued','abc123','https://github.com/octo-org/gridops/actions/runs/20',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES
              (30,20,'test','queued','["self-hosted-linux"]','https://github.com/octo-org/gridops/actions/runs/20/job/30',1,1),
              (31,20,'build','queued','["self-hosted-macos"]','https://github.com/octo-org/gridops/actions/runs/20/job/31',2,2);
            "#,
        )
        .execute(&database)
        .await?;

        assert_eq!(assigned_queued_jobs(&database, "plain").await?, 1);
        assert_eq!(assigned_queued_jobs(&database, "mac").await?, 1);

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn each_pool_offers_the_platform_labels_of_the_platforms_it_runs_on() -> anyhow::Result<()>
    {
        let (directory, database) = ownership_fixture(
            r#"
            INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,provider,providers,labels,image,docker_image,tart_image,autoscaling_enabled,created_at,updated_at)
              VALUES
              ('docker',1,NULL,'docker','organization','docker','["docker"]','["macos"]','runner:latest','runner:latest','base',1,1,1),
              ('tart',1,NULL,'tart','organization','tart','["tart"]','[]','runner:latest','runner:latest','base',1,2,2),
              ('machines',1,NULL,'machines','organization','docker','["docker"]','[]','runner:latest','runner:latest','base',1,3,3);
            INSERT INTO pool_execution_profiles (id,pool_id,name,enabled,runtime_kind,execution_os,architecture,mode,image,cpu_millis,memory_mib,disk_bytes,placement,archived_at,created_at,updated_at)
              VALUES
              ('windows','machines','windows',1,'native_process','windows','x64','ephemeral','native',1000,1024,0,'machines',NULL,1,1),
              ('off','machines','off',0,'docker','linux','arm64','ephemeral','runner',1000,1024,0,'machines',NULL,1,1),
              ('gone','machines','gone',1,'tart_vm','macos','arm64','ephemeral','base',1000,1024,0,'machines',5,1,1);
            "#,
        )
        .await?;

        let labels = pool_platform_labels(&database).await?;
        let host = runner_arch_label().to_lowercase();
        let mut docker = vec![
            "linux".to_owned(),
            host.clone(),
            LINUX_RUNNER_LABEL.to_owned(),
        ];
        docker.sort();
        // A configured `macos` label offers no macOS platform.
        assert_eq!(labels.get("docker"), Some(&docker));
        assert_eq!(
            labels.get("tart"),
            Some(&vec![
                "arm64".to_owned(),
                "macos".to_owned(),
                MACOS_RUNNER_LABEL.to_owned()
            ])
        );
        // Runs on machines only, so the built-in host's Linux is not offered;
        // disabled and archived platforms are not offered either.
        assert_eq!(
            labels.get("machines"),
            Some(&vec!["windows".to_owned(), "x64".to_owned()])
        );

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn jobs_left_queued_in_a_finished_run_dont_count() -> anyhow::Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-finished-run-queue-test-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        // Run 21 finished but GitHub still lists its old job 32 as queued.
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1);
            INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,provider,providers,labels,image,docker_image,autoscaling_enabled,created_at,updated_at)
              VALUES ('linux',1,10,'linux','repository','docker','["docker"]','["linux"]','runner:latest','runner:latest',1,1,1);
            INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at)
              VALUES ('linux',10,1);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES
              (20,10,'CI',2,'push','queued','abc123','https://github.com/octo-org/gridops/actions/runs/20',1,1,1,1),
              (21,10,'Release',1,'push','completed','abc122','https://github.com/octo-org/gridops/actions/runs/21',1,1,1,1);
            INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES
              (30,20,'test','queued','["self-hosted","linux"]','https://github.com/octo-org/gridops/actions/runs/20/job/30',9999999999999,9999999999999),
              (32,21,'publish','queued','["self-hosted","linux"]','https://github.com/octo-org/gridops/actions/runs/21/job/32',1,1);
            "#,
        )
        .execute(&database)
        .await?;

        let jobs = queued_job_assignments(&database, "linux").await?;
        assert_eq!(jobs.len(), 1);
        assert!(!jobs[0].prioritised);
        assert_eq!(assigned_queued_jobs(&database, "linux").await?, 1);
        let capacities = repository_capacities(&database, "linux").await?;
        assert_eq!(capacities.len(), 1);
        assert_eq!((capacities[0].queued, capacities[0].prioritised), (1, 0));

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// A database with one organization, repositories 10 and 11, and `extra`
    /// (pools, runners and jobs) applied on top.
    async fn ownership_fixture(extra: &str) -> anyhow::Result<(std::path::PathBuf, SqlitePool)> {
        let directory = std::env::temp_dir().join(format!(
            "gridops-capacity-ownership-test-{}",
            uuid::Uuid::new_v4()
        ));
        let database = connect_database_path(&directory.join("gridops.sqlite")).await?;
        sqlx::raw_sql(
            r#"
            INSERT INTO installations (id,account_id,account_login,account_type,target_type,repository_selection,created_at,updated_at)
              VALUES (1,1,'octo-org','Organization','Organization','all',1,1);
            INSERT INTO repositories (id,installation_id,owner,name,full_name,private,default_branch,html_url,last_synced_at,created_at,updated_at)
              VALUES
              (10,1,'octo-org','gridops','octo-org/gridops',0,'master','https://github.com/octo-org/gridops',1,1,1),
              (11,1,'octo-org','other','octo-org/other',0,'master','https://github.com/octo-org/other',1,1,1);
            INSERT INTO workflow_runs (id,repository_id,workflow_name,run_number,event,status,head_sha,html_url,github_created_at,github_updated_at,created_at,updated_at)
              VALUES (20,10,'CI',1,'push','queued','abc123','https://github.com/octo-org/gridops/actions/runs/20',1,1,1,1);
            "#,
        )
        .execute(&database)
        .await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(extra.to_owned()))
            .execute(&database)
            .await?;
        Ok((directory, database))
    }

    /// An organization pool carrying the `shared` label.
    fn org_pool(id: &str, created_at: i64, max_count: i64) -> String {
        format!(
            "INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,max_count,autoscaling_enabled,created_at,updated_at)
              VALUES ('{id}',1,NULL,'{id}','organization','[\"shared\"]','runner:latest',{max_count},1,{created_at},{created_at});"
        )
    }

    /// A repository pool on repository 10 carrying the `shared` label.
    fn repository_pool(id: &str, created_at: i64, max_count: i64) -> String {
        format!(
            "INSERT INTO runner_pools (id,installation_id,repository_id,name,scope,labels,image,max_count,autoscaling_enabled,created_at,updated_at)
              VALUES ('{id}',1,10,'{id}','repository','[\"shared\"]','runner:latest',{max_count},1,{created_at},{created_at});
            INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at) VALUES ('{id}',10,1);"
        )
    }

    /// `count` runners in the pool with the given status and busy flag.
    fn pool_runners(pool: &str, tag: &str, count: usize, status: &str, busy: bool) -> String {
        (0..count)
            .map(|index| {
                format!(
                    "INSERT INTO runners (id,pool_id,target_repository_id,name,status,busy,created_at,updated_at)
                      VALUES ('{pool}-{tag}-{index}','{pool}',10,'{pool}-{tag}-{index}','{status}',{},1,1);",
                    i64::from(busy)
                )
            })
            .collect::<Vec<_>>()
            .concat()
    }

    /// Queued `shared` jobs on repository 10 with the given ids; each is queued
    /// at its id, so lower ids are older.
    fn shared_jobs(ids: std::ops::Range<i64>) -> String {
        ids.map(|id| {
                format!(
                    "INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
                      VALUES ({id},20,'job-{id}','queued','[\"self-hosted\",\"shared\"]','https://github.com/octo-org/gridops/actions/runs/20/job/{id}',{id},{id});"
                )
            })
            .collect::<Vec<_>>()
            .concat()
    }

    /// The queue times of the jobs the pool owns, checking that the count and
    /// the per-job list agree.
    async fn owned(database: &SqlitePool, pool_id: &str) -> anyhow::Result<Vec<i64>> {
        let jobs = queued_job_assignments(database, pool_id).await?;
        let count = assigned_queued_jobs(database, pool_id).await?;
        assert_eq!(
            i64::try_from(jobs.len())?,
            count,
            "{pool_id}: assignment list and count disagree"
        );
        Ok(jobs.into_iter().map(|job| job.queued_at).collect())
    }

    #[tokio::test]
    async fn a_saturated_pool_passes_queued_jobs_to_the_next_compatible_pool() -> anyhow::Result<()>
    {
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}",
            org_pool("older", 1, 2),
            org_pool("newer", 2, 10),
            pool_runners("older", "busy", 2, "busy", true),
            shared_jobs(100..103),
        ))
        .await?;

        assert_eq!(owned(&database, "older").await?, Vec::<i64>::new());
        assert_eq!(owned(&database, "newer").await?, vec![100, 101, 102]);
        // The same state gives the same owners.
        assert_eq!(owned(&database, "newer").await?, vec![100, 101, 102]);

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn ownership_names_each_jobs_pool_and_whether_it_outgrew_every_pool() -> anyhow::Result<()>
    {
        // 'older' is full, so 100-101 go to 'newer' (room 2 of max 2); 102 finds
        // both full and stays with the first candidate; 'gone' needs a label no
        // pool has and has no owner at all.
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}{}",
            org_pool("older", 1, 1),
            org_pool("newer", 2, 2),
            pool_runners("older", "busy", 1, "busy", true),
            shared_jobs(100..103),
            "INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES (200,20,'gone','queued','[\"self-hosted\",\"no-such-label\"]','https://github.com/octo-org/gridops/actions/runs/20/job/200',1,1);",
        ))
        .await?;

        let owners = queued_job_ownership(&database).await?;
        let summary = owners
            .iter()
            .map(|owner| (owner.job_id, owner.pool_id.as_str(), owner.over_capacity))
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            vec![
                (100, "newer", false),
                (101, "newer", false),
                (102, "older", true)
            ]
        );
        // The per-pool counts come from the same ownership.
        assert_eq!(owned(&database, "newer").await?, vec![100, 101]);
        assert_eq!(owned(&database, "older").await?, vec![102]);

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn a_pool_with_room_keeps_its_queued_jobs() -> anyhow::Result<()> {
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}",
            org_pool("older", 1, 10),
            org_pool("newer", 2, 10),
            pool_runners("older", "busy", 2, "busy", true),
            shared_jobs(100..103),
        ))
        .await?;

        assert_eq!(owned(&database, "older").await?, vec![100, 101, 102]);
        assert_eq!(owned(&database, "newer").await?, Vec::<i64>::new());

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn a_partly_busy_pool_keeps_only_the_oldest_jobs_it_has_room_for() -> anyhow::Result<()> {
        // Room is max 4 less 2 busy: the idle runner can take a job and the
        // pool can start one more.
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}{}",
            org_pool("older", 1, 4),
            org_pool("newer", 2, 10),
            pool_runners("older", "busy", 2, "busy", true),
            pool_runners("older", "idle", 1, "idle", false),
            shared_jobs(100..105),
        ))
        .await?;

        assert_eq!(owned(&database, "older").await?, vec![100, 101]);
        assert_eq!(owned(&database, "newer").await?, vec![102, 103, 104]);

        // A newer job never takes room from an older one, so the older jobs
        // keep their owners as the queue grows.
        sqlx::raw_sql(sqlx::AssertSqlSafe(shared_jobs(105..106)))
            .execute(&database)
            .await?;
        assert_eq!(owned(&database, "older").await?, vec![100, 101]);
        assert_eq!(owned(&database, "newer").await?, vec![102, 103, 104, 105]);

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn when_every_compatible_pool_is_full_the_first_pool_keeps_the_jobs() -> anyhow::Result<()>
    {
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}{}",
            org_pool("older", 1, 2),
            org_pool("newer", 2, 1),
            pool_runners("older", "busy", 2, "busy", true),
            pool_runners("newer", "busy", 1, "busy", true),
            shared_jobs(100..103),
        ))
        .await?;

        assert_eq!(owned(&database, "older").await?, vec![100, 101, 102]);
        assert_eq!(owned(&database, "newer").await?, Vec::<i64>::new());

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn repository_pools_go_first_and_pass_overflow_to_organization_pools()
    -> anyhow::Result<()> {
        // The organization pool is older, but the repository pool still
        // outranks it while it has room.
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}",
            org_pool("org", 0, 10),
            repository_pool("repo", 5, 3),
            pool_runners("repo", "busy", 1, "busy", true),
            shared_jobs(100..104),
        ))
        .await?;

        assert_eq!(owned(&database, "repo").await?, vec![100, 101]);
        assert_eq!(owned(&database, "org").await?, vec![102, 103]);

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn idle_runners_are_room_and_finished_runners_are_ignored() -> anyhow::Result<()> {
        // Max 3: one busy runner takes room; idle runners don't; runners that
        // were deleted or failed, even if last seen busy, don't either.
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}{}{}{}",
            org_pool("older", 1, 3),
            org_pool("newer", 2, 10),
            pool_runners("older", "busy", 1, "busy", true),
            pool_runners("older", "idle", 2, "online", false),
            pool_runners("older", "deleted", 2, "deleted", true),
            pool_runners("older", "failed", 1, "failed", true),
            shared_jobs(100..104),
        ))
        .await?;
        sqlx::raw_sql(
            "INSERT INTO runners (id,pool_id,target_repository_id,name,status,busy,deleted_at,created_at,updated_at)
              VALUES ('older-gone','older',10,'older-gone','busy',1,5,1,1);",
        )
        .execute(&database)
        .await?;

        assert_eq!(owned(&database, "older").await?, vec![100, 101]);
        assert_eq!(owned(&database, "newer").await?, vec![102, 103]);

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn waiting_jobs_are_served_by_one_pool_that_can_start_a_runner() -> anyhow::Result<()> {
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}",
            org_pool("tripped", 1, 10),
            org_pool("open", 2, 10),
            shared_jobs(100..101),
            // Waiting, pending and requested jobs are waiting to start too.
            "INSERT INTO workflow_jobs (id,run_id,name,status,labels,html_url,created_at,updated_at)
              VALUES (101,20,'approval','waiting','[\"shared\"]','https://github.com/octo-org/gridops/actions/runs/20/job/101',2,2),
              (102,20,'later','requested','[\"shared\"]','https://github.com/octo-org/gridops/actions/runs/20/job/102',3,3),
              (103,20,'running','in_progress','[\"shared\"]','https://github.com/octo-org/gridops/actions/runs/20/job/103',4,4);"
        ))
        .await?;
        sqlx::raw_sql("UPDATE runner_pools SET provision_circuit_open=1 WHERE id='tripped';")
            .execute(&database)
            .await?;

        // The pool with an open circuit is passed over for display, and
        // scaling still sees only the plainly queued job, as it always did.
        let serving = waiting_job_ownership(&database).await?;
        assert_eq!(serving.len(), 3);
        assert!(
            serving.iter().all(|owner| owner.pool_id == "open"),
            "{serving:?}"
        );
        assert_eq!(assigned_queued_jobs(&database, "tripped").await?, 1);
        assert_eq!(assigned_queued_jobs(&database, "open").await?, 0);

        // With the only other pool tripped too, nobody serves the jobs.
        sqlx::raw_sql("UPDATE runner_pools SET provision_circuit_open=1 WHERE id='open';")
            .execute(&database)
            .await?;
        assert!(waiting_job_ownership(&database).await?.is_empty());

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn paused_and_manual_pools_never_own_queued_jobs() -> anyhow::Result<()> {
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}{}{}",
            org_pool("older", 1, 1),
            org_pool("paused", 2, 10),
            org_pool("manual", 3, 10),
            org_pool("newest", 4, 10),
            pool_runners("older", "busy", 1, "busy", true),
            shared_jobs(100..102),
        ))
        .await?;
        sqlx::raw_sql(
            "UPDATE runner_pools SET paused=1 WHERE id='paused';
             UPDATE runner_pools SET autoscaling_enabled=0 WHERE id='manual';",
        )
        .execute(&database)
        .await?;

        assert_eq!(owned(&database, "older").await?, Vec::<i64>::new());
        assert_eq!(owned(&database, "paused").await?, Vec::<i64>::new());
        assert_eq!(owned(&database, "manual").await?, Vec::<i64>::new());
        assert_eq!(owned(&database, "newest").await?, vec![100, 101]);

        // With the only other autoscaling pool paused too, the full pool keeps
        // the jobs rather than handing them to a pool that won't scale.
        sqlx::raw_sql("UPDATE runner_pools SET paused=1 WHERE id='newest';")
            .execute(&database)
            .await?;
        assert_eq!(owned(&database, "older").await?, vec![100, 101]);
        for pool in ["paused", "manual", "newest"] {
            assert_eq!(owned(&database, pool).await?, Vec::<i64>::new());
        }

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn every_queued_job_has_exactly_one_owner() -> anyhow::Result<()> {
        // Pools created at the same time are ordered by id.
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}{}{}{}",
            org_pool("pool-c", 1, 10),
            org_pool("pool-a", 1, 2),
            org_pool("pool-b", 1, 3),
            pool_runners("pool-a", "busy", 1, "busy", true),
            pool_runners("pool-b", "busy", 1, "busy", true),
            pool_runners("pool-b", "idle", 1, "idle", false),
            shared_jobs(100..106),
        ))
        .await?;

        let a = owned(&database, "pool-a").await?;
        let b = owned(&database, "pool-b").await?;
        let c = owned(&database, "pool-c").await?;
        assert_eq!(a, vec![100]);
        assert_eq!(b, vec![101, 102]);
        assert_eq!(c, vec![103, 104, 105]);
        let mut all = [a, b, c].concat();
        all.sort_unstable();
        assert_eq!(all, (100..106).collect::<Vec<_>>());

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[tokio::test]
    async fn repository_capacity_counts_the_jobs_the_pool_owns() -> anyhow::Result<()> {
        // A full repository pool passes its jobs on; the next repository pool
        // must then aim its runners at the repository those jobs are in.
        let (directory, database) = ownership_fixture(&format!(
            "{}{}{}{}",
            repository_pool("older", 1, 1),
            repository_pool("newer", 2, 10),
            pool_runners("older", "busy", 1, "busy", true),
            shared_jobs(100..102),
        ))
        .await?;
        sqlx::raw_sql(
            "INSERT INTO runner_pool_repositories (pool_id,repository_id,created_at) VALUES ('newer',11,0);",
        )
        .execute(&database)
        .await?;

        let older = repository_capacities(&database, "older").await?;
        assert_eq!(older.iter().map(|capacity| capacity.queued).sum::<i64>(), 0);
        let newer = repository_capacities(&database, "newer").await?;
        let queued = |repository_id: i64| {
            newer
                .iter()
                .find(|capacity| capacity.repository_id == repository_id)
                .map(|capacity| capacity.queued)
        };
        assert_eq!((queued(10), queued(11)), (Some(2), Some(0)));
        let target = next_runner_repository(&database, "newer", 1)
            .await?
            .map(|capacity| capacity.repository_id);
        assert_eq!(target, Some(10));

        database.close().await;
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
