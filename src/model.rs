use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AndroidProfile {
    pub name: String,
    pub device_name: String,
    pub api_level: Option<u32>,
    pub resolution: Option<String>,
    pub running_serial: Option<String>,
}

impl AndroidProfile {
    pub fn subtitle(&self) -> String {
        let device = if self.device_name.is_empty() {
            "Android device"
        } else {
            &self.device_name
        };
        match self.api_level {
            Some(api) => format!("{device}  ·  API {api}"),
            None => device.to_owned(),
        }
    }

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

impl SpeedPreset {
    pub const ALL: [Self; 3] = [Self::Efficient, Self::Balanced, Self::Fast];

    pub fn label(self) -> &'static str {
        match self {
            Self::Efficient => "Efficient",
            Self::Balanced => "Balanced",
            Self::Fast => "Fast",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PicturePreset {
    Compact,
    #[default]
    Phone,
    Sharp,
}

impl PicturePreset {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Phone, Self::Sharp];

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Phone => "Phone",
            Self::Sharp => "Sharp",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowPreset {
    #[default]
    Remember,
    Portrait,
    Landscape,
}

impl WindowPreset {
    pub const ALL: [Self; 3] = [Self::Remember, Self::Portrait, Self::Landscape];

    pub fn label(self) -> &'static str {
        match self {
            Self::Remember => "Remember",
            Self::Portrait => "Portrait",
            Self::Landscape => "Landscape",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub at: SystemTime,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct HostStats {
    pub cpu_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
}
