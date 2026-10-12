//! Shared query fixtures prove exact Rust/browser wire parity.
//! Authorization and observation freshness remain service-level responsibilities.

use anyhow::{Context as _, Result, bail};
use gridops_core::fleet::protocol::query::{
    BackendReadV1, DomainReadV1, HostEventReadV1, HostReadV1, QueryPage, QueryResult, SampleReadV1,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn roundtrip<T: DeserializeOwned + Serialize>(body: &Value) -> Result<()> {
    let parsed: T = serde_json::from_value(body.clone())?;
    assert_eq!(serde_json::to_value(parsed)?, *body);
    let mut wrong_version = body.clone();
    wrong_version["schemaVersion"] = json!(2);
    assert!(serde_json::from_value::<T>(wrong_version).is_err());
    let mut unknown = body.clone();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<T>(unknown).is_err());
    if body["item"]["projection"] == "target" {
        let mut global_counts = body.clone();
        global_counts["item"]["counts"]["scope"] = json!("host");
        assert!(serde_json::from_value::<T>(global_counts).is_err());
    }
    if let Some(items) = body["items"].as_array() {
        for (index, item) in items.iter().enumerate() {
            if item["projection"] == "target" {
                let mut global_counts = body.clone();
                global_counts["items"][index]["counts"]["scope"] = json!("host");
                assert!(serde_json::from_value::<T>(global_counts).is_err());
            }
        }
    }
    Ok(())
}

#[test]
fn shared_browser_query_responses_roundtrip_without_aliases_or_version_drift() -> Result<()> {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/fleet-query-contracts.json"
    ))?;
    let mut response_count = 0;
    for (group, values) in fixtures.as_object().context("query fixture object")? {
        let values = values.as_array().context("query fixture response array")?;
        assert!(!values.is_empty(), "empty fixture group: {group}");
        for body in values {
            match group.as_str() {
                "hostResults" => roundtrip::<QueryResult<HostReadV1>>(body)?,
                "hostPages" => roundtrip::<QueryPage<HostReadV1>>(body)?,
                "backendResults" => roundtrip::<QueryResult<BackendReadV1>>(body)?,
                "backendPages" => roundtrip::<QueryPage<BackendReadV1>>(body)?,
                "domainResults" => roundtrip::<QueryResult<DomainReadV1>>(body)?,
                "domainPages" => roundtrip::<QueryPage<DomainReadV1>>(body)?,
                "samplePages" => roundtrip::<QueryPage<SampleReadV1>>(body)?,
                "eventPages" => roundtrip::<QueryPage<HostEventReadV1>>(body)?,
                other => bail!("unknown query fixture group: {other}"),
            }
            response_count += 1;
        }
    }
    assert!(response_count >= 8);
    Ok(())
}

#[test]
fn query_text_and_tag_bounds_match_browser_unicode_rules() -> Result<()> {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/fleet-query-contracts.json"
    ))?;
    let original = fixtures["hostResults"][0].clone();
    let parses = |value: Value| serde_json::from_value::<QueryResult<HostReadV1>>(value).is_ok();
    for text in ["é".repeat(128), "🖥".repeat(128)] {
        let mut body = original.clone();
        body["item"]["name"] = json!(text);
        body["item"]["agentVersion"] = body["item"]["name"].clone();
        assert!(parses(body));
    }
    for text in [
        "é".repeat(129),
        "🖥".repeat(129),
        String::new(),
        "bad\u{0085}text".to_owned(),
    ] {
        let mut body = original.clone();
        body["item"]["name"] = json!(text);
        assert!(!parses(body));
    }
    for (key, value, valid) in [
        ("é".repeat(64), String::new(), true),
        ("é".repeat(65), String::new(), false),
        ("key".to_owned(), "🖥".repeat(32), true),
        ("key".to_owned(), "🖥".repeat(33), false),
        (String::new(), "value".to_owned(), false),
        ("bad\u{0085}".to_owned(), "value".to_owned(), false),
        ("key".to_owned(), "bad\u{0085}".to_owned(), false),
    ] {
        let mut body = original.clone();
        body["item"]["tags"] = Value::Object([(key, json!(value))].into_iter().collect());
        assert_eq!(parses(body), valid);
    }
    for (count, valid) in [(64, true), (65, false)] {
        let mut body = original.clone();
        body["item"]["tags"] = Value::Object(
            (0..count)
                .map(|index| (format!("key-{index}"), json!("")))
                .collect(),
        );
        assert_eq!(parses(body), valid);
    }
    // JSON may spell a surrogate, but Rust/UTF-8 strings cannot represent it.
    assert!(serde_json::from_str::<Value>(r#""\ud800""#).is_err());
    Ok(())
}
