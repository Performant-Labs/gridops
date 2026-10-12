//! Strict query extraction for fleet reads.
//! Reject ambiguous or malformed parameters before typed request decoding.

use std::collections::BTreeSet;

use axum::{
    extract::{FromRequestParts, Query},
    http::request::Parts,
};
use gridops_core::fleet::protocol::browser::ErrorCode;
use serde::de::DeserializeOwned;

use super::{boundary::context, error::FleetApiError};
use crate::state::AppState;

const MAX_QUERY_BYTES: usize = 8192;
const MAX_QUERY_PARAMETERS: usize = 8;

pub(super) struct FleetQuery<T>(pub T);

impl<T: DeserializeOwned + Send> FromRequestParts<AppState> for FleetQuery<T> {
    type Rejection = FleetApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let context = context(parts);
        StrictQuery::parse(parts.uri.query().unwrap_or_default())
            .map_err(|()| context.error(ErrorCode::InvalidRequest))?;
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|_| context.error(ErrorCode::InvalidRequest))
    }
}

/// The raw query has unique decoded keys and valid UTF-8, including escapes.
/// The typed DTO remains responsible for allowed keys and field bounds.
struct StrictQuery;

impl StrictQuery {
    fn parse(raw: &str) -> Result<Self, ()> {
        if raw.len() > MAX_QUERY_BYTES {
            return Err(());
        }
        if raw.is_empty() {
            return Ok(Self);
        }
        let mut keys = BTreeSet::new();
        for (index, pair) in raw.split('&').enumerate() {
            if index >= MAX_QUERY_PARAMETERS {
                return Err(());
            }
            let (key, value) = pair.split_once('=').ok_or(())?;
            let key = decode_component(key)?;
            let _value = decode_component(value)?;
            if key.is_empty() || !keys.insert(key) {
                return Err(());
            }
        }
        Ok(Self)
    }
}

fn decode_component(encoded: &str) -> Result<String, ()> {
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut bytes = encoded.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(match byte {
            b'+' => b' ',
            b'%' => {
                let high = hex_value(bytes.next().ok_or(())?)?;
                let low = hex_value(bytes.next().ok_or(())?)?;
                high * 16 + low
            }
            other => other,
        });
    }
    String::from_utf8(decoded).map_err(|_| ())
}

fn hex_value(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_decoding_rejects_ambiguity_and_invalid_encoding() {
        for raw in [
            "limit=1&limit=2",
            "limit=1&%6cimit=2",
            "cursor=x&cursor=y",
            "limit=1&",
            "=x",
            "limit",
            "limit=%",
            "limit=%2",
            "limit=%gg",
            "cursor=%FF",
            "cursor=%C3%28",
            "a=1&b=2&c=3&d=4&e=5&f=6&g=7&h=8&i=9",
        ] {
            assert!(StrictQuery::parse(raw).is_err(), "accepted malformed query");
        }
        assert!(StrictQuery::parse(&format!("cursor={}", "a".repeat(MAX_QUERY_BYTES))).is_err());
        for raw in [
            "",
            "limit=25",
            "cursor=q1.a_b-c&limit=1",
            "receivedFrom=2026-10-09T00%3A00%3A00Z",
            "name=caf%C3%A9",
        ] {
            assert!(StrictQuery::parse(raw).is_ok());
        }
        assert_eq!(decode_component("a+b%2Bc"), Ok("a b+c".to_owned()));
    }
}
