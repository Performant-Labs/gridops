//! Pins the GitLab shared contract (#137) that wave-1 issues build against:
//! the platform and target shapes, the wire strings, the bootstrap material,
//! the client types and the job-status rules. Behaviour belongs to later
//! issues; these tests hold the shapes and the rules those issues share.

use std::collections::BTreeSet;

use anyhow::{Context as _, Result};
use gridops_core::fleet::{
    CiPlatform, CiTarget, ExternalId,
    capabilities::{SupportedRunner, TargetKind},
    domain::{BitbucketConnectionId, GitlabConnectionId, NonNilTargetUuid},
    protocol::agent::{AgentCommandEnvelope, CommandError, GitlabRunnerMaterial, RunnerMode},
};
use gridops_core::gitlab::{
    CreatedRunner, GITLAB_ACTIVE_JOB_STATUSES, GitLabCredentials, JobStatus, Scope, ScopeKind,
};
use secrecy::SecretString;
use serde_json::{Value, json};

const MATERIAL: &str = r#"{"kind":"gitlab_runner","token":"glrt-…","url":"https://gitlab.example.com","wait_timeout_seconds":600}"#;
const MATERIAL_DEBUG: &str = r#"GitlabRunner { token: "[redacted]", url: "https://gitlab.example.com", wait_timeout_seconds: 600 }"#;

fn connection() -> Result<GitlabConnectionId> {
    Ok(GitlabConnectionId::new("gl-connection")?)
}

fn project_target() -> Result<CiTarget> {
    Ok(CiTarget::GitlabProject {
        connection_id: connection()?,
        project_id: ExternalId::new(42)?,
    })
}

fn group_target() -> Result<CiTarget> {
    Ok(CiTarget::GitlabGroup {
        connection_id: connection()?,
        group_id: ExternalId::new(7)?,
    })
}

fn bitbucket_target() -> Result<CiTarget> {
    Ok(CiTarget::BitbucketWorkspace {
        connection_id: BitbucketConnectionId::new("bb-connection")?,
        workspace_uuid: NonNilTargetUuid::new(uuid::Uuid::parse_str(
            "11111111-1111-4111-8111-111111111111",
        )?)?,
    })
}

#[test]
fn gitlab_is_a_ci_platform_with_a_snake_case_name() -> Result<()> {
    assert_eq!(serde_json::to_value(CiPlatform::Gitlab)?, json!("gitlab"));
    assert_eq!(
        serde_json::from_value::<CiPlatform>(json!("gitlab"))?,
        CiPlatform::Gitlab
    );
    assert_eq!(
        [
            CiPlatform::Github.as_str(),
            CiPlatform::Bitbucket.as_str(),
            CiPlatform::Gitlab.as_str(),
        ],
        ["github", "bitbucket", "gitlab"]
    );
    Ok(())
}

#[test]
fn gitlab_connection_ids_are_validated_plain_strings() -> Result<()> {
    let id = connection()?;
    assert_eq!(id.as_str(), "gl-connection");
    assert_eq!(serde_json::to_value(&id)?, json!("gl-connection"));
    assert_eq!(
        serde_json::from_value::<GitlabConnectionId>(json!("gl-connection"))?,
        id
    );
    assert_eq!(String::from(id.clone()), "gl-connection");
    assert_eq!(
        GitlabConnectionId::try_from("gl-connection".to_owned())?,
        id
    );
    // The same validation as the Bitbucket id it sits beside.
    assert!(GitlabConnectionId::new("").is_err());
    assert!(serde_json::from_value::<GitlabConnectionId>(json!("")).is_err());
    Ok(())
}

#[test]
fn gitlab_targets_have_the_pinned_wire_shape_and_canonical_keys() -> Result<()> {
    let project = project_target()?;
    let group = group_target()?;
    let project_json =
        json!({"platform":"gitlab_project","connection_id":"gl-connection","project_id":42});
    let group_json =
        json!({"platform":"gitlab_group","connection_id":"gl-connection","group_id":7});
    assert_eq!(serde_json::to_value(&project)?, project_json);
    assert_eq!(serde_json::to_value(&group)?, group_json);
    assert_eq!(serde_json::from_value::<CiTarget>(project_json)?, project);
    assert_eq!(serde_json::from_value::<CiTarget>(group_json)?, group);
    // deny_unknown_fields still holds for the new variants, and the ids are
    // ExternalIds (non-zero).
    for invalid in [
        json!({"platform":"gitlab_project","connection_id":"gl-connection","project_id":42,"group_id":7}),
        json!({"platform":"gitlab_group","connection_id":"gl-connection","group_id":0}),
        json!({"platform":"gitlab_project","connection_id":"","project_id":42}),
    ] {
        assert!(
            serde_json::from_value::<CiTarget>(invalid.clone()).is_err(),
            "{invalid}"
        );
    }
    assert_eq!(project.canonical_key(), "gitlab:project:gl-connection:42");
    assert_eq!(group.canonical_key(), "gitlab:group:gl-connection:7");
    Ok(())
}

