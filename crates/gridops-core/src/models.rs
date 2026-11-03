use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewer {
    pub id: String,
    pub github_id: i64,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub role: String,
    pub alerts: Alerts,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alerts {
    pub failed_runners: i64,
    pub failed_webhooks: i64,
    pub queued_jobs: i64,
    pub deferred_runner_cleanup: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationState {
    #[serde(rename = "githubOAuth")]
    pub github_oauth: bool,
    pub github_app_control: bool,
    pub webhook_active: bool,
    pub webhook_verification: bool,
    pub secure_storage: bool,
    pub runner_manager: bool,
    pub installation_tokens: bool,
    pub callback_url: String,
    pub webhook_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRunnerPool {
    pub installation_id: i64,
    pub repository_id: Option<i64>,
    #[serde(default)]
    pub repository_ids: Vec<i64>,
    #[serde(default)]
    pub bitbucket_connection_ids: Vec<String>,
    pub name: String,
    pub scope: String,
    pub mode: String,
    pub provider: String,
    #[serde(default)]
    pub providers: Vec<String>,
    pub labels: Vec<String>,
    pub image: String,
    #[serde(default)]
    pub docker_image: String,
    #[serde(default)]
    pub tart_image: String,
    /// How the macOS agent executes this pool's runners: "vm" clones a Tart VM
    /// per job, "native" runs the runner on the agent host. Defaulted so
    /// existing clients that never send it keep getting VMs.
    #[serde(default = "default_macos_runtime")]
    pub macos_runtime: String,
    pub desired_count: i64,
    pub min_count: i64,
    pub max_count: i64,
    pub autoscaling_enabled: bool,
    pub queue_scale_factor: i64,
    pub idle_timeout_minutes: i64,
    pub cpu_limit: f64,
    pub memory_limit_mb: i64,
    pub runner_group_id: i64,
}

pub fn default_macos_runtime() -> String {
    "vm".into()
}

fn selected_macos_runtime(macos_runtime: &str) -> String {
    if macos_runtime.trim().is_empty() {
        default_macos_runtime()
    } else {
        macos_runtime.trim().to_owned()
    }
}

/// Native execution is a property of the macOS agent, so a pool asking for it
/// without the Tart provider selected would silently never get a native runner.
fn validate_macos_runtime(macos_runtime: &str, providers: &[String]) -> Result<(), String> {
    let runtime = selected_macos_runtime(macos_runtime);
    if !matches!(runtime.as_str(), "vm" | "native") {
        return Err("macOS runtime must be vm or native.".into());
    }
    if runtime == "native"
        && !providers
            .iter()
            .any(|provider| provider.eq_ignore_ascii_case("tart"))
    {
        return Err("Native macOS execution requires the macOS runner provider.".into());
    }
    Ok(())
}

impl CreateRunnerPool {
    pub fn selected_macos_runtime(&self) -> String {
        selected_macos_runtime(&self.macos_runtime)
    }

    pub fn selected_providers(&self) -> Vec<String> {
        selected_providers(&self.provider, &self.providers)
    }

    /// The Docker image to store: empty when the pool does not use Docker.
    pub fn selected_docker_image(&self) -> String {
        selected_image(
            "docker",
            &self.provider,
            &self.providers,
            &self.image,
            &self.docker_image,
        )
    }

    /// The Tart base VM to store: empty when the pool does not use Tart.
    pub fn selected_tart_image(&self) -> String {
        selected_image(
            "tart",
            &self.provider,
            &self.providers,
            &self.image,
            &self.tart_image,
        )
    }

    pub fn selected_repository_ids(&self) -> Vec<i64> {
        let mut repositories = self.repository_ids.clone();
        if let Some(repository_id) = self.repository_id {
            repositories.push(repository_id);
        }
        repositories.sort_unstable();
        repositories.dedup();
        repositories
    }

    pub fn validate(&self) -> Result<(), String> {
        let repositories = self.selected_repository_ids();
        if self.scope == "repository" && repositories.is_empty() {
            return Err("Repository scope requires at least one repository.".into());
        }
        if self.scope == "organization" && !repositories.is_empty() {
            return Err(
                "Organization pools use runner-group access instead of repository assignments."
                    .into(),
            );
        }
        validate_bitbucket_connections(
            &self.bitbucket_connection_ids,
            &self.providers,
            &self.provider,
            &self.labels,
            self.max_count,
        )?;
        validate_macos_runtime(&self.macos_runtime, &self.selected_providers())?;
        validate_primary_image(
            &self.provider,
            &self.image,
            &self.docker_image,
            &self.tart_image,
        )?;
        if repositories.len() > 1_000
            || repositories.iter().any(|repository_id| *repository_id <= 0)
        {
            return Err("Repository assignments are invalid.".into());
        }
        if self.scope == "repository"
            && i64::try_from(repositories.len()).unwrap_or(i64::MAX) > self.max_count
        {
            return Err("Repository count cannot exceed maximum runner capacity.".into());
        }
        if !matches!(self.scope.as_str(), "repository" | "organization") {
            return Err("Runner pool scope is invalid.".into());
        }
        validate_pool_configuration(
            &self.name,
            &self.mode,
            &self.provider,
            &self.providers,
            &self.labels,
            &self.image,
            &self.docker_image,
            &self.tart_image,
            &self.selected_macos_runtime(),
            self.desired_count,
            self.min_count,
            self.max_count,
            self.queue_scale_factor,
            self.idle_timeout_minutes,
            self.cpu_limit,
            self.memory_limit_mb,
            self.runner_group_id,
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRunnerPool {
    #[serde(default)]
    pub repository_ids: Option<Vec<i64>>,
    #[serde(default)]
    pub bitbucket_connection_ids: Option<Vec<String>>,
    pub name: String,
    pub mode: String,
    pub provider: String,
    #[serde(default)]
    pub providers: Vec<String>,
    pub labels: Vec<String>,
    pub image: String,
    #[serde(default)]
    pub docker_image: String,
    #[serde(default)]
    pub tart_image: String,
    /// How the macOS agent executes this pool's runners: "vm" clones a Tart VM
    /// per job, "native" runs the runner on the agent host. Defaulted so
    /// existing clients that never send it keep getting VMs.
    #[serde(default = "default_macos_runtime")]
    pub macos_runtime: String,
    pub desired_count: i64,
    pub min_count: i64,
    pub max_count: i64,
    pub autoscaling_enabled: bool,
    pub queue_scale_factor: i64,
    pub idle_timeout_minutes: i64,
    pub cpu_limit: f64,
    pub memory_limit_mb: i64,
    pub runner_group_id: i64,
    /// Pool repositories whose queued jobs get the next runner first. Left
    /// out, the pool keeps the ones it has.
    #[serde(default)]
    pub priority_repository_ids: Option<Vec<i64>>,
    /// How long any job may wait before it counts as prioritised too.
    #[serde(default)]
    pub priority_wait_minutes: Option<i64>,
}

impl UpdateRunnerPool {
    pub fn selected_macos_runtime(&self) -> String {
        selected_macos_runtime(&self.macos_runtime)
    }

    pub fn selected_providers(&self) -> Vec<String> {
        selected_providers(&self.provider, &self.providers)
    }

    /// The Docker image to store: empty when the pool does not use Docker.
    pub fn selected_docker_image(&self) -> String {
        selected_image(
            "docker",
            &self.provider,
            &self.providers,
            &self.image,
            &self.docker_image,
        )
    }

    /// The Tart base VM to store: empty when the pool does not use Tart.
    pub fn selected_tart_image(&self) -> String {
        selected_image(
            "tart",
            &self.provider,
            &self.providers,
            &self.image,
            &self.tart_image,
        )
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.repository_ids.as_ref().is_some_and(|repositories| {
            repositories.is_empty()
                || repositories.len() > 1_000
                || repositories.iter().any(|repository_id| *repository_id <= 0)
                || {
                    let mut unique = repositories.clone();
                    unique.sort_unstable();
                    unique.dedup();
                    unique.len() != repositories.len()
                }
        }) {
            return Err("Choose between 1 and 1,000 unique repositories for the pool.".into());
        }
        if self
            .priority_wait_minutes
            .is_some_and(|minutes| !(1..=1440).contains(&minutes))
        {
            return Err("The priority wait must be between 1 and 1,440 minutes.".into());
        }
        if let Some(bitbucket_connection_ids) = &self.bitbucket_connection_ids {
            validate_bitbucket_connections(
                bitbucket_connection_ids,
                &self.providers,
                &self.provider,
                &self.labels,
                self.max_count,
            )?;
        }
        validate_macos_runtime(&self.macos_runtime, &self.selected_providers())?;
        validate_primary_image(
            &self.provider,
            &self.image,
            &self.docker_image,
            &self.tart_image,
        )?;
        if self.repository_ids.as_ref().is_some_and(|repositories| {
            i64::try_from(repositories.len()).unwrap_or(i64::MAX) > self.max_count
        }) {
            return Err("Repository count cannot exceed maximum runner capacity.".into());
        }
        validate_pool_configuration(
            &self.name,
            &self.mode,
            &self.provider,
            &self.providers,
            &self.labels,
            &self.image,
            &self.docker_image,
            &self.tart_image,
            &self.selected_macos_runtime(),
            self.desired_count,
            self.min_count,
            self.max_count,
            self.queue_scale_factor,
            self.idle_timeout_minutes,
            self.cpu_limit,
            self.memory_limit_mb,
            self.runner_group_id,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_pool_configuration(
    name: &str,
    mode: &str,
    provider: &str,
    providers: &[String],
    labels: &[String],
    image: &str,
    docker_image: &str,
    tart_image: &str,
    macos_runtime: &str,
    desired_count: i64,
    min_count: i64,
    max_count: i64,
    queue_scale_factor: i64,
    idle_timeout_minutes: i64,
    cpu_limit: f64,
    memory_limit_mb: i64,
    runner_group_id: i64,
) -> Result<(), String> {
    if name.len() < 2 || name.len() > 48 || !valid_pool_name(name) {
        return Err("Use 2-48 lowercase letters, numbers, and hyphens for the pool name.".into());
    }
    if !matches!(mode, "ephemeral" | "persistent") {
        return Err("Runner pool mode is invalid.".into());
    }
    let selected = selected_providers(provider, providers);
    if selected.is_empty()
        || selected.len() > 2
        || selected
            .iter()
            .any(|provider| !matches!(provider.as_str(), "docker" | "tart"))
    {
        return Err("Choose one or more supported runner providers.".into());
    }
    let mut unique = selected.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != selected.len() || (!providers.is_empty() && selected[0] != provider) {
        return Err(
            "Runner providers must be unique and the primary provider must be first.".into(),
        );
    }
    let includes_tart = selected.iter().any(|provider| provider == "tart");
    if includes_tart && mode != "ephemeral" {
        return Err("Pools that include Tart must be ephemeral.".into());
    }
    if min_count < 0 || desired_count < min_count || desired_count > max_count || max_count > 100 {
        return Err("Capacity must satisfy 0 <= minimum <= desired <= maximum <= 100.".into());
    }
    if !cpu_limit.is_finite() || cpu_limit <= 0.0 || memory_limit_mb <= 0 {
        return Err("Runner CPU and memory must be positive values.".into());
    }
    if includes_tart && cpu_limit.fract() != 0.0 {
        return Err("Tart runners require whole CPU cores.".into());
    }
    if !(1..=20).contains(&queue_scale_factor) || !(1..=1_440).contains(&idle_timeout_minutes) {
        return Err("Autoscaling settings are outside the supported range.".into());
    }
    if labels.len() > 20
        || labels.iter().any(|label| {
            label.is_empty()
                || label.len() > 64
                || label.contains([',', '\n', '\r'])
                || label.trim() != label
        })
    {
        return Err("Use at most 20 runner labels of 1-64 characters each.".into());
    }
    if labels
        .iter()
        .any(|label| crate::autoscaling::is_runner_system_label(label))
    {
        return Err("System runner labels are assigned by the selected provider.".into());
    }
    let selected_docker_image = selected_image("docker", provider, providers, image, docker_image);
    let selected_tart_image = selected_image("tart", provider, providers, image, tart_image);
    if selected.iter().any(|provider| provider == "docker") {
        // A provider added to a pool brings its own image: none is filled in
        // from a default (issue #103).
        if selected_docker_image.is_empty() {
            return Err(
                "Docker image (dockerImage) is required for a pool that uses Linux · Docker."
                    .into(),
            );
        }
        if !valid_image(&selected_docker_image) {
            return Err("Docker image must contain 1-300 non-padding characters.".into());
        }
    }
    if includes_tart {
        // Native execution runs on the agent host and clones no VM, so only
        // the VM runtime needs a base VM. One that is given is still checked.
        if selected_tart_image.is_empty() && macos_runtime != "native" {
            return Err(
                "Tart base VM (tartImage) is required for a pool that uses macOS · Tart virtual machines."
                    .into(),
            );
        }
        if !selected_tart_image.is_empty() && !valid_image(&selected_tart_image) {
            return Err("Tart base VM must contain 1-300 non-padding characters.".into());
        }
    }
    if runner_group_id <= 0 {
        return Err("Runner group ID must be positive.".into());
    }
    Ok(())
}

fn validate_bitbucket_connections(
    connections: &[String],
    providers: &[String],
    provider: &str,
    labels: &[String],
    max_count: i64,
) -> Result<(), String> {
    if connections.len() > 100
        || connections
            .iter()
            .any(|connection| uuid::Uuid::parse_str(connection).is_err() || connection.len() > 64)
    {
        return Err("Choose up to 100 valid Bitbucket workspace connections.".into());
    }
    let mut unique = connections.to_vec();
    unique.sort();
    unique.dedup();
    if unique.len() != connections.len() {
        return Err("Bitbucket workspace connections must be unique.".into());
    }
    if connections.is_empty() {
        return Ok(());
    }
    let selected = selected_providers(provider, providers);
    if !selected.iter().any(|item| item == "tart") {
        return Err(
            "Bitbucket macOS runners require the Tart provider in this first adapter release."
                .into(),
        );
    }
    if i64::try_from(connections.len()).unwrap_or(i64::MAX) > max_count {
        return Err("Bitbucket workspace count cannot exceed maximum runner capacity.".into());
    }
    if labels.iter().any(|label| {
        label.is_empty()
            || !label.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '.'
            })
    }) {
        return Err(
            "Bitbucket-compatible pool labels use lowercase letters, numbers, and dots.".into(),
        );
    }
    Ok(())
}

fn selected_providers(provider: &str, providers: &[String]) -> Vec<String> {
    if providers.is_empty() {
        vec![provider.to_owned()]
    } else {
        providers.to_vec()
    }
}

/// The image a pool stores for one provider (`kind`). A provider the pool does
/// not use gets none, whatever the request carries: the web form and a body
/// built from `/options` `.defaults` send both providers' images, and a stored
/// default would be cloned silently once that provider is added (issue #103).
/// For the primary provider, `image` stands in when the specific field is empty.
fn selected_image(
    kind: &str,
    provider: &str,
    providers: &[String],
    image: &str,
    configured: &str,
) -> String {
    if !selected_providers(provider, providers)
        .iter()
        .any(|selected| selected == kind)
    {
        String::new()
    } else if configured.is_empty() && provider == kind {
        image.to_owned()
    } else {
        configured.to_owned()
    }
}

/// A create or edit request names the primary provider's image twice, as
/// `image` and as `dockerImage` or `tartImage`, but only the provider-specific
/// field is stored when it is sent. Two different values would keep one
/// silently, for example the server's default Tart base VM instead of the one
/// in `image`, so they must agree (or the provider-specific field be left out).
fn validate_primary_image(
    provider: &str,
    image: &str,
    docker_image: &str,
    tart_image: &str,
) -> Result<(), String> {
    let (field, configured) = if provider == "tart" {
        ("tartImage", tart_image)
    } else {
        ("dockerImage", docker_image)
    };
    if configured.is_empty() || configured == image {
        Ok(())
    } else {
        Err(format!(
            "image and {field} name different images for the primary provider; send the same value in both, or leave {field} out."
        ))
    }
}

fn valid_image(image: &str) -> bool {
    image.trim() == image && !image.is_empty() && image.len() <= 300
}

fn valid_pool_name(value: &str) -> bool {
    !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> CreateRunnerPool {
        CreateRunnerPool {
            installation_id: 1,
            repository_id: Some(2),
            repository_ids: Vec::new(),
            bitbucket_connection_ids: Vec::new(),
            name: "linux-general".into(),
            scope: "repository".into(),
            mode: "ephemeral".into(),
            provider: "docker".into(),
            providers: vec!["docker".into()],
            labels: vec!["docker".into()],
            image: "ghcr.io/actions/actions-runner:latest".into(),
            docker_image: "ghcr.io/actions/actions-runner:latest".into(),
            tart_image: "gridops-macos-tahoe-base".into(),
            macos_runtime: "vm".into(),
            desired_count: 1,
            min_count: 0,
            max_count: 10,
            autoscaling_enabled: true,
            queue_scale_factor: 1,
            idle_timeout_minutes: 5,
            cpu_limit: 2.0,
            memory_limit_mb: 4096,
            runner_group_id: 1,
        }
    }

    #[test]
    fn validates_runner_pool_invariants() {
        assert!(pool().validate().is_ok());
        let mut invalid = pool();
        invalid.desired_count = 11;
        invalid.max_count = 10;
        assert!(invalid.validate().is_err());
        let mut invalid = pool();
        invalid.name = "Bad Pool".into();
        assert!(invalid.validate().is_err());

        let mut mixed = pool();
        mixed.providers = vec!["docker".into(), "tart".into()];
        assert!(mixed.validate().is_ok());
        mixed.mode = "persistent".into();
        assert!(mixed.validate().is_err());

        let mut invalid = pool();
        invalid.labels = vec!["bad,label".into()];
        assert!(invalid.validate().is_err());

        let mut invalid = pool();
        invalid.docker_image = " runner:latest".into();
        assert!(invalid.validate().is_err());

        let mut multi_repository = pool();
        multi_repository.repository_id = None;
        multi_repository.repository_ids = vec![2, 3];
        multi_repository.max_count = 1;
        assert!(multi_repository.validate().is_err());
        multi_repository.max_count = 2;
        assert!(multi_repository.validate().is_ok());
    }

    /// A create request names the primary provider's image twice: `image`
    /// and `tartImage` (or `dockerImage`). Only the provider-specific field is
    /// stored, so a request whose two values disagree would silently clone a
    /// VM the caller did not ask for (issue #97). It is refused instead.
    #[test]
    fn refuses_a_create_whose_image_and_primary_provider_image_disagree() {
        let mut tart = pool();
        tart.provider = "tart".into();
        tart.providers = vec!["tart".into()];
        tart.image = "acme-xcode-26".into();
        tart.tart_image = "acme-xcode-26".into();
        assert!(tart.validate().is_ok());
        assert_eq!(tart.selected_tart_image(), "acme-xcode-26");

        // Omitting tartImage keeps working: `image` is the Tart base VM.
        tart.tart_image = String::new();
        assert!(tart.validate().is_ok());
        assert_eq!(tart.selected_tart_image(), "acme-xcode-26");

        // The server default left in tartImage next to the VM the caller
        // meant in `image` is a contradiction, not a choice.
        tart.tart_image = "gridops-macos-tahoe-base".into();
        assert_eq!(
            tart.validate(),
            Err("image and tartImage name different images for the primary provider; send the same value in both, or leave tartImage out.".into())
        );

        let mut docker = pool();
        docker.image = "ghcr.io/acme/runner:2".into();
        assert_eq!(
            docker.validate(),
            Err("image and dockerImage name different images for the primary provider; send the same value in both, or leave dockerImage out.".into())
        );

        // A secondary provider's image is independent of `image`.
        let mut mixed = pool();
        mixed.providers = vec!["docker".into(), "tart".into()];
        mixed.tart_image = "acme-xcode-26".into();
        assert!(mixed.validate().is_ok());
        assert_eq!(mixed.selected_tart_image(), "acme-xcode-26");
    }

    #[test]
    fn validates_runner_pool_updates() {
        let original = pool();
        let update = UpdateRunnerPool {
            repository_ids: None,
            bitbucket_connection_ids: None,
            name: original.name,
            mode: original.mode,
            provider: original.provider,
            providers: original.providers,
            labels: original.labels,
            image: original.image,
            docker_image: original.docker_image,
            tart_image: original.tart_image,
            macos_runtime: original.macos_runtime,
            desired_count: original.desired_count,
            min_count: original.min_count,
            max_count: original.max_count,
            autoscaling_enabled: original.autoscaling_enabled,
            queue_scale_factor: original.queue_scale_factor,
            idle_timeout_minutes: original.idle_timeout_minutes,
            cpu_limit: original.cpu_limit,
            memory_limit_mb: original.memory_limit_mb,
            runner_group_id: original.runner_group_id,
            priority_repository_ids: None,
            priority_wait_minutes: None,
        };
        assert!(update.validate().is_ok());

        let mut undersized = update;
        undersized.repository_ids = Some(vec![2, 3]);
        undersized.max_count = 1;
        assert!(undersized.validate().is_err());

        let mut bitbucket = pool();
        bitbucket.providers = vec!["docker".into(), "tart".into()];
        bitbucket.bitbucket_connection_ids = vec!["a39d4e94-a0cc-4e71-9548-741f1fd54820".into()];
        bitbucket.labels = vec!["release.ios".into()];
        assert!(bitbucket.validate().is_ok());
        bitbucket.providers = vec!["docker".into()];
        assert!(bitbucket.validate().is_err());
    }

    fn update(original: CreateRunnerPool) -> UpdateRunnerPool {
        UpdateRunnerPool {
            repository_ids: None,
            bitbucket_connection_ids: None,
            name: original.name,
            mode: original.mode,
            provider: original.provider,
            providers: original.providers,
            labels: original.labels,
            image: original.image,
            docker_image: original.docker_image,
            tart_image: original.tart_image,
            macos_runtime: original.macos_runtime,
            desired_count: original.desired_count,
            min_count: original.min_count,
            max_count: original.max_count,
            autoscaling_enabled: original.autoscaling_enabled,
            queue_scale_factor: original.queue_scale_factor,
            idle_timeout_minutes: original.idle_timeout_minutes,
            cpu_limit: original.cpu_limit,
            memory_limit_mb: original.memory_limit_mb,
            runner_group_id: original.runner_group_id,
            priority_repository_ids: None,
            priority_wait_minutes: None,
        }
    }

    /// Issue #103: an image kept for a provider the pool does not use is
    /// cloned silently once that provider is added. Create and edit store no
    /// image for it, whatever the request carries (the form and `.defaults`
    /// send both).
    #[test]
    fn stores_no_image_for_a_provider_the_pool_does_not_use() {
        // pool() is Docker-only and carries the default Tart base VM.
        let docker = pool();
        assert!(docker.validate().is_ok());
        assert_eq!(
            docker.selected_docker_image(),
            "ghcr.io/actions/actions-runner:latest"
        );
        assert_eq!(docker.selected_tart_image(), "");
        let docker_edit = update(pool());
        assert!(docker_edit.validate().is_ok());
        assert_eq!(docker_edit.selected_tart_image(), "");

        let mut tart = pool();
        tart.provider = "tart".into();
        tart.providers = vec!["tart".into()];
        tart.image = "acme-xcode-26".into();
        tart.tart_image = "acme-xcode-26".into();
        assert!(tart.validate().is_ok());
        assert_eq!(tart.selected_tart_image(), "acme-xcode-26");
        assert_eq!(tart.selected_docker_image(), "");
        // A legacy request naming only `provider` uses that provider alone.
        tart.providers = Vec::new();
        assert_eq!(tart.selected_docker_image(), "");
        assert_eq!(tart.selected_tart_image(), "acme-xcode-26");
        tart.providers = vec!["tart".into()];
        let tart_edit = update(tart);
        assert_eq!(tart_edit.selected_docker_image(), "");
        assert_eq!(tart_edit.selected_tart_image(), "acme-xcode-26");

        let mut mixed = update(pool());
        mixed.providers = vec!["docker".into(), "tart".into()];
        mixed.tart_image = "acme-xcode-26".into();
        assert!(mixed.validate().is_ok());
        assert_eq!(
            mixed.selected_docker_image(),
            "ghcr.io/actions/actions-runner:latest"
        );
        assert_eq!(mixed.selected_tart_image(), "acme-xcode-26");
    }

    /// Adding a provider to a pool needs that provider's image; it is never
    /// filled in from a default.
    #[test]
    fn requires_the_image_of_a_provider_the_pool_adds() {
        let mut add_tart = update(pool());
        add_tart.providers = vec!["docker".into(), "tart".into()];
        add_tart.tart_image = String::new();
        assert_eq!(
            add_tart.validate(),
            Err("Tart base VM (tartImage) is required for a pool that uses macOS · Tart virtual machines.".into())
        );
        // Native execution clones no VM, so it needs no base VM.
        add_tart.macos_runtime = "native".into();
        assert!(add_tart.validate().is_ok());
        assert_eq!(add_tart.selected_tart_image(), "");

        let mut create_mixed = pool();
        create_mixed.providers = vec!["docker".into(), "tart".into()];
        create_mixed.tart_image = String::new();
        assert_eq!(
            create_mixed.validate(),
            Err("Tart base VM (tartImage) is required for a pool that uses macOS · Tart virtual machines.".into())
        );

        let mut add_docker = update(pool());
        add_docker.provider = "tart".into();
        add_docker.providers = vec!["tart".into(), "docker".into()];
        add_docker.image = "acme-xcode-26".into();
        add_docker.tart_image = "acme-xcode-26".into();
        add_docker.docker_image = String::new();
        assert_eq!(
            add_docker.validate(),
            Err(
                "Docker image (dockerImage) is required for a pool that uses Linux · Docker."
                    .into()
            )
        );
        add_docker.docker_image = "ghcr.io/acme/runner:2".into();
        assert!(add_docker.validate().is_ok());

        // A value that is there but malformed keeps its own message.
        add_docker.docker_image = " runner:latest".into();
        assert_eq!(
            add_docker.validate(),
            Err("Docker image must contain 1-300 non-padding characters.".into())
        );
    }

    /// #99's agreement rule applies to an edit as it does to a create: two
    /// different values for the primary provider's image would keep one
    /// silently.
    #[test]
    fn refuses_an_edit_whose_image_and_primary_provider_image_disagree() {
        let mut tart = update(pool());
        tart.provider = "tart".into();
        tart.providers = vec!["tart".into()];
        tart.image = "acme-xcode-26".into();
        tart.tart_image = "acme-xcode-26".into();
        assert!(tart.validate().is_ok());
        tart.tart_image = String::new();
        assert!(tart.validate().is_ok());
        assert_eq!(tart.selected_tart_image(), "acme-xcode-26");
        tart.tart_image = "gridops-macos-tahoe-base".into();
        assert_eq!(
            tart.validate(),
            Err("image and tartImage name different images for the primary provider; send the same value in both, or leave tartImage out.".into())
        );

        let mut docker = update(pool());
        docker.image = "ghcr.io/acme/runner:2".into();
        assert_eq!(
            docker.validate(),
            Err("image and dockerImage name different images for the primary provider; send the same value in both, or leave dockerImage out.".into())
        );
    }
}
