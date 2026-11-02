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
    #[cfg(target_os = "windows")]
    if let Some(boot) = powershell(
        "(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('o')",
    ) {
        if !boot.is_empty() {
            return format!("windows-{boot}");
        }
    }
    PROCESS_BOOT
        .get_or_init(|| format!("process-{}", uuid::Uuid::new_v4()))
        .clone()
}

/// One reading of the host's capacity and use. Missing values stay unknown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Measurement {
    pub memory_total_mib: Option<u64>,
    pub memory_used_mib: Option<u64>,
    /// One-minute load average as CPU demand, in thousandths of a CPU.
    pub cpu_used_millis: Option<u64>,
    pub uptime_seconds: Option<u64>,
    /// Total and free bytes of the root filesystem: the disk runners and
    /// their logs share with the host.
    pub disk_total_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
}

/// Reads the root filesystem's total and free bytes: the disk runners and
/// their logs share with the host.
#[cfg(unix)]
fn disk_bytes() -> Option<(u64, u64)> {
    gridops_core::disk::filesystem_bytes(std::path::Path::new("/"))
}

/// Reads memory and load without privileges.
pub fn measure() -> Measurement {
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let field = |name: &str| {
            meminfo
                .lines()
                .find(|line| line.starts_with(name))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse::<u64>().ok())
                .map(|kib| kib / 1024)
        };
        let total = field("MemTotal:");
        let available = field("MemAvailable:");
        let load = std::fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|value| value.split_whitespace().next().map(str::to_owned));
        let uptime = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|value| value.split_whitespace().next().map(str::to_owned));
        let (disk_total_bytes, disk_free_bytes) = disk_bytes().unzip();
        return Measurement {
            memory_total_mib: total,
            memory_used_mib: total
                .zip(available)
                .map(|(total, free)| total.saturating_sub(free)),
            cpu_used_millis: load.as_deref().and_then(millis),
            uptime_seconds: uptime
                .as_deref()
                .and_then(|value| value.split('.').next()?.parse().ok()),
            disk_total_bytes,
            disk_free_bytes,
        };
    }
    #[cfg(target_os = "macos")]
    {
        let sysctl = |name: &str| {
            std::process::Command::new("/usr/sbin/sysctl")
                .args(["-n", name])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        };
        let total = sysctl("hw.memsize")
            .and_then(|value| value.parse::<u64>().ok())
            .map(|bytes| bytes / (1024 * 1024));
        let used = std::process::Command::new("/usr/bin/vm_stat")
            .output()
            .ok()
            .and_then(|output| vm_stat_used_mib(&String::from_utf8_lossy(&output.stdout)));
        let load = sysctl("vm.loadavg").and_then(|value| {
            value
                .trim_matches(['{', '}', ' '])
                .split_whitespace()
                .next()
                .and_then(millis)
        });
        let (disk_total_bytes, disk_free_bytes) = disk_bytes().unzip();
        return Measurement {
            memory_total_mib: total,
            memory_used_mib: used,
            cpu_used_millis: load,
            uptime_seconds: None,
            disk_total_bytes,
            disk_free_bytes,
        };
    }
    #[cfg(target_os = "windows")]
    {
        // Total/free physical memory in KiB and average CPU load percent.
        let reading = powershell(
            "$o=Get-CimInstance Win32_OperatingSystem; $c=(Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average; $d=Get-CimInstance Win32_LogicalDisk -Filter \"DeviceID='$env:SystemDrive'\"; '{0} {1} {2} {3} {4}' -f $o.TotalVisibleMemorySize,$o.FreePhysicalMemory,$c,$d.Size,$d.FreeSpace",
        )
        .unwrap_or_default();
        let mut fields = reading
            .split_whitespace()
            .map(|value| value.parse::<f64>().ok());
        let (total, free, load, disk_total, disk_free) = (
            fields.next().flatten(),
            fields.next().flatten(),
            fields.next().flatten(),
            fields.next().flatten(),
            fields.next().flatten(),
        );
        let cores = std::thread::available_parallelism().map_or(1, |count| count.get()) as f64;
        return Measurement {
            memory_total_mib: total.map(|kib| (kib / 1024.0) as u64),
            memory_used_mib: total
                .zip(free)
                .map(|(total, free)| ((total - free).max(0.0) / 1024.0) as u64),
            cpu_used_millis: load.map(|percent| (percent / 100.0 * cores * 1000.0) as u64),
            uptime_seconds: None,
            disk_total_bytes: disk_total.map(|bytes| bytes as u64),
            disk_free_bytes: disk_free.map(|bytes| bytes as u64),
        };
    }
    #[allow(unreachable_code)]
    Measurement::default()
}

