use std::{
    fs::{self, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

const O_NOFOLLOW: i32 = 0o400000;

#[derive(Debug, PartialEq, Eq)]
pub enum GuardOutcome {
    Absent,
    Unchanged,
    Truncated { previous_bytes: u64 },
    Refused(&'static str),
}

pub fn enforce_log_limit(path: &Path, max_bytes: u64) -> io::Result<GuardOutcome> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Ok(GuardOutcome::Refused("not a regular file"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(GuardOutcome::Absent),
        Err(error) => return Err(error),
    }

    let file = OpenOptions::new()
        .write(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Ok(GuardOutcome::Refused("not a regular file"));
    }
    let previous_bytes = metadata.len();
    if previous_bytes <= max_bytes {
        return Ok(GuardOutcome::Unchanged);
    }
    file.set_len(0)?;
    Ok(GuardOutcome::Truncated { previous_bytes })
}

pub fn default_log_path() -> Option<PathBuf> {
    let user = std::env::var("USER").ok()?;
    default_log_path_for(&std::env::temp_dir(), &user)
}

fn default_log_path_for(temp_dir: &Path, user: &str) -> Option<PathBuf> {
    (!user.is_empty()
        && user
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character)))
    .then(|| {
        temp_dir
            .join(format!("android-{user}"))
            .join("netsimd/netsim_stderr.log")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink, time::SystemTime};

    fn test_root(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("emumi-{name}-{}-{unique}", std::process::id()))
    }

    #[test]
    fn truncates_only_when_the_netsimd_log_exceeds_the_limit() {
        let root = test_root("netsimd-log-limit");
        fs::create_dir_all(&root).unwrap();
        let log = root.join("netsim_stderr.log");
        fs::write(&log, b"123456789").unwrap();

        assert_eq!(
            enforce_log_limit(&log, 8).unwrap(),
            GuardOutcome::Truncated { previous_bytes: 9 }
        );
        assert_eq!(fs::metadata(&log).unwrap().len(), 0);

        fs::write(&log, b"12345678").unwrap();
        assert_eq!(enforce_log_limit(&log, 8).unwrap(), GuardOutcome::Unchanged);
        assert_eq!(fs::metadata(&log).unwrap().len(), 8);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_a_symlink_instead_of_truncating_its_target() {
        let root = test_root("netsimd-log-symlink");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target.log");
        let link = root.join("netsim_stderr.log");
        fs::write(&target, b"123456789").unwrap();
        symlink(&target, &link).unwrap();

        assert_eq!(
            enforce_log_limit(&link, 8).unwrap(),
            GuardOutcome::Refused("not a regular file")
        );
        assert_eq!(fs::read(&target).unwrap(), b"123456789");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn derives_only_a_valid_users_netsimd_log_path() {
        assert_eq!(
            default_log_path_for(Path::new("/tmp"), "elroy"),
            Some(Path::new("/tmp/android-elroy/netsimd/netsim_stderr.log").to_path_buf())
        );
        assert_eq!(default_log_path_for(Path::new("/tmp"), "../root"), None);
    }
}
