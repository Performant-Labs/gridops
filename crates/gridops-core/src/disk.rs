//! Filesystem capacity, read through `df -Pk`, which every supported Unix
//! ships. Windows callers shell out differently and do not use this module.

/// Parses POSIX `df -Pk` output into total and available bytes of the first
/// data row.
#[must_use]
pub fn parse_df_blocks(output: &str) -> Option<(u64, u64)> {
    let line = output.lines().nth(1)?;
    let mut fields = line.split_whitespace();
    let filesystem = fields.next()?;
    // A filesystem name starts with a slash or a drive/overlay style tag;
    // anything else means the row is not machine-readable data.
    if filesystem.starts_with('/') || filesystem.contains(':') {
        let total: u64 = fields.next()?.parse().ok()?;
        fields.next()?; // used
        let available: u64 = fields.next()?.parse().ok()?;
        return Some((total.saturating_mul(1024), available.saturating_mul(1024)));
    }
    None
}

/// The total and free bytes of the filesystem holding `path`.
#[cfg(unix)]
pub fn filesystem_bytes(path: &std::path::Path) -> Option<(u64, u64)> {
    let output = std::process::Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_df_blocks(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

/// Alarm levels for a filesystem, mirroring the manager's runner-host
/// watermark semantics: warn at 10 GiB or 10% free, escalate at 5 GiB or
/// 5%, whichever bound is larger for the disk at hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskAlarm {
    Warning,
    Critical,
}

const GIB: u64 = 1024 * 1024 * 1024;

#[must_use]
pub fn alarm_for(total_bytes: u64, free_bytes: u64) -> Option<DiskAlarm> {
    if total_bytes == 0 {
        return None;
    }
    let warn_floor = (10 * GIB).max(total_bytes / 10);
    let critical_floor = (5 * GIB).max(total_bytes / 20);
    if free_bytes < critical_floor {
        Some(DiskAlarm::Critical)
    } else if free_bytes < warn_floor {
        Some(DiskAlarm::Warning)
    } else {
        None
    }
}

#[must_use]
pub fn gib(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0 / 1024.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_posix_df_output() {
        let linux = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n\
                     /dev/sda1         982873244 812345672  170527572      83% /\n";
        assert_eq!(
            parse_df_blocks(linux),
            Some((982_873_244 * 1024, 170_527_572 * 1024))
        );
        let mac = "Filesystem   1024-blocks      Used Available Capacity  Mounted on\n\
                   /dev/disk3s1s1  981243972 690417912 290689060    71%    /\n";
        assert_eq!(
            parse_df_blocks(mac),
            Some((981_243_972 * 1024, 290_689_060 * 1024))
        );
    }

    #[test]
    fn rejects_output_that_is_not_a_data_row() {
        assert_eq!(parse_df_blocks(""), None);
        assert_eq!(
            parse_df_blocks("Filesystem 1024-blocks Used Available Capacity Mounted on\n"),
            None
        );
    }

    #[test]
    fn disk_alarms_follow_the_watermark_semantics() {
        use super::DiskAlarm;
        // A big disk: the percentage bounds bind.
        let big = 500 * GIB;
        assert_eq!(super::alarm_for(big, big * 9 / 10), None);
        assert_eq!(
            super::alarm_for(big, big / 10 - 1),
            Some(DiskAlarm::Warning)
        );
        assert_eq!(
            super::alarm_for(big, big / 20 - 1),
            Some(DiskAlarm::Critical)
        );
        // A small disk: the absolute floors bind (warn below 10 GiB free,
        // critical below 5 GiB free).
        let small = 40 * GIB;
        assert_eq!(super::alarm_for(small, 11 * GIB), None);
        assert_eq!(super::alarm_for(small, 9 * GIB), Some(DiskAlarm::Warning));
        assert_eq!(super::alarm_for(small, 4 * GIB), Some(DiskAlarm::Critical));
        // Unknown capacity never alarms.
        assert_eq!(super::alarm_for(0, 0), None);
    }

    #[test]
    #[cfg(unix)]
    fn reads_the_current_filesystem() -> anyhow::Result<()> {
        let Some((total, free)) = filesystem_bytes(std::path::Path::new("/")) else {
            anyhow::bail!("df on / must succeed");
        };
        assert!(total > 0);
        assert!(free > 0 && free <= total);
        Ok(())
    }
}
