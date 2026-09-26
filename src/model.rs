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
}

impl Default for ProfileOptions {
    fn default() -> Self {
        Self {
            speed: SpeedPreset::Balanced,
            picture: PicturePreset::Phone,
            window: WindowPreset::Remember,
            cores: 4,
            memory_mb: 4096,
            dpi: 320,
            adb_port: None,
            gpu_mode: "auto".into(),
            cold_boot: false,
            host_keyboard: true,
            device_frame: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct HostStats {
    pub cpu_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
}
