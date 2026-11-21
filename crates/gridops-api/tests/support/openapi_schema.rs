//! Checks a response against what `docs/openapi/gridops-v1.json` declares for
//! its operation (#138: "responses validate against the spec").
//!
//! Shared by the binary-level tests (`tests/control_plane_contracts.rs`) and
//! the in-crate GitLab handler tests (`src/resources/gitlab_tests.rs`), which
//! both include this file with `#[path]`.
//!
//! It covers the JSON Schema subset the spec's platform-connection schemas
//! use: `$ref`, `type`, `required`, `properties`, `items`, `const`, `enum`,
//! `minimum`, `minLength`, and the `date-time` and `uri` formats. Objects are
//! checked strictly: a property the schema does not declare is a violation,
//! because `GitlabConnection` must carry nothing beyond its declared fields
//! (no token, webhook secret or webhook URL).

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

const SPEC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/openapi/gridops-v1.json"
));

fn spec() -> Result<Value> {
    serde_json::from_str(SPEC).context("docs/openapi/gridops-v1.json is not JSON")
}

fn resolve<'a>(spec: &'a Value, value: &'a Value) -> Result<&'a Value> {
    let Some(reference) = value.get("$ref").and_then(Value::as_str) else {
        return Ok(value);
    };
    let mut cursor = spec;
    for segment in reference.trim_start_matches("#/").split('/') {
        cursor = cursor
            .get(segment)
            .with_context(|| format!("dangling $ref {reference}"))?;
    }
    Ok(cursor)
}

/// Fails unless `status` is a response the spec declares for `method`
/// `path` (the spec's path template, e.g.
/// `/api/v1/platform-connections/gitlab/{connection_id}`) and `body` matches
/// that response's schema. A response the spec declares without content must
/// have an empty body (`Value::Null`).
pub fn check_response(path: &str, method: &str, status: u16, body: &Value) -> Result<()> {
    let spec = spec()?;
    let operation = spec["paths"][path][method.to_ascii_lowercase()]
        .as_object()
        .with_context(|| format!("the spec has no {method} {path}"))?;
    let response = operation
        .get("responses")
        .and_then(|responses| responses.get(status.to_string()))
        .with_context(|| format!("the spec declares no {status} for {method} {path}: {body}"))?;
    let response = resolve(&spec, response)?;
    match response.get("content") {
        None => {
            if !body.is_null() {
                bail!("{method} {path} {status} declares no body but got {body}");
            }
            Ok(())
        }
        Some(content) => {
            let schema = resolve(&spec, &content["application/json"]["schema"])?;
            let mut violations = Vec::new();
            check(&spec, schema, body, "$", &mut violations)?;
            if violations.is_empty() {
                Ok(())
            } else {
                bail!(
                    "{method} {path} {status} does not match the spec:\n  {}\nbody: {body}",
                    violations.join("\n  ")
                )
            }
        }
    }
}

fn check(
    spec: &Value,
    schema: &Value,
    value: &Value,
    at: &str,
    violations: &mut Vec<String>,
) -> Result<()> {
    let schema = resolve(spec, schema)?;
    if let Some(expected) = schema.get("const")
        && expected != value
    {
        violations.push(format!("{at}: expected {expected}, got {value}"));
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        violations.push(format!("{at}: {value} is not one of {allowed:?}"));
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => {
            let Some(object) = value.as_object() else {
                violations.push(format!("{at}: expected an object, got {value}"));
                return Ok(());
            };
            for required in schema["required"].as_array().into_iter().flatten() {
                let name = required.as_str().unwrap_or_default();
                if !object.contains_key(name) {
                    violations.push(format!("{at}: missing required {name}"));
                }
            }
            let properties = schema["properties"].as_object();
            for (name, item) in object {
                match properties.and_then(|properties| properties.get(name)) {
                    Some(property) => {
                        check(spec, property, item, &format!("{at}.{name}"), violations)?;
                    }
                    None if properties.is_some() => {
                        violations.push(format!("{at}: undeclared property {name}"));
                    }
                    None => {}
                }
            }
        }
        Some("array") => {
            let Some(items) = value.as_array() else {
                violations.push(format!("{at}: expected an array, got {value}"));
                return Ok(());
            };
            if let Some(item_schema) = schema.get("items") {
                for (index, item) in items.iter().enumerate() {
                    check(
                        spec,
                        item_schema,
                        item,
                        &format!("{at}[{index}]"),
                        violations,
                    )?;
                }
            }
        }
        Some("string") => {
            let Some(text) = value.as_str() else {
                violations.push(format!("{at}: expected a string, got {value}"));
                return Ok(());
            };
            if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64)
                && (text.chars().count() as u64) < minimum
            {
                violations.push(format!("{at}: shorter than {minimum}"));
            }
            match schema.get("format").and_then(Value::as_str) {
                Some("date-time") if chrono::DateTime::parse_from_rfc3339(text).is_err() => {
                    violations.push(format!("{at}: {text:?} is not an RFC 3339 date-time"));
                }
                Some("uri") if url::Url::parse(text).is_err() => {
                    violations.push(format!("{at}: {text:?} is not a URI"));
                }
                _ => {}
            }
        }
        Some("integer") => match value.as_i64() {
            None => violations.push(format!("{at}: expected an integer, got {value}")),
            Some(number) => {
                if let Some(minimum) = schema.get("minimum").and_then(Value::as_i64)
                    && number < minimum
                {
                    violations.push(format!("{at}: {number} is below {minimum}"));
                }
            }
        },
        Some("boolean") if !value.is_boolean() => {
            violations.push(format!("{at}: expected a boolean, got {value}"));
        }
        _ => {}
    }
    Ok(())
}
