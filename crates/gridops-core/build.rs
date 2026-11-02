//! Records the build number and commit in the binary: `GRIDOPS_BUILD_NUMBER` and `GRIDOPS_COMMIT` for
//! `build_info`. Both come from the environment first (`scripts/build-info.sh` prints them; Docker has no
//! `.git`, so it passes them in), then from git (`git rev-list --count HEAD`, `git rev-parse --short=7 HEAD`),
//! and otherwise read `dev` and `unknown`. A shallow clone has no usable count, so it reads `dev` too.

use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

mod build_support;

fn main() -> ExitCode {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GRIDOPS_BUILD_NUMBER");
    println!("cargo:rerun-if-env-changed=GRIDOPS_COMMIT");

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let env_number = env::var("GRIDOPS_BUILD_NUMBER").ok();
    let env_commit = env::var("GRIDOPS_COMMIT").ok();
    let set = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());

    // Git is consulted, and its state watched, only for what the environment doesn't give.
    let git_needed = !(set(&env_number) && set(&env_commit));
    if git_needed {
        watch_git(&manifest_dir);
    }

    let number = build_support::resolve_build_number(env_number.as_deref(), || {
        if git_needed {
            git_build_number(&manifest_dir)
        } else {
            None
        }
    });
    let commit = build_support::resolve_commit(env_commit.as_deref(), || {
        git(&manifest_dir, &["rev-parse", "--short=7", "HEAD"])
    });
    match (number, commit) {
        (Ok(number), Ok(commit)) => {
            println!("cargo:rustc-env=GRIDOPS_BUILD_NUMBER={number}");
            println!("cargo:rustc-env=GRIDOPS_COMMIT={commit}");
            ExitCode::SUCCESS
        }
        (number, commit) => {
            for message in [number.err(), commit.err()].into_iter().flatten() {
                eprintln!("gridops-core build script: {message}");
            }
            ExitCode::FAILURE
        }
    }
}

fn git(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn git_build_number(directory: &Path) -> Option<String> {
    // A shallow clone counts only the commits it fetched, which would read as a real but wrong number.
    if git(directory, &["rev-parse", "--is-shallow-repository"]).as_deref() != Some("false") {
        return None;
    }
    git(directory, &["rev-list", "--count", "HEAD"])
}

/// Reruns the script when HEAD moves: a checkout (HEAD), a commit (the branch's ref, or packed-refs). Works in a
/// linked worktree, where HEAD lives in the worktree's own git directory and refs in the common one.
fn watch_git(directory: &Path) {
    let Some(dirs) = git(directory, &["rev-parse", "--git-dir", "--git-common-dir"]) else {
        return;
    };
    let mut lines = dirs.lines();
    let (Some(git_dir), Some(common_dir)) = (lines.next(), lines.next()) else {
        return;
    };
    let git_dir = directory.join(git_dir);
    let common_dir = directory.join(common_dir);
    let head = git_dir.join("HEAD");
    let mut watch = vec![head.clone(), common_dir.join("packed-refs")];
    if let Some(branch) = std::fs::read_to_string(&head)
        .ok()
        .and_then(|text| text.strip_prefix("ref: ").map(|r| r.trim().to_owned()))
    {
        watch.push(common_dir.join(branch));
    }
    for path in watch.into_iter().filter(|path| path.exists()) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
