use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AndroidProfile {
    pub name: String,
    pub device_name: String,
    pub api_level: Option<u32>,
    pub resolution: Option<String>,
    pub running_serial: Option<String>,
}

impl AndroidProfile {
    pub fn is_running(&self) -> bool {
        self.running_serial.is_some()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpeedPreset {
    Efficient,
    #[default]
    LeanGaming,
    Balanced,
    Fast,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PicturePreset {
    Compact,
    #[default]
    Phone,
    Sharp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowPreset {
    #[default]
    Remember,
    Portrait,
    Landscape,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileOptions {
    pub speed: SpeedPreset,
    pub picture: PicturePreset,
    pub window: WindowPreset,
    pub cores: u8,
    pub memory_mb: u32,
    pub dpi: u16,
    pub adb_port: Option<u16>,
    pub gpu_mode: String,
    pub cold_boot: bool,
    pub host_keyboard: bool,
    pub device_frame: bool,
    pub rectangular_display: bool,
    pub mute_audio: bool,
    pub window_scale: Option<f32>,
    pub headless_automation: bool,
    pub disable_vulkan: bool,
    pub refresh_rate_hz: u16,
    pub suspend_store_during_automation: bool,
}

impl Default for ProfileOptions {
    fn default() -> Self {
        Self {
            speed: SpeedPreset::LeanGaming,
            picture: PicturePreset::Phone,
            window: WindowPreset::Remember,
            cores: 2,
            memory_mb: 4096,
            dpi: 240,
            adb_port: None,
            gpu_mode: "host-intel".into(),
            cold_boot: false,
            host_keyboard: true,
            device_frame: false,
            rectangular_display: true,
            mute_audio: true,
            window_scale: Some(0.55),
            headless_automation: true,
            disable_vulkan: true,
            refresh_rate_hz: 30,
            suspend_store_during_automation: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProfileOptions;

    #[test]
    fn older_profiles_inherit_the_fixed_lean_device_defaults() {
        let options: ProfileOptions =
            serde_json::from_str(r#"{"speed":"LeanGaming","cores":2,"memory_mb":4096}"#).unwrap();

        assert_eq!(options.refresh_rate_hz, 30);
        assert!(options.suspend_store_during_automation);
        assert_eq!(options.gpu_mode, "host-intel");
        assert!(options.disable_vulkan);
        assert_eq!(options.memory_mb, 4096);
    }
}