#[test]
fn gitlab_targets_are_gitlab_and_never_classified_as_bitbucket() -> Result<()> {
    for target in [project_target()?, group_target()?] {
        assert!(target.is_gitlab(), "{target:?}");
        assert!(!target.is_bitbucket(), "{target:?}");
        assert!(!target.is_github(), "{target:?}");
    }
    let bitbucket = bitbucket_target()?;
    assert!(bitbucket.is_bitbucket());
    assert!(!bitbucket.is_gitlab());
    let github = CiTarget::GithubRepository {
        installation_id: ExternalId::new(1)?,
        repository_id: ExternalId::new(10)?,
    };
    assert!(!github.is_gitlab());
    assert!(!github.is_bitbucket());
    Ok(())
}

#[test]
fn target_kinds_for_gitlab_use_the_same_bare_wire_strings() -> Result<()> {
    assert_eq!(
        serde_json::to_value(TargetKind::GitlabProject)?,
        json!("gitlab_project")
    );
    assert_eq!(
        serde_json::to_value(TargetKind::GitlabGroup)?,
        json!("gitlab_group")
    );
    assert_eq!(
        TargetKind::from(&project_target()?),
        TargetKind::GitlabProject
    );
    assert_eq!(TargetKind::from(&group_target()?), TargetKind::GitlabGroup);
    for kind in [TargetKind::GitlabProject, TargetKind::GitlabGroup] {
        assert!(kind.is_gitlab());
        assert!(!kind.is_bitbucket());
    }
    for kind in [
        TargetKind::GithubRepository,
        TargetKind::GithubOrganization,
        TargetKind::BitbucketWorkspace,
        TargetKind::BitbucketRepository,
    ] {
        assert!(!kind.is_gitlab(), "{kind:?}");
    }
    let advertised: SupportedRunner =
        serde_json::from_value(json!({"target_kind":"gitlab_project","mode":"ephemeral"}))?;
    assert_eq!(advertised.target_kind, TargetKind::GitlabProject);
    assert_eq!(advertised.mode, RunnerMode::Ephemeral);
    Ok(())
}

fn material() -> GitlabRunnerMaterial {
    GitlabRunnerMaterial::GitlabRunner {
        token: "glrt-…".to_owned(),
        url: "https://gitlab.example.com".to_owned(),
        wait_timeout_seconds: 600,
    }
}

#[test]
fn gitlab_runner_material_is_the_pinned_json_and_round_trips() -> Result<()> {
    let expected: Value = serde_json::from_str(MATERIAL)?;
    assert_eq!(serde_json::to_value(material())?, expected);
    assert_eq!(
        serde_json::from_str::<GitlabRunnerMaterial>(MATERIAL)?,
        material()
    );
    // Key order is not part of the contract.
    assert_eq!(
        serde_json::from_str::<GitlabRunnerMaterial>(
            r#"{"url":"https://gitlab.example.com","wait_timeout_seconds":600,"token":"glrt-…","kind":"gitlab_runner"}"#
        )?,
        material()
    );
    let GitlabRunnerMaterial::GitlabRunner {
        token,
        url,
        wait_timeout_seconds,
    } = material();
    assert_eq!(
        (token.as_str(), url.as_str(), wait_timeout_seconds),
        ("glrt-…", "https://gitlab.example.com", 600)
    );
    Ok(())
}

#[test]
fn gitlab_runner_material_refuses_extra_fields_and_other_kinds() {
    for payload in [
        r#"{"kind":"gitlab_runner","token":"t","url":"u","wait_timeout_seconds":1,"extra":1}"#,
        r#"{"extra":1,"kind":"gitlab_runner","token":"t","url":"u","wait_timeout_seconds":1}"#,
        r#"{"kind":"github_registration","token":"t","url":"u","wait_timeout_seconds":1}"#,
        r#"{"token":"t","url":"u","wait_timeout_seconds":1}"#,
        r#"{"kind":"gitlab_runner","token":"t","url":"u"}"#,
    ] {
        assert!(
            serde_json::from_str::<GitlabRunnerMaterial>(payload).is_err(),
            "accepted {payload}"
        );
    }
}

