use crate::model::ProfileOptions;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub android_sdk_path: String,
    pub jdk_path: String,
    pub profile_options: BTreeMap<String, ProfileOptions>,
}

impl AppConfig {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<PathBuf> {
        let path = config_path().ok_or_else(|| io::Error::other("HOME is not set"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(&path, body)?;
        Ok(path)
    }
}

pub fn config_path() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(base).join("emumi/config.json"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/emumi/config.json"))
}
