use crate::model::AndroidProfile;
use std::os::unix::process::CommandExt;
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemImage {
    pub package_id: String,
    pub api_level: u32,
    pub flavor: String,
    pub architecture: String,
}

impl SystemImage {
    pub fn label(&self) -> String {
        let flavor = match self.flavor.as_str() {
            "google_apis_playstore" => "Google Play",
            "google_apis" => "Google APIs",
            "default" => "Android Open Source",
            other => other,
        };
        format!(
            "Android {} · {} · {}",
            self.api_level, flavor, self.architecture
        )
    }
}

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

    pub fn installed_system_images(&self) -> Vec<SystemImage> {
        let Some(root) = self
            .sdk_root
            .as_ref()
            .map(|root| root.join("system-images"))
        else {
            return Vec::new();
        };
        let mut images = Vec::new();
        let Ok(api_dirs) = fs::read_dir(root) else {
            return images;
        };
        for api_dir in api_dirs.flatten().filter(|entry| entry.path().is_dir()) {
            let api_name = api_dir.file_name().to_string_lossy().into_owned();
            let Some(api_level) = api_name
                .strip_prefix("android-")
                .and_then(|v| v.parse().ok())
            else {
                continue;
            };
            let Ok(flavor_dirs) = fs::read_dir(api_dir.path()) else {
                continue;
            };
            for flavor_dir in flavor_dirs.flatten().filter(|entry| entry.path().is_dir()) {
                let flavor = flavor_dir.file_name().to_string_lossy().into_owned();
                let Ok(arch_dirs) = fs::read_dir(flavor_dir.path()) else {
                    continue;
                };
                for arch_dir in arch_dirs.flatten().filter(|entry| entry.path().is_dir()) {
                    let architecture = arch_dir.file_name().to_string_lossy().into_owned();
                    images.push(SystemImage {
                        package_id: format!("system-images;{api_name};{flavor};{architecture}"),
                        api_level,
                        flavor: flavor.clone(),
                        architecture,
                    });
                }
            }
        }
        images.sort_by(|left, right| right.api_level.cmp(&left.api_level));
        images
    }

    pub fn create_profile(
        &self,
        name: &str,
        device_id: &str,
        image: &SystemImage,
    ) -> Result<(), String> {
        validate_profile_name(name)?;
        let avd_manager = self
            .avd_manager
            .as_ref()
            .ok_or("AVD Manager was not found. Check Settings.")?;
        let mut child = Command::new(avd_manager)
            .args([
                "create",
                "avd",
                "--name",
                name,
                "--package",
                &image.package_id,
                "--device",
                device_id,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Could not start AVD Manager: {error}"))?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(b"no\n")
                .map_err(|error| format!("Could not configure the profile: {error}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("AVD Manager did not finish: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            let output = String::from_utf8_lossy(&output.stdout);
            let message = [error.trim(), output.trim()]
                .into_iter()
                .find(|message| !message.is_empty())
                .unwrap_or("AVD Manager could not create the profile");
            Err(message.lines().last().unwrap_or(message).to_owned())
        }
    }

    pub fn delete_profile(&self, name: &str) -> Result<(), String> {
        validate_profile_name(name)?;
        let avd_manager = self
            .avd_manager
            .as_ref()
            .ok_or("AVD Manager was not found")?;
        let output = Command::new(avd_manager)
            .args(["delete", "avd", "--name", name])
            .output()
            .map_err(|error| format!("Could not start AVD Manager: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        }
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
        let mut command = Command::new(emulator);
        command
            .arg("-avd")
            .arg(&profile.name)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().map_err(|err| err.to_string())?))
            .stderr(Stdio::from(log))
            .process_group(0);
        command.spawn().map(|_| ()).map_err(|err| err.to_string())
    }

    pub fn configure_input_and_window(
        &self,
        profile_name: &str,
        host_keyboard: bool,
        device_frame: bool,
    ) -> Result<(), String> {
        validate_profile_name(profile_name)?;
        let path = home_path(&format!(".android/avd/{profile_name}.avd/config.ini"))
            .ok_or("HOME is not set")?;
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let text = set_ini_value(
            &text,
            "hw.keyboard",
            if host_keyboard { "yes" } else { "no" },
        );
        let text = set_ini_value(
            &text,
            "showDeviceFrame",
            if device_frame { "yes" } else { "no" },
        );
        fs::write(&path, text)
            .map_err(|error| format!("Could not update {}: {error}", path.display()))
    }

    pub fn configure_window_scale(&self, profile_name: &str, scale: f32) -> Result<(), String> {
        validate_profile_name(profile_name)?;
        let path = home_path(&format!(
            ".android/avd/{profile_name}.avd/emulator-user.ini"
        ))
        .ok_or("HOME is not set")?;
        let text = fs::read_to_string(&path).unwrap_or_default();
        let scale = scale.clamp(0.45, 1.0);
        let text = set_ini_value(&text, "window.scale", &format!("{scale:.6}"));
        fs::write(&path, text)
            .map_err(|error| format!("Could not update {}: {error}", path.display()))
    }

    pub fn forget_window_scale(&self, profile_name: &str) -> Result<(), String> {
        validate_profile_name(profile_name)?;
        let path = home_path(&format!(
            ".android/avd/{profile_name}.avd/emulator-user.ini"
        ))
        .ok_or("HOME is not set")?;
        let Ok(text) = fs::read_to_string(&path) else {
            return Ok(());
        };
        fs::write(&path, remove_ini_value(&text, "window.scale"))
            .map_err(|error| format!("Could not update {}: {error}", path.display()))
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

pub fn validate_profile_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give this Android a name.".into());
    }
    if name.len() > 48 {
        return Err("Use a name shorter than 49 characters.".into());
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err("Use only letters, numbers, dashes, and underscores.".into());
    }
    Ok(())
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

fn set_ini_value(text: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut lines = text
        .lines()
        .map(|line| {
            if line
                .split_once('=')
                .is_some_and(|(candidate, _)| candidate.trim() == key)
            {
                found = true;
                format!("{key}={value}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>();
    if !found {
        lines.push(format!("{key}={value}"));
    }
    format!("{}\n", lines.join("\n"))
}

fn remove_ini_value(text: &str, key: &str) -> String {
    let lines = text
        .lines()
        .filter(|line| {
            !line
                .split_once('=')
                .is_some_and(|(candidate, _)| candidate.trim() == key)
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
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

    #[test]
    fn validates_safe_profile_names() {
        assert!(validate_profile_name("Pixel_7_work").is_ok());
        assert!(validate_profile_name("bad profile").is_err());
        assert!(validate_profile_name("../bad").is_err());
    }

    #[test]
    fn updates_ini_values_without_duplicates() {
        let updated = set_ini_value("hw.keyboard=no\nfoo=bar\n", "hw.keyboard", "yes");
        assert_eq!(updated, "hw.keyboard=yes\nfoo=bar\n");
        let inserted = set_ini_value("foo=bar\n", "showDeviceFrame", "no");
        assert_eq!(inserted, "foo=bar\nshowDeviceFrame=no\n");
    }

    #[test]
    fn removes_a_remembered_ini_value() {
        let updated = remove_ini_value("window.x = 10\nwindow.scale = 0.7\n", "window.scale");
        assert_eq!(updated, "window.x = 10\n");
    }
}
