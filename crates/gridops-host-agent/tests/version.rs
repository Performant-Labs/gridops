//! `gridops-host-agent --version` names the release version and the build, so an upgraded agent can be told apart
//! from the one it replaced.

use std::process::Command;

use anyhow::{Result, ensure};

#[test]
fn version_flag_prints_the_release_version_the_build_number_and_the_commit() -> Result<()> {
    for flag in ["--version", "-V", "version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_gridops-host-agent"))
            .arg(flag)
            .output()?;
        ensure!(output.status.success(), "{flag} failed: {output:?}");
        let printed = String::from_utf8(output.stdout)?;
        let expected = format!(
            "gridops-host-agent {}+build.{} ({})\n",
            env!("CARGO_PKG_VERSION"),
            gridops_core::build_info::BUILD_NUMBER,
            gridops_core::build_info::COMMIT,
        );
        ensure!(
            printed == expected,
            "{flag} printed {printed:?}, expected {expected:?}"
        );
    }
    Ok(())
}
