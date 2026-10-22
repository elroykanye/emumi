use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct ManagerGuard {
    _file: File,
    state_dir: PathBuf,
}

impl ManagerGuard {
    pub fn acquire() -> io::Result<Self> {
        Self::acquire_in(&state_directory()?)
    }

    pub fn acquire_in(state_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(state_dir)?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(state_dir.join("manager.lock"))?;
        file.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => {
                io::Error::new(io::ErrorKind::AlreadyExists, "EmuMi is already running")
            }
            fs::TryLockError::Error(error) => error,
        })?;
        Ok(Self {
            _file: file,
            state_dir: state_dir.to_owned(),
        })
    }

    pub fn publish_endpoint(&self, url: &str) -> io::Result<()> {
        publish_endpoint_atomic(&self.state_dir, url)
    }
}

impl Drop for ManagerGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.state_dir.join("api-url"));
    }
}

pub fn state_directory() -> io::Result<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|base| base.join("emumi"))
        .ok_or_else(|| io::Error::other("HOME is not set"))
}

pub fn publish_endpoint_atomic(state_dir: &Path, url: &str) -> io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let temporary = state_dir.join(format!("api-url.tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    writeln!(file, "{url}")?;
    file.sync_all()?;
    fs::rename(&temporary, state_dir.join("api-url"))?;
    File::open(state_dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ManagerGuard, publish_endpoint_atomic};
    use std::fs;

    fn temp_root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "emumi-manager-guard-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_second_manager_cannot_claim_the_same_state_directory() {
        let root = temp_root("exclusive");
        let first = ManagerGuard::acquire_in(&root).unwrap();

        let error = ManagerGuard::acquire_in(&root).unwrap_err();

        assert!(error.to_string().contains("already running"));
        drop(first);
        assert!(ManagerGuard::acquire_in(&root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn endpoint_publication_atomically_replaces_stale_content() {
        let root = temp_root("endpoint");
        fs::write(root.join("api-url"), "http://127.0.0.1:1\n").unwrap();

        publish_endpoint_atomic(&root, "http://127.0.0.1:45997").unwrap();

        assert_eq!(
            fs::read_to_string(root.join("api-url")).unwrap(),
            "http://127.0.0.1:45997\n"
        );
        assert!(
            fs::read_dir(&root)
                .unwrap()
                .all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with("api-url.tmp"))
        );
        fs::remove_dir_all(root).unwrap();
    }
}
