use anyhow::{Context as _, Result};
use gridops_core::{BitbucketClient, Config, GitHubClient, Vault};
use reqwest::Url;
use secrecy::ExposeSecret;
use sqlx::SqlitePool;

pub const GITHUB_CLIENT_ID: &str = "github.client_id";
pub const GITHUB_CLIENT_SECRET: &str = "github.client_secret";
pub const GITHUB_APP_ID: &str = "github.app_id";
pub const GITHUB_APP_PRIVATE_KEY: &str = "github.app_private_key";
pub const GITHUB_APP_SLUG: &str = "github.app_slug";
pub const GITHUB_APP_SETTINGS_URL: &str = "github.app_settings_url";
pub const GITHUB_WEBHOOK_SECRET: &str = "github.webhook_secret";

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub database: SqlitePool,
    pub vault: Vault,
    pub github: GitHubClient,
    pub bitbucket: BitbucketClient,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(
        config: Config,
        database: SqlitePool,
        vault: Vault,
        github: GitHubClient,
        bitbucket: BitbucketClient,
    ) -> Result<Self> {
        Ok(Self {
            config,
            database,
            vault,
            github,
            bitbucket,
            http: reqwest::Client::builder()
                .user_agent("GridOps/0.1")
                .build()?,
        })
    }

    pub fn manager_url(&self, path: &str) -> Result<Url> {
        Ok(self
            .config
            .manager_url()
            .join(path.trim_start_matches('/'))?)
    }

    pub fn manager_token(&self) -> Option<&str> {
        self.config.manager_token().map(ExposeSecret::expose_secret)
    }

    pub async fn runtime_secret(&self, key: &str) -> Result<Option<String>> {
        let sealed =
            sqlx::query_scalar::<_, String>("SELECT value FROM runtime_secrets WHERE key=?")
                .bind(key)
                .fetch_optional(&self.database)
                .await?;
        sealed
            .map(|value| {
                self.vault
                    .open(&value)
                    .context("could not decrypt runtime secret")
            })
            .transpose()
    }

    pub async fn github_oauth_credentials(&self) -> Result<Option<(String, String)>> {
        let client_id = match self.runtime_secret(GITHUB_CLIENT_ID).await? {
            Some(value) => Some(value),
            None => self.config.github_client_id().map(ToOwned::to_owned),
        };
        let client_secret = match self.runtime_secret(GITHUB_CLIENT_SECRET).await? {
            Some(value) => Some(value),
            None => self
                .config
                .github_client_secret()
                .map(|value| value.expose_secret().to_owned()),
        };
        Ok(client_id.zip(client_secret))
    }

    pub async fn github_app_credentials(&self) -> Result<Option<(String, String)>> {
        let app_id = match self.runtime_secret(GITHUB_APP_ID).await? {
            Some(value) => Some(value),
            None => self.config.github_app_id().map(ToOwned::to_owned),
        };
        let private_key = match self.runtime_secret(GITHUB_APP_PRIVATE_KEY).await? {
            Some(value) => Some(value),
            None => self
                .config
                .github_app_private_key()
                .map(|value| value.expose_secret().to_owned()),
        };
        Ok(app_id.zip(private_key))
    }

    pub async fn github_app_slug(&self) -> Result<String> {
        Ok(self
            .runtime_secret(GITHUB_APP_SLUG)
            .await?
            .unwrap_or_else(|| self.config.github_app_slug().to_owned()))
    }

    /// The App's settings page. Stored once GitHub reports the App's owner,
    /// since an organization's App lives under that organization.
    pub async fn github_app_settings_url(&self) -> Result<String> {
        match self.runtime_secret(GITHUB_APP_SETTINGS_URL).await? {
            Some(url) => Ok(url),
            None => Ok(format!(
                "https://github.com/settings/apps/{}",
                self.github_app_slug().await?
            )),
        }
    }

    /// Stores the App's slug and settings page as GitHub now has them, so
    /// links follow a rename or a move to another owner. Returns the slug
    /// when either changed.
    pub async fn refresh_github_app(&self) -> Result<Option<String>> {
        let Some((app_id, private_key)) = self.github_app_credentials().await? else {
            return Ok(None);
        };
        let app = self
            .github
            .app_with_credentials(&app_id, &private_key)
            .await?;
        let settings_url = app.settings_url();
        if app.slug == self.github_app_slug().await?
            && settings_url == self.github_app_settings_url().await?
        {
            return Ok(None);
        }
        let now = gridops_core::now_millis();
        let mut transaction = self.database.begin().await?;
        for (key, value) in [
            (GITHUB_APP_SLUG, app.slug.as_str()),
            (GITHUB_APP_SETTINGS_URL, settings_url.as_str()),
        ] {
            sqlx::query(
                r#"INSERT INTO runtime_secrets (key,value,updated_by,updated_at) VALUES (?,?,NULL,?)
                   ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_by=NULL,updated_at=excluded.updated_at"#,
            )
            .bind(key)
            .bind(self.vault.seal(value)?)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(Some(app.slug))
    }

    /// Refreshes the App's slug and owner, logging rather than failing: they
    /// only build links, so a GitHub hiccup must not block anything else.
    pub async fn follow_github_app_rename(&self) {
        match self.refresh_github_app().await {
            Ok(Some(slug)) => tracing::info!(slug = %slug, "GitHub App was renamed or moved"),
            Ok(None) => {}
            Err(error) => tracing::warn!(error = ?error, "could not refresh the GitHub App's name"),
        }
    }

    pub async fn github_webhook_secret(&self) -> Result<Option<String>> {
        match self.runtime_secret(GITHUB_WEBHOOK_SECRET).await? {
            Some(value) => Ok(Some(value)),
            None => Ok(self
                .config
                .github_webhook_secret()
                .map(|value| value.expose_secret().to_owned())),
        }
    }

    pub async fn installation_token(&self, installation_id: i64) -> Result<Option<String>> {
        let Some((app_id, private_key)) = self.github_app_credentials().await? else {
            return Ok(None);
        };
        self.github
            .installation_token_with_credentials(installation_id, &app_id, &private_key)
            .await
            .map(Some)
    }

    pub async fn validate_api(&self) -> Result<()> {
        self.github_oauth_credentials()
            .await?
            .context("GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET are required")?;
        Ok(())
    }
}