#[test]
fn gitlab_runner_material_debug_never_prints_the_token() {
    let secret = GitlabRunnerMaterial::GitlabRunner {
        token: "glrt-material-secret-value-0001".to_owned(),
        url: "https://gitlab.example.com".to_owned(),
        wait_timeout_seconds: 600,
    };
    assert!(!format!("{secret:?}").contains("glrt-material-secret-value-0001"));
    assert_eq!(format!("{:?}", material()), MATERIAL_DEBUG);
}

#[test]
fn client_credentials_and_created_runners_redact_their_secrets() -> Result<()> {
    let credentials = GitLabCredentials {
        base_url: url::Url::parse("https://gitlab.example.com")?,
        access_token: SecretString::from("glpat-credential-secret-0001".to_owned()),
    };
    let printed = format!("{credentials:?}");
    assert!(
        !printed.contains("glpat-credential-secret-0001"),
        "{printed}"
    );
    assert!(printed.contains("https://gitlab.example.com"), "{printed}");
    assert!(
        printed.contains(r#"access_token: "[redacted]""#),
        "{printed}"
    );
    let created = CreatedRunner {
        id: ExternalId::new(9)?,
        token: SecretString::from("glrt-created-secret-0001".to_owned()),
    };
    assert!(!format!("{created:?}").contains("glrt-created-secret-0001"));
    Ok(())
}

#[test]
fn scopes_name_their_kind_id_path_and_fleet_target() -> Result<()> {
    assert_eq!(serde_json::to_value(ScopeKind::Project)?, json!("project"));
    assert_eq!(serde_json::to_value(ScopeKind::Group)?, json!("group"));
    assert_eq!(
        [ScopeKind::Project.as_str(), ScopeKind::Group.as_str()],
        ["project", "group"]
    );
    let project = Scope::Project {
        id: ExternalId::new(42)?,
        path: "acme/tools/api".to_owned(),
    };
    let group = Scope::Group {
        id: ExternalId::new(7)?,
        path: "acme/tools".to_owned(),
    };
    assert_eq!(
        (project.kind(), project.id(), project.path()),
        (ScopeKind::Project, ExternalId::new(42)?, "acme/tools/api")
    );
    assert_eq!(
        (group.kind(), group.id(), group.path()),
        (ScopeKind::Group, ExternalId::new(7)?, "acme/tools")
    );
    assert_eq!(project.target(connection()?), project_target()?);
    assert_eq!(group.target(connection()?), group_target()?);
    Ok(())
}

const EVERY_STATUS: [(JobStatus, &str); 12] = [
    (JobStatus::Created, "created"),
    (JobStatus::WaitingForResource, "waiting_for_resource"),
    (JobStatus::Preparing, "preparing"),
    (JobStatus::Pending, "pending"),
    (JobStatus::Running, "running"),
    (JobStatus::Success, "success"),
    (JobStatus::Failed, "failed"),
    (JobStatus::Canceled, "canceled"),
    (JobStatus::Skipped, "skipped"),
    (JobStatus::Manual, "manual"),
    (JobStatus::Scheduled, "scheduled"),
    (JobStatus::Unknown, "unknown"),
];

#[test]
fn job_statuses_use_gitlab_spellings_and_absorb_unknown_values() -> Result<()> {
    for (status, wire) in EVERY_STATUS {
        assert_eq!(status.as_str(), wire, "{status:?}");
        assert_eq!(serde_json::to_value(status)?, json!(wire), "{status:?}");
        assert_eq!(
            serde_json::from_value::<JobStatus>(json!(wire))?,
            status,
            "{wire}"
        );
    }
    // A status GitLab adds later is stored as "unknown", not refused.
    assert_eq!(
        serde_json::from_value::<JobStatus>(json!("some_future_status"))?,
        JobStatus::Unknown
    );
    Ok(())
}

#[test]
fn only_finished_jobs_are_terminal_and_only_runner_seeking_jobs_are_active() {
    let terminal: BTreeSet<&str> = EVERY_STATUS
        .iter()
        .filter(|(status, _)| status.is_terminal())
        .map(|(_, wire)| *wire)
        .collect();
    assert_eq!(
        terminal,
        BTreeSet::from(["success", "failed", "canceled", "skipped"])
    );
    let active: BTreeSet<&str> = EVERY_STATUS
        .iter()
        .filter(|(status, _)| status.is_active())
        .map(|(_, wire)| *wire)
        .collect();
    assert_eq!(
        active,
        BTreeSet::from([
            "created",
            "waiting_for_resource",
            "preparing",
            "pending",
            "running"
        ])
    );
    for (status, _) in EVERY_STATUS {
        assert!(
            !(status.is_active() && status.is_terminal()),
            "{status:?} is both active and terminal"
        );
    }
    let constant: BTreeSet<&str> = GITLAB_ACTIVE_JOB_STATUSES.into_iter().collect();
    assert_eq!(constant, active, "GITLAB_ACTIVE_JOB_STATUSES");
}

/// Removes all whitespace, so the comparison holds the SQL's tokens, not its
/// formatting.
fn compact(sql: &str) -> String {
    sql.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn the_active_jobs_index_predicate_is_built_from_the_active_statuses() -> Result<()> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/0047_gitlab.sql"
    );
    let migration = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let index = migration
        .split(';')
        .find(|statement| statement.contains("gitlab_jobs_active_created_idx"))
        .context("0047 creates gitlab_jobs_active_created_idx")?;
    let statuses: Vec<String> = EVERY_STATUS
        .iter()
        .filter(|(status, _)| status.is_active())
        .map(|(status, _)| format!("'{}'", status.as_str()))
        .collect();
    let predicate = format!("IN({})", statuses.join(","));
    assert!(
        compact(index).contains(&predicate),
        "the index predicate must be {predicate}: {index}"
    );
    let constant: Vec<String> = GITLAB_ACTIVE_JOB_STATUSES
        .iter()
        .map(|status| format!("'{status}'"))
        .collect();
    assert_eq!(constant, statuses, "constant order matches the migration");
    Ok(())
}

