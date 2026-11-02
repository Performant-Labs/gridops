//! How a build number and commit are resolved. `build.rs` includes this file, and so does the crate
//! (`build_info.rs`), so the rules are tested as ordinary code even though a build script can't be.

#![allow(dead_code)] // build.rs uses most of this, the crate's `build_info` the rest

/// What a build records when no number could be found (no env, no usable git history).
pub const UNKNOWN_BUILD_NUMBER: &str = "dev";
/// What a build records when no commit could be found.
pub const UNKNOWN_COMMIT: &str = "unknown";

/// A build number from the environment or git: a positive integer without leading zeros, as `git rev-list --count` prints it.
pub fn clean_build_number(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let valid =
        !raw.is_empty() && !raw.starts_with('0') && raw.bytes().all(|byte| byte.is_ascii_digit());
    valid.then(|| raw.to_owned())
}

/// A commit id (7 to 40 hex digits) cut to the 7-character short form, in lower case.
pub fn clean_commit(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let valid = (7..=40).contains(&raw.len()) && raw.bytes().all(|byte| byte.is_ascii_hexdigit());
    valid.then(|| raw[..7].to_ascii_lowercase())
}

/// The build number: the environment's if it is set (a malformed one is an error, never silently replaced), else git's, else `dev`.
pub fn resolve_build_number(
    from_env: Option<&str>,
    from_git: impl FnOnce() -> Option<String>,
) -> Result<String, String> {
    match from_env.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => clean_build_number(value).ok_or_else(|| {
            format!("GRIDOPS_BUILD_NUMBER must be a positive integer without leading zeros, got '{value}'")
        }),
        None => Ok(from_git()
            .and_then(|value| clean_build_number(&value))
            .unwrap_or_else(|| UNKNOWN_BUILD_NUMBER.to_owned())),
    }
}

/// The short commit: the environment's if it is set (a malformed one is an error), else git's, else `unknown`.
pub fn resolve_commit(
    from_env: Option<&str>,
    from_git: impl FnOnce() -> Option<String>,
) -> Result<String, String> {
    match from_env.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => clean_commit(value).ok_or_else(|| {
            format!("GRIDOPS_COMMIT must be a hexadecimal commit id of 7 to 40 characters, got '{value}'")
        }),
        None => Ok(from_git()
            .and_then(|value| clean_commit(&value))
            .unwrap_or_else(|| UNKNOWN_COMMIT.to_owned())),
    }
}
