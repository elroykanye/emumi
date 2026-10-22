use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_LOG_TAIL_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryOutcome {
    NotSnapshotFailure,
    SnapshotMissing,
    Quarantined,
}

pub fn recover_owned_snapshot_failure(profile: &str, now: u64) -> Result<RecoveryOutcome, String> {
    validate_profile_name(profile)?;
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&home).join(".local/state"));
    let log_path = state.join("emumi/emulators").join(format!("{profile}.log"));
    let log_tail = read_tail(&log_path)?;
    recover_snapshot_in(
        &PathBuf::from(home).join(".android/avd"),
        profile,
        &log_tail,
        now,
    )
}

pub fn recover_snapshot_in(
    avd_root: &Path,
    profile: &str,
    owned_log: &str,
    now: u64,
) -> Result<RecoveryOutcome, String> {
    validate_profile_name(profile)?;
    if !is_snapshot_load_failure(owned_log) {
        return Ok(RecoveryOutcome::NotSnapshotFailure);
    }
    let canonical_root = avd_root
        .canonicalize()
        .map_err(|error| format!("Could not validate the AVD root: {error}"))?;
    let profile_dir = avd_root.join(format!("{profile}.avd"));
    let snapshots = profile_dir.join("snapshots");
    let source = snapshots.join("default_boot");
    if !source.exists() {
        return Ok(RecoveryOutcome::SnapshotMissing);
    }
    for (label, path) in [
        ("profile", &profile_dir),
        ("snapshot directory", &snapshots),
        ("default snapshot", &source),
    ] {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("Could not validate {label}: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!("Unsafe {label} path"));
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Could not resolve {label}: {error}"))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(format!("Unsafe {label} path outside the AVD root"));
        }
    }
    let quarantine = snapshots.join(format!("default_boot.emumi-quarantine-{now}"));
    if quarantine.exists() {
        return Err("Snapshot quarantine destination already exists".into());
    }
    fs::rename(&source, &quarantine)
        .map_err(|error| format!("Could not quarantine the failed snapshot: {error}"))?;
    Ok(RecoveryOutcome::Quarantined)
}

fn is_snapshot_load_failure(log: &str) -> bool {
    log.contains("goldfish_pipe")
        || log.contains("Failed to load snapshot")
        || log.contains("error while loading state")
}

fn validate_profile_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        || name == "."
        || name == ".."
    {
        return Err("Unsafe Android profile name".into());
    }
    Ok(())
}

fn read_tail(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("Could not read the owned emulator log: {error}"))?;
    let length = file
        .metadata()
        .map_err(|error| format!("Could not inspect the owned emulator log: {error}"))?
        .len();
    let start = length.saturating_sub(MAX_LOG_TAIL_BYTES);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| format!("Could not seek the owned emulator log: {error}"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read the owned emulator log: {error}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::{RecoveryOutcome, recover_snapshot_in};
    use std::{fs, os::unix::fs::symlink};

    fn avd_root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "emumi-snapshot-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let avd = root.join("Device_4.avd");
        fs::create_dir_all(avd.join("snapshots/default_boot")).unwrap();
        fs::write(avd.join("snapshots/default_boot/ram.img"), "broken").unwrap();
        fs::write(avd.join("userdata-qemu.img"), "keep-me").unwrap();
        root
    }

    #[test]
    fn confirmed_goldfish_failure_quarantines_only_default_boot() {
        let root = avd_root("confirmed");
        let outcome = recover_snapshot_in(
            &root,
            "Device_4",
            "qemu-system-x86_64: error while loading state for instance 0x0 of device 'goldfish_pipe'\nFailed to load snapshot",
            123,
        )
        .unwrap();

        assert_eq!(outcome, RecoveryOutcome::Quarantined);
        assert!(!root.join("Device_4.avd/snapshots/default_boot").exists());
        assert!(
            root.join("Device_4.avd/snapshots/default_boot.emumi-quarantine-123")
                .is_dir()
        );
        assert_eq!(
            fs::read_to_string(root.join("Device_4.avd/userdata-qemu.img")).unwrap(),
            "keep-me"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_boot_failure_never_touches_snapshots_or_other_profiles() {
        let root = avd_root("generic");
        let other = root.join("Device_3.avd/snapshots/default_boot");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("ram.img"), "other").unwrap();

        let outcome =
            recover_snapshot_in(&root, "Device_4", "emulator exited with code 1", 124).unwrap();

        assert_eq!(outcome, RecoveryOutcome::NotSnapshotFailure);
        assert!(root.join("Device_4.avd/snapshots/default_boot").is_dir());
        assert_eq!(fs::read_to_string(other.join("ram.img")).unwrap(), "other");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsafe_profile_names_are_refused_before_any_filesystem_change() {
        let root = avd_root("unsafe");
        let error =
            recover_snapshot_in(&root, "../Device_4", "Failed to load snapshot", 125).unwrap_err();

        assert!(error.contains("profile name"));
        assert!(root.join("Device_4.avd/snapshots/default_boot").is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_symlink_cannot_quarantine_another_profiles_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "emumi-snapshot-symlink-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let other = root.join("Other.avd/snapshots/default_boot");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("ram.img"), "unrelated").unwrap();
        symlink(root.join("Other.avd"), root.join("Device_4.avd")).unwrap();

        let error =
            recover_snapshot_in(&root, "Device_4", "Failed to load snapshot", 123).unwrap_err();

        assert!(error.contains("Unsafe profile"));
        assert_eq!(
            fs::read_to_string(other.join("ram.img")).unwrap(),
            "unrelated"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
