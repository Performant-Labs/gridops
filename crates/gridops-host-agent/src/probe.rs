//! Host facts the agent can read without privileges. Backends are added by
//! the platform adapters; until then the agent reports only its physical root
//! domain, and unknown values stay unknown rather than zero.

use gridops_core::fleet::{
    domain::{Architecture, ExecutionOs},
    ids::UpstreamText,
    protocol::{
        inventory::{DomainKind, ObservedCapacity, ObservedDomain, ObservedSample, SampleCoverage},
        primitives::{BrowserUint53, WireTimestamp},
    },
};

pub const ROOT_DOMAIN: &str = "root";

pub const fn host_os() -> ExecutionOs {
    if cfg!(target_os = "macos") {
        ExecutionOs::Macos
    } else if cfg!(target_os = "windows") {
        ExecutionOs::Windows
    } else {
        ExecutionOs::Linux
    }
}

pub const fn architecture() -> Architecture {
    if cfg!(target_arch = "aarch64") {
        Architecture::Arm64
    } else {
        Architecture::X64
    }
}

/// The kernel's boot identity, so the server can tell a reboot from a
/// reconnect. When the OS doesn't expose one, a per-process value is used,
/// which makes every restart look like a reboot: the conservative choice.
pub fn boot_id() -> String {
    static PROCESS_BOOT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    #[cfg(target_os = "linux")]
    if let Ok(id) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id") {
        return id.trim().to_owned();
    }
    #[cfg(target_os = "macos")]
    if let Ok(output) = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.bootsessionuuid"])
        .output()
    {
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if output.status.success() && !id.is_empty() {
            return id;
        }
    }
    PROCESS_BOOT
        .get_or_init(|| format!("process-{}", uuid::Uuid::new_v4()))
        .clone()
}

pub fn root_domain(name: &str) -> anyhow::Result<ObservedDomain> {
    let cpu_millis = std::thread::available_parallelism()
        .ok()
        .and_then(|count| u64::try_from(count.get()).ok())
        .map(|count| BrowserUint53::new(count * 1000))
        .transpose()?;
    Ok(ObservedDomain {
        local_key: UpstreamText::new(ROOT_DOMAIN)?,
        known_id: None,
        parent_local_key: None,
        kind: DomainKind::Physical,
        name: UpstreamText::new(name)?,
        runtime_identity: None,
        capacity: ObservedCapacity {
            cpu_millis,
            memory_mib: None,
            disk_bytes: None,
        },
    })
}

pub fn root_sample(observed_at: WireTimestamp) -> anyhow::Result<ObservedSample> {
    Ok(ObservedSample {
        domain_local_key: UpstreamText::new(ROOT_DOMAIN)?,
        observed_at,
        coverage: SampleCoverage::Unknown,
        cpu_used_millis: None,
        memory_used_mib: None,
        disk_free_bytes: None,
        uptime_seconds: None,
    })
}
