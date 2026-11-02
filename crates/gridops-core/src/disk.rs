//! Filesystem capacity, read through `df -Pk`, which every supported Unix
//! ships. Windows callers shell out differently and do not use this module.

/// Parses POSIX `df -Pk` output into total and available bytes of the first
/// data row.
///
/// A row reads `<filesystem> <blocks> <used> <available> <capacity> <mount>`.
/// The filesystem is whatever the kernel calls it (`/dev/sda1`, `overlay`,
/// `tmpfs`, `pool/dataset`, `host:/export`), so the row is found by its three
/// numbers followed by the capacity (`83%`, or `-` where a filesystem has none),
/// not by the shape of the name.
#[must_use]
pub fn parse_df_blocks(output: &str) -> Option<(u64, u64)> {
    let line = output.lines().nth(1)?;
    let fields: Vec<&str> = line.split_whitespace().collect();
    (3..fields.len()).find_map(|capacity| {
        let is_capacity = fields[capacity] == "-" || fields[capacity].ends_with('%');
        if !is_capacity {
            return None;
        }
        let total: u64 = fields[capacity - 3].parse().ok()?;
        let _used: u64 = fields[capacity - 2].parse().ok()?;
        let available: u64 = fields[capacity - 1].parse().ok()?;
        Some((total.saturating_mul(1024), available.saturating_mul(1024)))
    })
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
    fn parses_filesystems_that_are_not_device_paths() {
        let header = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n";
        for (row, total, free) in [
            // The root of a Docker container.
            (
                "overlay  506771172 326688324 180066464  65% /",
                506_771_172_u64,
                180_066_464_u64,
            ),
            ("tmpfs  1024 24 1000   3% /run", 1024, 1000),
            (
                "rpool/ROOT/ubuntu  900000 400000 500000  45% /",
                900_000,
                500_000,
            ),
            ("nas:/export/ci  2000 500 1500  25% /mnt/ci", 2000, 1500),
            ("none  100 100 0  100% /", 100, 0),
            ("fusefs  100 40 60  - /mnt/x", 100, 60),
            (
                "/dev/mapper/vg-root  100 40 60  40% /mnt/with space",
                100,
                60,
            ),
        ] {
            assert_eq!(
                parse_df_blocks(&format!("{header}{row}\n")),
                Some((total * 1024, free * 1024)),
                "{row}"
            );
        }
    }

    #[test]
    fn rejects_output_that_is_not_a_data_row() {
        assert_eq!(parse_df_blocks(""), None);
        assert_eq!(
            parse_df_blocks("Filesystem 1024-blocks Used Available Capacity Mounted on\n"),
            None
        );
        assert_eq!(
            parse_df_blocks(
                "Filesystem 1024-blocks Used Available Capacity Mounted on\noverlay - - - - /\n"
            ),
            None
        );
        assert_eq!(parse_df_blocks("header\nsomething 12 34 56 /\n"), None);
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
