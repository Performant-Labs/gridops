//! Serves the checked-in [`OpenAPI`] 3.1 description of the fleet and
//! operations routes, and keeps it honest: the drift test below fails the
//! build when the token-capable route table and the spec disagree in either
//! direction.

use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

pub(crate) const SPEC: &str = include_str!("../../../../docs/openapi/gridops-v1.json");

pub(crate) async fn get() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SPEC,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::TOKEN_SCOPED_ROUTES;
    use anyhow::Context as _;
    use serde_json::Value;

    fn spec() -> anyhow::Result<Value> {
        Ok(serde_json::from_str(SPEC)?)
    }

    /// The (method, path, scope) triples the spec declares token-capable: any
    /// operation whose security includes the bearer scheme with a scope.
    fn spec_token_routes(spec: &Value) -> anyhow::Result<Vec<(String, String, String)>> {
        let mut routes = Vec::new();
        let paths: &serde_json::Map<String, Value> = spec["paths"].as_object().context("paths")?;
        for (path, item) in paths {
            let Some(item) = item.as_object() else {
                continue;
            };
            for (method, operation) in item {
                if !matches!(method.as_str(), "get" | "put" | "post" | "delete" | "patch")
                    || method.starts_with("x-")
                {
                    continue;
                }
                for alternative in operation["security"].as_array().into_iter().flatten() {
                    let Some(requirements) = alternative["bearerAuth"].as_array() else {
                        continue;
                    };
                    for scope in requirements {
                        if let Some(scope) = scope.as_str() {
                            routes.push((
                                method.to_ascii_uppercase(),
                                path.clone(),
                                scope.to_owned(),
                            ));
                        }
                    }
                }
            }
        }
        routes.sort();
        routes.dedup();
        Ok(routes)
    }

    #[test]
    fn the_spec_matches_the_token_scope_table_in_both_directions() -> anyhow::Result<()> {
        let spec = spec()?;
        let table = {
            let mut table = TOKEN_SCOPED_ROUTES.to_vec();
            table.sort_unstable();
            table
        };
        let declared = spec_token_routes(&spec)?;
        let normalized: Vec<_> = declared
            .iter()
            .map(|(method, path, scope)| (method.as_str(), path.as_str(), scope.as_str()))
            .collect();
        assert_eq!(
            normalized, table,
            "the spec and TOKEN_SCOPED_ROUTES disagree; update docs/openapi/gridops-v1.json"
        );
        Ok(())
    }

    #[test]
    fn the_spec_is_structurally_sound_openapi_31() -> anyhow::Result<()> {
        let spec = spec()?;
        assert!(
            spec["openapi"]
                .as_str()
                .is_some_and(|version| version.starts_with("3.1."))
        );
        for scheme in ["bearerAuth", "cookieAuth", "hostCredential"] {
            assert!(
                spec["components"]["securitySchemes"][scheme].is_object(),
                "missing security scheme {scheme}"
            );
        }
        let paths = spec["paths"].as_object().context("paths")?;
        assert!(paths.len() > 20, "expected the full route surface");
        for (path, item) in paths {
            for (method, operation) in item.as_object().into_iter().flatten() {
                if method.starts_with("x-") || method == "parameters" {
                    continue;
                }
                assert!(
                    operation["responses"].is_object(),
                    "{method} {path} has no responses"
                );
                for security in operation["security"].as_array().into_iter().flatten() {
                    for requirement in security.as_object().into_iter().flatten() {
                        assert!(
                            spec["components"]["securitySchemes"][requirement.0.as_str()]
                                .is_object(),
                            "{method} {path} references unknown scheme {}",
                            requirement.0
                        );
                    }
                }
            }
        }
        // Every internal $ref resolves inside the document.
        let text = SPEC;
        for reference in text.match_indices("\"$ref\"").map(|(at, _)| at) {
            let tail = &text[reference..];
            let target = tail
                .split('"')
                .nth(3)
                .unwrap_or_default()
                .strip_prefix("#")
                .unwrap_or_default();
            assert!(!target.is_empty(), "unparseable $ref near byte {reference}");
            let mut cursor = &spec;
            let mut ok = true;
            for segment in target.trim_start_matches('/').split('/') {
                if segment.is_empty() {
                    continue;
                }
                let decoded = segment.replace("~1", "/").replace("~0", "~");
                if let Some(value) = cursor.get(&decoded) {
                    cursor = value;
                } else {
                    ok = false;
                    break;
                }
            }
            assert!(ok, "dangling $ref {target}");
        }
        Ok(())
    }

    /// The error examples parse as the shared contract type, exactly the way
    /// `crates/gridops-core/tests/fleet_contracts.rs` parses documentation
    /// examples, so the spec cannot drift from the wire types.
    #[test]
    fn error_examples_parse_as_the_contract_type() -> anyhow::Result<()> {
        use gridops_core::fleet::protocol::browser::{ErrorCode, FleetErrorResponse};
        let spec = spec()?;
        for (name, expected_status) in [
            ("fleetErrorRevisionConflict", 409u16),
            ("fleetErrorScopeDenied", 403),
            ("fleetErrorHostNotDeletable", 409),
            ("fleetErrorHostNotRetirable", 409),
            ("fleetErrorTokenNotAccepted", 403),
        ] {
            let example = spec["components"]["examples"][name]["value"].to_string();
            let parsed: FleetErrorResponse = serde_json::from_str(&example)
                .with_context(|| format!("example {name} does not parse as FleetErrorResponse"))?;
            assert_eq!(parsed.code.status(), expected_status, "example {name}");
            let _ = ErrorCode::InvalidRequest;
        }
        Ok(())
    }

    /// The enrollment example matches the issue response serialization:
    /// exactly `code`, `enrollmentId` and `expiresAt`, with a 43-character
    /// one-time code, a UUID and an RFC 3339 timestamp.
    #[test]
    fn enrollment_example_matches_the_issue_response_shape() -> anyhow::Result<()> {
        let spec = spec()?;
        let example = &spec["components"]["examples"]["enrollmentIssueCreated"]["value"];
        let object = example.as_object().context("object example")?;
        assert_eq!(object.len(), 3, "keys: {object:?}");
        let keys: Vec<&str> = object.keys().map(String::as_str).collect();
        for key in ["code", "enrollmentId", "expiresAt"] {
            assert!(keys.contains(&key), "missing key {key}");
        }
        assert_eq!(example["code"].as_str().map(str::len), Some(43));
        let enrollment_id = example["enrollmentId"].as_str().context("uuid")?;
        assert!(uuid::Uuid::parse_str(enrollment_id).is_ok());
        assert!(
            example["expiresAt"]
                .as_str()
                .is_some_and(|stamp| stamp.ends_with('Z'))
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_serving_route_ships_the_checked_in_bytes_with_the_right_headers()
    -> anyhow::Result<()> {
        let response = get().await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
        let body = axum::body::to_bytes(response.into_body(), SPEC.len() + 1).await?;
        assert_eq!(body.as_ref(), SPEC.as_bytes());
        Ok(())
    }

    /// Resolves a `#/components/...` reference inside the document.
    fn resolve<'a>(spec: &'a Value, value: &'a Value) -> &'a Value {
        let Some(reference) = value["$ref"].as_str() else {
            return value;
        };
        reference
            .trim_start_matches("#/")
            .split('/')
            .fold(spec, |cursor, segment| &cursor[segment])
    }

    fn property_names(schema: &Value) -> Vec<&str> {
        let mut names: Vec<&str> = schema["properties"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, _)| name.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    fn json_schema<'a>(spec: &'a Value, holder: &'a Value) -> &'a Value {
        resolve(
            spec,
            &resolve(spec, holder)["content"]["application/json"]["schema"],
        )
    }

    const GITLAB_CONNECTIONS: &str = "/api/v1/platform-connections/gitlab";
    const GITLAB_CONNECTION: &str = "/api/v1/platform-connections/gitlab/{connection_id}";
    const GITLAB_VERIFY: &str = "/api/v1/platform-connections/gitlab/{connection_id}/verify";
    const GITLAB_ROTATE: &str =
        "/api/v1/platform-connections/gitlab/{connection_id}/webhook-secret/rotate";
    const GITLAB_WEBHOOK: &str = "/api/v1/webhooks/gitlab";

    /// (path, method, the success status, its body schema, issue that serves it)
    const GITLAB_OPERATIONS: [(&str, &str, &str, Option<&str>, &str); 7] = [
        (
            GITLAB_CONNECTIONS,
            "get",
            "200",
            Some("GitlabConnectionList"),
            "#138",
        ),
        (
            GITLAB_CONNECTIONS,
            "post",
            "201",
            Some("GitlabConnectionCreated"),
            "#138",
        ),
        (
            GITLAB_CONNECTION,
            "patch",
            "200",
            Some("GitlabConnection"),
            "#138",
        ),
        (GITLAB_CONNECTION, "delete", "204", None, "#138"),
        (GITLAB_VERIFY, "post", "204", None, "#138"),
        (
            GITLAB_ROTATE,
            "post",
            "200",
            Some("GitlabWebhookSecret"),
            "#138",
        ),
        (GITLAB_WEBHOOK, "post", "202", None, "#140"),
    ];

    /// #137 W-7: the GitLab routes #138 and #140 implement against, declared
    /// before any handler exists.
    #[test]
    fn the_gitlab_operations_are_specified_before_they_are_served() -> anyhow::Result<()> {
        let spec = spec()?;
        for (path, method, status, body, owner) in GITLAB_OPERATIONS {
            let operation = &spec["paths"][path][method];
            assert!(operation.is_object(), "missing {method} {path}");
            let description = operation["description"].as_str().unwrap_or_default();
            assert!(
                description.starts_with(&format!("Not served yet: implemented by {owner}")),
                "{method} {path} description: {description:?}"
            );
            let expected_security = if path == GITLAB_WEBHOOK {
                serde_json::json!([])
            } else {
                serde_json::json!([{ "cookieAuth": [] }])
            };
            assert_eq!(
                operation["security"], expected_security,
                "{method} {path} security"
            );
            let responses = operation["responses"].as_object().context("responses")?;
            assert!(
                responses.contains_key(status),
                "{method} {path} has no {status}"
            );
            if let Some(body) = body {
                assert_eq!(
                    json_schema(&spec, &responses[status]),
                    &spec["components"]["schemas"][body],
                    "{method} {path} {status} body"
                );
            }
            // Errors use the API's own body, never the fleet ErrorResponse.
            assert!(
                !operation.to_string().contains("ErrorResponse"),
                "{method} {path} uses the fleet ErrorResponse"
            );
            for (code, response) in responses {
                let response = resolve(&spec, response);
                if code.starts_with('2') || response.get("content").is_none() {
                    continue;
                }
                assert_eq!(
                    json_schema(&spec, response),
                    &spec["components"]["schemas"]["ApiErrorBody"],
                    "{method} {path} {code} error body"
                );
            }
        }
        let conflicts = |path: &str, method: &str| {
            spec["paths"][path][method]["responses"]["409"]["description"]
                .as_str()
                .map(str::to_owned)
        };
        assert!(
            conflicts(GITLAB_CONNECTIONS, "post").is_some(),
            "POST 409 on a duplicate scope"
        );
        assert!(
            conflicts(GITLAB_CONNECTION, "delete")
                .is_some_and(|text| text.contains("while a pool or a fleet target references it")),
            "DELETE 409 covers pools and fleet targets"
        );
        for (path, method) in [
            (GITLAB_CONNECTION, "patch"),
            (GITLAB_CONNECTION, "delete"),
            (GITLAB_VERIFY, "post"),
            (GITLAB_ROTATE, "post"),
        ] {
            let parameters = spec["paths"][path][method]["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(
                    spec["paths"][path]["parameters"]
                        .as_array()
                        .into_iter()
                        .flatten(),
                )
                .map(|parameter| resolve(&spec, parameter));
            assert!(
                parameters
                    .into_iter()
                    .any(|parameter| parameter["name"] == "connection_id"
                        && parameter["in"] == "path"
                        && parameter["required"] == true),
                "{method} {path} declares the connection_id path parameter"
            );
        }
        let webhook = &spec["paths"][GITLAB_WEBHOOK]["post"];
        for header in ["X-Gitlab-Token", "X-Gitlab-Event"] {
            assert!(
                webhook["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|parameter| resolve(&spec, parameter))
                    .any(|parameter| parameter["name"] == header
                        && parameter["in"] == "header"
                        && parameter["required"] == true),
                "the webhook requires the {header} header"
            );
        }
        assert!(webhook["responses"]["401"].is_object(), "webhook 401");
        assert_eq!(
            json_schema(&spec, &webhook["requestBody"])["type"],
            "object",
            "the webhook body is a free-form JSON object"
        );
        Ok(())
    }

    #[test]
    fn the_gitlab_schemas_have_the_pinned_fields_and_no_secrets() -> anyhow::Result<()> {
        let spec = spec()?;
        let schemas = &spec["components"]["schemas"];
        let connection = &schemas["GitlabConnection"];
        assert_eq!(
            property_names(connection),
            [
                "baseUrl",
                "canManage",
                "createdAt",
                "id",
                "name",
                "provider",
                "scopeId",
                "scopeKind",
                "scopePath",
                "updatedAt"
            ]
        );
        let provider = &connection["properties"]["provider"];
        assert!(
            provider["const"] == "gitlab" || provider["enum"] == serde_json::json!(["gitlab"]),
            "provider is always \"gitlab\": {provider}"
        );
        assert_eq!(connection["properties"]["scopeId"]["type"], "integer");
        assert_eq!(
            property_names(&schemas["GitlabConnectionList"]),
            ["canManage", "items"]
        );
        assert_eq!(
            schemas["GitlabConnectionList"]["properties"]["canManage"]["type"],
            "boolean"
        );
        assert_eq!(
            resolve(
                &spec,
                &schemas["GitlabConnectionList"]["properties"]["items"]["items"]
            ),
            connection
        );
        let create = &schemas["CreateGitlabConnectionRequest"];
        assert_eq!(
            property_names(create),
            ["accessToken", "baseUrl", "name", "scopeKind", "scopePath"]
        );
        assert_eq!(
            create["properties"]["baseUrl"]["default"],
            "https://gitlab.com"
        );
        assert_eq!(
            create["properties"]["scopeKind"]["enum"],
            serde_json::json!(["project", "group"])
        );
        assert_eq!(
            property_names(&schemas["GitlabConnectionCreated"]),
            ["connection", "webhookSecret", "webhookUrl"]
        );
        assert_eq!(
            property_names(&schemas["UpdateGitlabConnectionRequest"]),
            ["accessToken", "name"]
        );
        assert!(
            schemas["UpdateGitlabConnectionRequest"]["required"]
                .as_array()
                .is_none_or(Vec::is_empty),
            "a PATCH body requires nothing"
        );
        assert_eq!(
            property_names(&schemas["GitlabWebhookSecret"]),
            ["webhookSecret", "webhookUrl"]
        );
        assert_eq!(property_names(&schemas["ApiErrorBody"]), ["code", "error"]);
        assert_eq!(
            schemas["ApiErrorBody"]["required"],
            serde_json::json!(["error"])
        );
        // Only the create response and the rotation carry the webhook secret.
        for (name, schema) in schemas.as_object().context("schemas")? {
            if name == "GitlabConnectionCreated" || name == "GitlabWebhookSecret" {
                continue;
            }
            if name.starts_with("Gitlab") {
                let text = schema.to_string();
                assert!(
                    !text.contains("webhookSecret"),
                    "{name} carries the webhook secret"
                );
            }
        }
        Ok(())
    }
}