fn id(n: u32) -> String {
    format!("{n:08x}-0000-4000-8000-000000000001")
}

/// The `fleet_agent_commands.rs` `prepare_environment` fixture, with the
/// target and the placement-shape fields as parameters.
fn prepare(target: &Value, runtime: &str, os: &str, architecture: &str, mode: &str) -> Value {
    json!({
        "protocol_version":1,"command_id":id(1),"operation_id":id(2),"idempotency_key":"prepare:fixture",
        "host_id":id(3),"host_epoch":1,"control_plane_incarnation":id(4),"authority_session_id":id(5),
        "scope":{"kind":"placement","placement_id":id(6),"backend_id":id(7),"generation":1},
        "expected_revision":1,"issued_at":"2026-10-09T12:00:00.000Z","deadline_at":"2026-10-09T12:20:00.000Z",
        "request_hash":"","operation":{
            "kind":"prepare_environment","execution_profile_id":id(8),"profile_revision":1,"workload":"ci_runner",
            "target":target,
            "runtime_kind":runtime,"execution_os":os,"architecture":architecture,"mode":mode,
            "image":"runner:fixture","labels":["self-hosted","Linux"],"configuration":{},
            "resource_request":{"cpu_millis":1000,"memory_mib":1024,"disk_bytes":1_073_741_824},
            "allocation_ids":[id(9),id(10)]
        }
    })
}

/// Parses the envelope (a parse failure is a test error, never a refusal),
/// then seals it, which runs the `PrepareEnvironment` rules.
fn seal(value: Value) -> Result<Result<AgentCommandEnvelope, CommandError>> {
    Ok(serde_json::from_value::<AgentCommandEnvelope>(value)?.seal_hash())
}

#[test]
fn gitlab_targets_travel_in_prepare_environment_free_of_bitbucket_rules() -> Result<()> {
    let gitlab = serde_json::to_value(project_target()?)?;
    let bitbucket = serde_json::to_value(bitbucket_target()?)?;
    let sealed = seal(prepare(&gitlab, "docker", "linux", "x64", "ephemeral"))??;
    assert_eq!(
        serde_json::to_value(&sealed)?["operation"]["target"],
        gitlab,
        "the GitLab target travels unchanged"
    );
    let replayed: AgentCommandEnvelope = serde_json::from_value(serde_json::to_value(&sealed)?)?;
    assert_eq!(replayed, sealed);
    // Bitbucket's persistent-only and no-Windows-ARM64 rules do not apply to
    // GitLab; the Bitbucket controls show each shape is refused for Bitbucket.
    for (runtime, os, architecture, mode) in [
        ("docker", "linux", "x64", "ephemeral"),
        ("native_process", "windows", "arm64", "persistent"),
    ] {
        assert!(
            seal(prepare(&gitlab, runtime, os, architecture, mode))?.is_ok(),
            "GitLab {runtime} {os} {architecture} {mode} was refused"
        );
        assert_eq!(
            seal(prepare(&bitbucket, runtime, os, architecture, mode))?.err(),
            Some(CommandError::Configuration),
            "Bitbucket control {runtime} {os} {architecture} {mode}"
        );
    }
    Ok(())
}
