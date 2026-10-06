use crate::model::{PicturePreset, ProfileOptions};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::PathBuf};

const CURRENT_POLICY_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub android_sdk_path: String,
    pub jdk_path: String,
    pub profile_options: BTreeMap<String, ProfileOptions>,
    #[serde(default = "legacy_policy_version")]
    pub policy_version: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            android_sdk_path: String::new(),
            jdk_path: String::new(),
            profile_options: BTreeMap::new(),
            policy_version: CURRENT_POLICY_VERSION,
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let mut config: Self = fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        if config.migrate_policy() {
            let _ = config.save();
        }
        config
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

    fn migrate_policy(&mut self) -> bool {
        if self.policy_version >= CURRENT_POLICY_VERSION {
            return false;
        }
        for options in self.profile_options.values_mut() {
            options.picture = PicturePreset::Phone;
            options.cores = 2;
            options.memory_mb = 4096;
            options.dpi = 240;
            if options.gpu_mode != "host-intel" {
                options.gpu_mode = "host".into();
            }
            options.mute_audio = true;
            options.headless_automation = true;
            options.refresh_rate_hz = 30;
            options.host_memory_policy = true;
            options.memory_high_mb = 5632;
            options.memory_max_mb = 6656;
            options.memory_swap_max_mb = 1024;
        }
        self.policy_version = CURRENT_POLICY_VERSION;
        true
    }
}

fn legacy_policy_version() -> u32 {
    0
}

pub fn config_path() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(base).join("emumi/config.json"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/emumi/config.json"))
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, CURRENT_POLICY_VERSION};
    use crate::model::PicturePreset;

    #[test]
    fn migrates_existing_profiles_to_the_four_emulator_defaults_once() {
        let mut config: AppConfig = serde_json::from_str(
            r#"{"profile_options":{"Device_1":{"cores":8,"memory_mb":2048,"picture":"Sharp","gpu_mode":"auto","adb_port":5554,"cold_boot":true,"headless_automation":false,"mute_audio":false,"memory_high_mb":7168,"memory_max_mb":8192},"Device_Intel":{"cores":8,"memory_mb":4096,"gpu_mode":"host-intel","adb_port":5556}}}"#,
        )
        .unwrap();

        assert_eq!(config.policy_version, 0);
        assert!(config.migrate_policy());
        let options = &config.profile_options["Device_1"];
        assert_eq!(config.policy_version, CURRENT_POLICY_VERSION);
        assert_eq!(options.picture, PicturePreset::Phone);
        assert_eq!(options.cores, 2);
        assert_eq!(options.memory_mb, 4096);
        assert_eq!(options.gpu_mode, "host");
        assert_eq!(options.adb_port, Some(5554));
        assert!(options.cold_boot);
        assert!(options.headless_automation);
        assert!(options.mute_audio);
        assert_eq!(options.refresh_rate_hz, 30);
        assert!(options.host_memory_policy);
        assert_eq!(options.memory_high_mb, 5632);
        assert_eq!(options.memory_max_mb, 6656);
        assert_eq!(options.memory_swap_max_mb, 1024);
        let intel = &config.profile_options["Device_Intel"];
        assert_eq!(intel.cores, 2);
        assert_eq!(intel.gpu_mode, "host-intel");
        assert_eq!(intel.adb_port, Some(5556));
        assert!(!config.migrate_policy());
    }
}
