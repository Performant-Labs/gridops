//! The host agent installers, served with this deployment's address and
//! version filled in, so a machine always gets the agent that matches it.

use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};

use crate::state::AppState;

const SHELL: &str = include_str!("../../../scripts/install.sh");
const POWERSHELL: &str = include_str!("../../../scripts/install.ps1");

pub async fn shell(State(state): State<AppState>) -> Response {
    script(&state, SHELL, "text/x-shellscript; charset=utf-8")
}

pub async fn powershell(State(state): State<AppState>) -> Response {
    script(&state, POWERSHELL, "text/plain; charset=utf-8")
}

fn script(state: &AppState, template: &str, content_type: &'static str) -> Response {
    let body = template
        .replace("__GRIDOPS_URL__", &address(state.config.base_url()))
        .replace("__GRIDOPS_VERSION__", env!("CARGO_PKG_VERSION"));
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

/// The address scripts embed in quoted strings. Anything a shell or
/// PowerShell could expand falls back to the bare origin.
fn address(base_url: &url::Url) -> String {
    let full = base_url.as_str().trim_end_matches('/');
    let safe = full
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || ":/._~%-[]".contains(c));
    if safe {
        full.to_owned()
    } else {
        base_url.origin().ascii_serialization()
    }
}
