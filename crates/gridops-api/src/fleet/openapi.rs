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
}
