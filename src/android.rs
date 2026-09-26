use crate::model::AndroidProfile;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Default)]
pub struct AndroidTools {
    pub sdk_root: Option<PathBuf>,
    pub java_home: Option<PathBuf>,
    pub emulator: Option<PathBuf>,
    pub adb: Option<PathBuf>,
    pub avd_manager: Option<PathBuf>,
    pub kvm_available: bool,
}

impl AndroidTools {
    pub fn detect(configured_sdk: &str, configured_jdk: &str) -> Self {
        let sdk_root = first_directory(
            (!configured_sdk.trim().is_empty()).then(|| PathBuf::from(configured_sdk.trim())),
            [
                std::env::var_os("ANDROID_HOME").map(PathBuf::from),
                std::env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from),
                home_path("Android/Sdk"),
                home_path(".local/share/emumi/android-sdk"),
            ],
        );
        let java_home = first_directory(
            (!configured_jdk.trim().is_empty()).then(|| PathBuf::from(configured_jdk.trim())),
            [
                std::env::var_os("JAVA_HOME").map(PathBuf::from),
                path_command("java").and_then(|path| {
                    path.canonicalize()
                        .unwrap_or(path)
                        .parent()?
                        .parent()
                        .map(Path::to_path_buf)
                }),
                None,
                None,
            ],
        );
        let emulator = sdk_root
            .as_ref()
            .map(|p| p.join("emulator/emulator"))
            .filter(|p| p.is_file());
        let adb = sdk_root
            .as_ref()
            .map(|p| p.join("platform-tools/adb"))
            .filter(|p| p.is_file())
            .or_else(|| path_command("adb"));
        let avd_manager = sdk_root
            .as_ref()
            .map(|p| p.join("cmdline-tools/latest/bin/avdmanager"))
            .filter(|p| p.is_file());
        Self {
            sdk_root,
            java_home,
            emulator,
            adb,
            avd_manager,
            kvm_available: Path::new("/dev/kvm").exists(),
        }
    }

    pub fn discover_profiles(&self) -> Vec<AndroidProfile> {
        let running = self.running_avds();
        let mut profiles = self
            .emulator
            .as_ref()
            .and_then(|emulator| Command::new(emulator).arg("-list-avds").output().ok())
            .filter(|output| output.status.success())
            .map(|output| parse_avd_list(&String::from_utf8_lossy(&output.stdout)))
            .unwrap_or_default()
            .into_iter()
            .map(|name| {
                let mut profile = read_avd_profile(&name);
                profile.running_serial = running.iter().find_map(|(serial, running_name)| {
                    (running_name == &name).then(|| serial.clone())
                });
                profile
            })
            .collect::<Vec<_>>();
        profiles.sort_by_key(|profile| profile.name.to_lowercase());
        profiles
    }

    pub fn start(&self, profile: &AndroidProfile, args: &[String]) -> Result<(), String> {
        let emulator = self
            .emulator
            .as_ref()
            .ok_or("Android Emulator was not found")?;
        let log_path = state_path("emulator.log").ok_or("HOME is not set")?;
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let log = fs::File::create(log_path).map_err(|err| err.to_string())?;
        Command::new(emulator)
            .arg("-avd")
            .arg(&profile.name)
            .args(args)
            .stdout(Stdio::from(log.try_clone().map_err(|err| err.to_string())?))
            .stderr(Stdio::from(log))
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    pub fn stop(&self, serial: &str) -> Result<(), String> {
        let adb = self.adb.as_ref().ok_or("ADB was not found")?;
        let status = Command::new(adb)
            .args(["-s", serial, "emu", "kill"])
            .status()
            .map_err(|err| err.to_string())?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| format!("ADB exited with {status}"))
    }

    fn running_avds(&self) -> HashMap<String, String> {
        let Some(adb) = &self.adb else {
            return HashMap::new();
        };
        let devices = Command::new(adb)
            .args(["devices"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| parse_adb_devices(&String::from_utf8_lossy(&output.stdout)))
            .unwrap_or_default();
        devices
            .into_iter()
            .filter_map(|serial| {
                let output = Command::new(adb)
                    .args(["-s", &serial, "emu", "avd", "name"])
                    .output()
                    .ok()?;
                let name = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .find(|line| !line.trim().is_empty() && line.trim() != "OK")?
                    .trim()
                    .to_owned();
                Some((serial, name))
            })
            .collect()
    }
}

pub fn parse_avd_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn parse_adb_devices(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| !line.starts_with("List of devices"))
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let serial = columns.next()?;
            (columns.next()? == "device").then(|| serial.to_owned())
        })
        .collect()
}

fn read_avd_profile(name: &str) -> AndroidProfile {
    let config = home_path(&format!(".android/avd/{name}.avd/config.ini"))
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let values = config
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect::<HashMap<_, _>>();
    let width = values.get("hw.lcd.width");
    let height = values.get("hw.lcd.height");
    AndroidProfile {
        name: name.to_owned(),
        device_name: values
            .get("hw.device.name")
            .copied()
            .unwrap_or("Android device")
            .replace('_', " "),
        api_level: values
            .get("image.sysdir.1")
            .and_then(|value| value.split("android-").nth(1))
            .and_then(|value| value.split('/').next())
            .and_then(|value| value.parse().ok()),
        resolution: width.zip(height).map(|(w, h)| format!("{w} × {h}")),
        running_serial: None,
    }
}

fn first_directory(first: Option<PathBuf>, rest: [Option<PathBuf>; 4]) -> Option<PathBuf> {
    first
        .into_iter()
        .chain(rest.into_iter().flatten())
        .find(|path| path.is_dir())
}

fn home_path(relative: &str) -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(relative))
}

fn state_path(relative: &str) -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(base).join("emumi").join(relative));
    }
    home_path(&format!(".local/state/emumi/{relative}"))
}

fn path_command(name: &str) -> Option<PathBuf> {
    let output = Command::new("which").arg(name).output().ok()?;
    output
        .status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_avd_names() {
        assert_eq!(
            parse_avd_list("Pixel_7\nTablet\n\n"),
            vec!["Pixel_7", "Tablet"]
        );
    }

    #[test]
    fn parses_only_online_adb_devices() {
        let input = "List of devices attached\nemulator-5554\tdevice\nemulator-5556\toffline\n";
        assert_eq!(parse_adb_devices(input), vec!["emulator-5554"]);
    }
}
