//! The build this binary came from: the release version, a build number that rises with every commit, and the
//! commit. `build.rs` records the number and commit (see there for where they come from).
//!
//! The release version (`VERSION`) is what installers and release tags pin and it changes only for a release.
//! The build number tells builds of the same release apart. `FULL_VERSION` joins them as semver build metadata
//! (`0.1.3+build.412`), which stays a valid semver string and an acceptable `agent_version` for the fleet protocol
//! (any printable text of up to 128 characters).

/// The release version, `[workspace.package] version` in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The build number: the commits in the history of the commit built, or `dev` when the build had none.
pub const BUILD_NUMBER: &str = env!("GRIDOPS_BUILD_NUMBER");
/// The short commit, or `unknown`.
pub const COMMIT: &str = env!("GRIDOPS_COMMIT");
/// `VERSION` plus the build number as semver build metadata: `0.1.3+build.412`.
pub const FULL_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "+build.",
    env!("GRIDOPS_BUILD_NUMBER")
);

/// The build number as a number; `None` for a `dev` build.
pub fn build_number() -> Option<u64> {
    BUILD_NUMBER.parse().ok()
}

/// The commit; `None` when it wasn't known.
pub fn commit() -> Option<&'static str> {
    (COMMIT != build_support::UNKNOWN_COMMIT).then_some(COMMIT)
}

/// Human-readable form: `0.1.3+build.412 (9d92a33)`.
pub fn describe() -> String {
    format!("{FULL_VERSION} ({COMMIT})")
}

#[path = "../build_support.rs"]
mod build_support;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::ids::UpstreamText;

    fn git_says(value: &'static str) -> impl FnOnce() -> Option<String> {
        move || Some(value.to_owned())
    }

    #[test]
    fn the_build_number_is_a_number_or_dev() {
        assert!(
            BUILD_NUMBER == build_support::UNKNOWN_BUILD_NUMBER
                || (BUILD_NUMBER.bytes().all(|b| b.is_ascii_digit()) && build_number().is_some()),
            "unexpected build number {BUILD_NUMBER}"
        );
    }

    #[test]
    fn the_full_version_is_the_release_version_with_the_build_number_as_build_metadata() {
        assert_eq!(
            FULL_VERSION,
            format!("{VERSION}+build.{BUILD_NUMBER}"),
            "semver build metadata"
        );
        assert!(FULL_VERSION.starts_with(VERSION));
        assert!(FULL_VERSION.contains(BUILD_NUMBER));
        // Semver build metadata: dot-separated identifiers of [0-9A-Za-z-].
        let metadata = FULL_VERSION.split_once('+').map(|(_, m)| m).unwrap_or("");
        assert!(
            !metadata.is_empty()
                && metadata.split('.').all(|part| !part.is_empty()
                    && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')),
            "{FULL_VERSION} has invalid build metadata"
        );
    }

    #[test]
    fn the_full_version_is_acceptable_as_an_agent_version() {
        assert!(UpstreamText::new(FULL_VERSION).is_ok());
        assert!(UpstreamText::new("0.1.3+build.412").is_ok());
        assert!(UpstreamText::new("0.1.3+build.dev").is_ok());
    }

    #[test]
    fn describe_names_the_commit() {
        assert_eq!(describe(), format!("{FULL_VERSION} ({COMMIT})"));
    }

    #[test]
    fn a_build_number_is_a_positive_integer_without_leading_zeros() {
        assert_eq!(
            build_support::clean_build_number("412").as_deref(),
            Some("412")
        );
        assert_eq!(
            build_support::clean_build_number(" 7\n").as_deref(),
            Some("7")
        );
        for bad in ["", "0", "012", "-1", "4.1", "dev", "12a", "+3"] {
            assert_eq!(build_support::clean_build_number(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_commit_is_cut_to_seven_lower_case_hex_digits() {
        assert_eq!(
            build_support::clean_commit("9D92A33ffe0123").as_deref(),
            Some("9d92a33")
        );
        assert_eq!(
            build_support::clean_commit("9d92a33").as_deref(),
            Some("9d92a33")
        );
        for bad in ["", "9d92a3", "xyz1234", &"a".repeat(41)] {
            assert_eq!(build_support::clean_commit(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_environment_wins_over_git_and_a_bad_value_is_an_error() {
        assert_eq!(
            build_support::resolve_build_number(Some("900"), git_says("5")),
            Ok("900".to_owned())
        );
        assert_eq!(
            build_support::resolve_build_number(None, git_says("5")),
            Ok("5".to_owned())
        );
        assert_eq!(
            build_support::resolve_build_number(Some("  "), git_says("5")),
            Ok("5".to_owned())
        );
        assert!(build_support::resolve_build_number(Some("dev"), git_says("5")).is_err());
        assert_eq!(
            build_support::resolve_commit(Some("ABCDEF0123"), git_says("1234567")),
            Ok("abcdef0".to_owned())
        );
        assert_eq!(
            build_support::resolve_commit(None, git_says("1234567")),
            Ok("1234567".to_owned())
        );
        assert!(build_support::resolve_commit(Some("nothex!"), git_says("1234567")).is_err());
    }

    #[test]
    fn without_env_or_usable_git_the_build_reads_dev_and_unknown() {
        assert_eq!(
            build_support::resolve_build_number(None, || None),
            Ok("dev".to_owned())
        );
        assert_eq!(
            build_support::resolve_build_number(None, git_says("garbage")),
            Ok("dev".to_owned())
        );
        assert_eq!(
            build_support::resolve_commit(None, || None),
            Ok("unknown".to_owned())
        );
    }
}