/// Runs one PowerShell expression and returns its trimmed output.
#[cfg(target_os = "windows")]
fn powershell(script: &str) -> Option<String> {
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Active, wired and compressed pages: memory the system can't reclaim.
#[cfg(target_os = "macos")]
fn vm_stat_used_mib(output: &str) -> Option<u64> {
    let page = output
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    let pages = |name: &str| {
        output
            .lines()
            .find(|line| line.starts_with(name))
            .and_then(|line| line.rsplit(':').next())
            .and_then(|value| value.trim().trim_end_matches('.').parse::<u64>().ok())
    };
    let used = pages("Pages active")?
        + pages("Pages wired down")?
        + pages("Pages occupied by compressor").unwrap_or(0);
    Some(used * page / (1024 * 1024))
}

fn millis(load: &str) -> Option<u64> {
    load.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| (value * 1000.0) as u64)
}

pub fn root_domain(name: &str, measurement: Measurement) -> anyhow::Result<ObservedDomain> {
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
            memory_mib: measurement
                .memory_total_mib
                .map(BrowserUint53::new)
                .transpose()?,
            disk_bytes: measurement
                .disk_total_bytes
                .map(BrowserUint53::new)
                .transpose()?,
        },
    })
}

pub fn root_sample(
    observed_at: WireTimestamp,
    measurement: Measurement,
) -> anyhow::Result<ObservedSample> {
    let known = measurement.memory_used_mib.is_some()
        || measurement.cpu_used_millis.is_some()
        || measurement.disk_free_bytes.is_some();
    Ok(ObservedSample {
        domain_local_key: UpstreamText::new(ROOT_DOMAIN)?,
        observed_at,
        coverage: if known {
            SampleCoverage::Partial
        } else {
            SampleCoverage::Unknown
        },
        cpu_used_millis: measurement
            .cpu_used_millis
            .map(BrowserUint53::new)
            .transpose()?,
        memory_used_mib: measurement
            .memory_used_mib
            .map(BrowserUint53::new)
            .transpose()?,
        disk_free_bytes: measurement
            .disk_free_bytes
            .map(BrowserUint53::new)
            .transpose()?,
        uptime_seconds: measurement
            .uptime_seconds
            .map(BrowserUint53::new)
            .transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_reaches_the_domain_budget_and_the_sample() -> anyhow::Result<()> {
        use gridops_core::fleet::protocol::primitives::WireTimestamp as Timestamp;
        let measurement = Measurement {
            memory_total_mib: Some(1024),
            memory_used_mib: Some(512),
            cpu_used_millis: Some(1000),
            uptime_seconds: Some(60),
            disk_total_bytes: Some(1_000_000_000_000),
            disk_free_bytes: Some(400_000_000_000),
        };
        let domain = root_domain("io", measurement)?;
        assert_eq!(
            domain.capacity.disk_bytes.map(|value| value.get()),
            Some(1_000_000_000_000)
        );
        let sample = root_sample(
            Timestamp::from_millis(1).map_err(|_| anyhow::anyhow!("timestamp out of range"))?,
            measurement,
        )?;
        assert_eq!(
            sample.disk_free_bytes.map(|value| value.get()),
            Some(400_000_000_000)
        );
        assert_eq!(sample.coverage, SampleCoverage::Partial);
        Ok(())
    }
}
