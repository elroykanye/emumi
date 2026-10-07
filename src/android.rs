use crate::model::{AndroidProfile, LaunchAdmission, ProfileOptions};
use std::os::unix::process::CommandExt;
use std::{
    collections::HashMap,
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
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
    pub systemd_run_available: bool,
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
            systemd_run_available: path_command("systemd-run").is_some()
                && path_command("systemctl").is_some(),
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
        profiles.sort_by_key(|profile| {
            let created = home_path(&format!(".android/avd/{}.ini", profile.name))
                .and_then(|path| fs::metadata(path).ok())
                .and_then(|metadata| metadata.created().or_else(|_| metadata.modified()).ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis())
                .unwrap_or(u128::MAX);
            (created, profile.name.to_lowercase())
        });
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
        images.sort_by_key(|image| std::cmp::Reverse(image.api_level));
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

    pub fn clone_profile(&self, source_name: &str, clone_name: &str) -> Result<(), String> {
        validate_profile_name(source_name)?;
        validate_profile_name(clone_name)?;
        if source_name == clone_name {
            return Err("Give the clone a different name.".into());
        }
        if self
            .running_avds()
            .values()
            .any(|running_name| running_name == source_name)
        {
            return Err("Stop the source Android before cloning it.".into());
        }

        let avd_root = home_path(".android/avd").ok_or("HOME is not set")?;
        self.clone_profile_in_root(source_name, clone_name, &avd_root)
    }

    fn clone_profile_in_root(
        &self,
        source_name: &str,
        clone_name: &str,
        avd_root: &Path,
    ) -> Result<(), String> {
        let source_dir = avd_root.join(format!("{source_name}.avd"));
        let source_ini = avd_root.join(format!("{source_name}.ini"));
        let clone_dir = avd_root.join(format!("{clone_name}.avd"));
        let clone_ini = avd_root.join(format!("{clone_name}.ini"));
        if !source_dir.is_dir() || !source_ini.is_file() {
            return Err(format!("Source Android {source_name} is incomplete."));
        }
        if clone_dir.exists() || clone_ini.exists() {
            return Err(format!("An Android named {clone_name} already exists."));
        }

        let operation = format!("{}-{}", std::process::id(), now_millis());
        let staging_dir = avd_root.join(format!(".{clone_name}.avd.emumi-{operation}"));
        let staging_ini = avd_root.join(format!(".{clone_name}.ini.emumi-{operation}"));
        let result = (|| {
            let output = Command::new("cp")
                .args(["--archive", "--reflink=auto", "--sparse=always", "--"])
                .arg(&source_dir)
                .arg(&staging_dir)
                .output()
                .map_err(|error| format!("Could not start the clone copy: {error}"))?;
            if !output.status.success() {
                let detail = String::from_utf8_lossy(&output.stderr);
                return Err(format!(
                    "Could not copy Android data: {}",
                    detail.trim().lines().last().unwrap_or("copy failed")
                ));
            }

            remove_clone_runtime_state(&staging_dir)?;
            let config_path = staging_dir.join("config.ini");
            let config = fs::read_to_string(&config_path)
                .map_err(|error| format!("Could not read cloned config: {error}"))?;
            let config = set_ini_value(&config, "avd.ini.displayname", clone_name);
            let config = set_ini_value(&config, "avd.id", clone_name);
            let config = set_ini_value(&config, "avd.name", clone_name);
            fs::write(&config_path, config)
                .map_err(|error| format!("Could not update cloned config: {error}"))?;

            let outer = fs::read_to_string(&source_ini)
                .map_err(|error| format!("Could not read source registration: {error}"))?;
            let outer = set_ini_value(&outer, "path", &clone_dir.to_string_lossy());
            let outer = set_ini_value(&outer, "path.rel", &format!("avd/{clone_name}.avd"));
            fs::write(&staging_ini, outer)
                .map_err(|error| format!("Could not register cloned Android: {error}"))?;

            fs::rename(&staging_dir, &clone_dir)
                .map_err(|error| format!("Could not finish cloned Android data: {error}"))?;
            if let Err(error) = fs::rename(&staging_ini, &clone_ini) {
                let _ = fs::remove_dir_all(&clone_dir);
                return Err(format!(
                    "Could not finish cloned Android registration: {error}"
                ));
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging_dir);
            let _ = fs::remove_file(&staging_ini);
        }
        result
    }

    pub fn start(
        &self,
        profile: &AndroidProfile,
        args: &[String],
        options: &ProfileOptions,
        _admission: LaunchAdmission,
    ) -> Result<(), String> {
        let emulator = self
            .emulator
            .as_ref()
            .ok_or("Android Emulator was not found")?;
        clear_stale_runtime_locks(&profile.name)?;
        let runtime_image = runtime_system_image(&profile.name);
        if let Some(image) = runtime_image.as_deref() {
            prepare_profile_runtime_overlay(&profile.name, image)?;
        }
        validate_memory_policy(options)?;
        let log_path =
            state_path(&format!("emulators/{}.log", profile.name)).ok_or("HOME is not set")?;
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let log = fs::File::create(log_path).map_err(|err| err.to_string())?;
        let emulator_args = emulator_launch_args(&profile.name, args, runtime_image);
        let launch = launch_spec(emulator, &emulator_args, &profile.name, options)?;
        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().map_err(|err| err.to_string())?))
            .stderr(Stdio::from(log))
            .process_group(0);
        if options.gpu_mode == "host-intel" {
            let (device, manifest) = intel_renderer()?;
            configure_intel_renderer(&mut command, &device, &manifest);
        } else if options.gpu_mode == "host" && nvidia_gpu_available() {
            command
                .env("__NV_PRIME_RENDER_OFFLOAD", "1")
                .env("__GLX_VENDOR_LIBRARY_NAME", "nvidia")
                .env("__VK_LAYER_NV_optimus", "NVIDIA_only");
        }
        command.spawn().map(|_| ()).map_err(|err| err.to_string())
    }

    /// Android's Pixel hardware profiles can re-apply their device-shape overlay
    /// several times while System UI settles. Keep normalizing the display for
    /// the whole post-boot window instead of doing a single best-effort pass.
    /// This is safe to run for newly started and already-running profiles.
    pub fn watch_rectangular_display_after_start(&self, profile_name: &str) {
        let Some(adb) = self.adb.clone() else {
            return;
        };
        let profile_name = profile_name.to_owned();
        thread::spawn(move || {
            let mut found_serial = None;
            for _ in 0..120 {
                let devices = Command::new(&adb)
                    .args(["devices"])
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .map(|output| parse_adb_devices(&String::from_utf8_lossy(&output.stdout)))
                    .unwrap_or_default();
                for candidate in devices {
                    if running_avd_name(&adb, &candidate).as_deref() != Some(profile_name.as_str())
                    {
                        continue;
                    }
                    let booted = Command::new(&adb)
                        .args(["-s", &candidate, "shell", "getprop", "sys.boot_completed"])
                        .output()
                        .ok()
                        .filter(|output| output.status.success())
                        .is_some_and(|output| {
                            String::from_utf8_lossy(&output.stdout).trim() == "1"
                        });
                    if !booted {
                        continue;
                    }
                    found_serial = Some(candidate);
                    break;
                }
                if found_serial.is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(500));
            }

            let Some(serial) = found_serial else {
                return;
            };
            // Pixel overlays may return after boot_completed while System UI is
            // still restoring state. Re-check for one minute so the final state,
            // not a transient state, is rectangular.
            for _ in 0..120 {
                if running_avd_name(&adb, &serial).as_deref() != Some(profile_name.as_str()) {
                    return;
                }
                let overlays = Command::new(&adb)
                    .args(overlay_list_args(&serial))
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .map(|output| {
                        enabled_display_shape_overlays(&String::from_utf8_lossy(&output.stdout))
                    })
                    .unwrap_or_default();
                for overlay in overlays {
                    let _ = Command::new(&adb)
                        .args([
                            "-s", &serial, "shell", "cmd", "overlay", "disable", "--user", "0",
                            &overlay,
                        ])
                        .status();
                }
                thread::sleep(Duration::from_millis(500));
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure_input_and_window(
        &self,
        profile_name: &str,
        host_keyboard: bool,
        device_frame: bool,
        display_width: u16,
        display_height: u16,
        display_dpi: u16,
        lean_gaming: bool,
    ) -> Result<(), String> {
        validate_profile_name(profile_name)?;
        let path = home_path(&format!(".android/avd/{profile_name}.avd/config.ini"))
            .ok_or("HOME is not set")?;
        let original = fs::read_to_string(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let text = set_ini_value(
            &original,
            "hw.keyboard",
            if host_keyboard { "yes" } else { "no" },
        );
        let text = set_ini_value(
            &text,
            "showDeviceFrame",
            if device_frame { "yes" } else { "no" },
        );
        let text = set_ini_value(&text, "hw.lcd.width", &display_width.to_string());
        let text = set_ini_value(&text, "hw.lcd.height", &display_height.to_string());
        let mut text = set_ini_value(&text, "hw.lcd.density", &display_dpi.to_string());
        // Whiteout's ARM translation path expects a normal phone hardware surface.
        // Removing sensors/cameras saves almost nothing and caused repeatable native startup aborts.
        let phone_hardware = [
            ("hw.audioInput", if lean_gaming { "no" } else { "yes" }),
            ("hw.audioOutput", if lean_gaming { "no" } else { "yes" }),
            ("hw.camera.back", "emulated"),
            ("hw.camera.front", "none"),
            ("hw.sensors.accelerometer", "yes"),
            ("hw.sensors.gyroscope", "yes"),
            ("hw.sensors.light", "yes"),
            ("hw.sensors.magnetic_field", "yes"),
            ("hw.sensors.orientation", "yes"),
            ("hw.sensors.pressure", "yes"),
            ("hw.sensors.proximity", "yes"),
        ];
        for (key, value) in phone_hardware {
            text = set_ini_value(&text, key, value);
        }
        if text != original {
            fs::write(&path, text)
                .map_err(|error| format!("Could not update {}: {error}", path.display()))?;
        }
        Ok(())
    }

    pub fn watch_lean_profile_after_start(
        &self,
        profile_name: &str,
        suspend_store_during_automation: bool,
    ) {
        let Some(adb) = self.adb.clone() else {
            return;
        };
        let profile_name = profile_name.to_owned();
        thread::spawn(move || {
            let Some(serial) = wait_for_booted_profile(&adb, &profile_name) else {
                return;
            };
            for setting in [
                "window_animation_scale",
                "transition_animation_scale",
                "animator_duration_scale",
            ] {
                let _ = Command::new(&adb)
                    .args([
                        "-s", &serial, "shell", "settings", "put", "global", setting, "0",
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = Command::new(&adb)
                .args([
                    "-s",
                    &serial,
                    "shell",
                    "cmd",
                    "netpolicy",
                    "set",
                    "restrict-background",
                    "true",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new(&adb)
                .args([
                    "-s",
                    &serial,
                    "shell",
                    "content",
                    "set",
                    "master_sync_enabled",
                    "false",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let mut packages = vec![
                "com.android.printspooler",
                "com.android.dreams.basic",
                "com.android.wallpaper.livepicker",
                "com.google.android.apps.wallpaper",
                "com.google.android.apps.youtube.music",
                "com.google.android.apps.photos",
                "com.google.android.apps.wellbeing",
                "com.google.android.googlequicksearchbox",
                "com.google.android.apps.messaging",
                "com.google.android.dialer",
                "com.google.android.tts",
                "com.google.android.projection.gearhead",
                "com.google.android.marvin.talkback",
                "com.google.android.accessibility.switchaccess",
                "com.google.android.apps.accessibility.voiceaccess",
                "com.google.android.apps.docs",
                "com.google.android.apps.maps",
                "com.google.android.apps.safetyhub",
                "com.google.android.as",
                "com.google.android.as.oss",
                "com.google.android.federatedcompute",
                "com.google.android.health.connect.backuprestore",
                "com.google.android.healthconnect.controller",
                "com.google.android.ondevicepersonalization.services",
                "com.google.android.deskclock",
                "com.android.chrome",
                "com.android.camera2",
            ];
            if suspend_store_during_automation {
                packages.extend(["com.android.vending", "com.google.android.apps.restore"]);
            } else {
                for package in ["com.android.vending", "com.google.android.apps.restore"] {
                    let _ = Command::new(&adb)
                        .args([
                            "-s", &serial, "shell", "pm", "enable", "--user", "0", package,
                        ])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
            for package in packages {
                let _ = Command::new(&adb)
                    .args([
                        "-s",
                        &serial,
                        "shell",
                        "pm",
                        "disable-user",
                        "--user",
                        "0",
                        package,
                    ])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        });
    }

    pub fn profile_boot_completed(&self, profile_name: &str) -> bool {
        let Some(adb) = &self.adb else {
            return false;
        };
        self.running_avds().into_iter().any(|(serial, name)| {
            name == profile_name
                && Command::new(adb)
                    .args(["-s", &serial, "shell", "getprop", "sys.boot_completed"])
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .is_some_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "1")
        })
    }

    pub fn configure_window_scale(&self, profile_name: &str, scale: f32) -> Result<(), String> {
        validate_profile_name(profile_name)?;
        let path = home_path(&format!(
            ".android/avd/{profile_name}.avd/emulator-user.ini"
        ))
        .ok_or("HOME is not set")?;
        let text = fs::read_to_string(&path).unwrap_or_default();
        let scale = scale.clamp(0.45, 1.0);
        let updated = set_ini_value(&text, "window.scale", &format!("{scale:.6}"));
        if updated != text {
            fs::write(&path, updated)
                .map_err(|error| format!("Could not update {}: {error}", path.display()))?;
        }
        Ok(())
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
                let name = running_avd_name(adb, &serial)?;
                Some((serial, name))
            })
            .collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct LaunchSpec {
    program: PathBuf,
    args: Vec<OsString>,
}

fn launch_spec(
    emulator: &Path,
    emulator_args: &[OsString],
    profile_name: &str,
    options: &ProfileOptions,
) -> Result<LaunchSpec, String> {
    if !options.host_memory_policy {
        return Ok(LaunchSpec {
            program: emulator.to_owned(),
            args: emulator_args.to_vec(),
        });
    }
    let systemd_run = path_command("systemd-run")
        .ok_or("Per-emulator memory policy needs systemd-run, but it was not found in PATH")?;
    let systemctl = path_command("systemctl")
        .ok_or("Per-emulator memory policy needs systemctl, but it was not found in PATH")?;
    let user_manager = Command::new(systemctl)
        .args(["--user", "show-environment"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Could not contact the user systemd manager: {error}"))?;
    if !user_manager.success() {
        return Err("Per-emulator memory policy needs an available user systemd manager".into());
    }
    Ok(systemd_scope_launch_spec(
        systemd_run,
        emulator,
        emulator_args,
        profile_name,
        now_millis(),
        options,
    ))
}

fn systemd_scope_launch_spec(
    systemd_run: PathBuf,
    emulator: &Path,
    emulator_args: &[OsString],
    profile_name: &str,
    launch_id: u128,
    options: &ProfileOptions,
) -> LaunchSpec {
    // netsimd is shared by emulator processes and can legitimately outlive the
    // AVD that first spawned it. A fixed scope name then makes systemd reject a
    // later restart even though the emulator itself is gone. Per-launch scope
    // identities preserve accounting without coupling restarts to helper life.
    let unit = format!("emumi-{profile_name}-{launch_id}");
    let mut args = vec![
        OsString::from("--user"),
        OsString::from("--scope"),
        OsString::from("--quiet"),
        OsString::from("--collect"),
        OsString::from(format!("--unit={unit}")),
        OsString::from(format!("--property=MemoryHigh={}M", options.memory_high_mb)),
        OsString::from(format!("--property=MemoryMax={}M", options.memory_max_mb)),
        OsString::from(format!(
            "--property=MemorySwapMax={}M",
            options.memory_swap_max_mb
        )),
        // Four active emulators receive at most 3.2 host CPUs in aggregate,
        // leaving capacity for Frostguard, the desktop and system services.
        OsString::from("--property=CPUQuota=80%"),
        // Emulator work stays responsive, but competes below normal desktop
        // applications when several Androids are busy at once.
        OsString::from("--property=CPUWeight=25"),
        OsString::from("--property=IOWeight=25"),
        OsString::from("--"),
        emulator.as_os_str().to_owned(),
    ];
    args.extend_from_slice(emulator_args);
    LaunchSpec {
        program: systemd_run,
        args,
    }
}

fn validate_memory_policy(options: &ProfileOptions) -> Result<(), String> {
    if !options.host_memory_policy {
        return Ok(());
    }
    if options.memory_high_mb < 4096 {
        return Err("MemoryHigh must be at least 4096 MB for the first safe trials".into());
    }
    if options.memory_max_mb < options.memory_high_mb {
        return Err("MemoryMax must be greater than or equal to MemoryHigh".into());
    }
    if options.memory_max_mb < options.memory_mb {
        return Err("MemoryMax cannot be lower than Android guest memory".into());
    }
    Ok(())
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

fn running_avd_name(adb: &Path, serial: &str) -> Option<String> {
    let output = Command::new(adb)
        .args(["-s", serial, "emu", "avd", "name"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.trim().is_empty() && line.trim() != "OK")
        .map(|line| line.trim().to_owned())
}

fn wait_for_booted_profile(adb: &Path, profile_name: &str) -> Option<String> {
    for _ in 0..120 {
        let devices = Command::new(adb)
            .args(["devices"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| parse_adb_devices(&String::from_utf8_lossy(&output.stdout)))
            .unwrap_or_default();
        for serial in devices {
            if running_avd_name(adb, &serial).as_deref() != Some(profile_name) {
                continue;
            }
            let booted = Command::new(adb)
                .args(["-s", &serial, "shell", "getprop", "sys.boot_completed"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .is_some_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "1");
            if booted {
                return Some(serial);
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
    None
}

fn enabled_display_shape_overlays(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("[x] "))
        .filter(|package| {
            package.starts_with("com.android.internal.emulation.")
                || package.starts_with("com.android.internal.display.cutout.emulation.")
                || package.starts_with("com.android.systemui.emulation.")
        })
        .map(str::to_owned)
        .collect()
}

fn overlay_list_args(serial: &str) -> [&str; 8] {
    [
        "-s", serial, "shell", "cmd", "overlay", "list", "--user", "0",
    ]
}

fn remove_clone_runtime_state(clone_dir: &Path) -> Result<(), String> {
    for relative in [
        "bootcompleted.ini",
        "emu-launch-params.txt",
        "hardware-qemu.ini",
        "hardware-qemu.ini.lock",
        "multiinstance.lock",
        "snapshot.lock.lock",
        "read-snapshot.txt",
    ] {
        let path = clone_dir.join(relative);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| format!("Could not clean cloned runtime state: {error}"))?;
        }
    }
    for relative in ["snapshots", "tmpAdbCmds"] {
        let path = clone_dir.join(relative);
        if path.exists() {
            fs::remove_dir_all(&path)
                .map_err(|error| format!("Could not clean cloned runtime state: {error}"))?;
        }
    }
    Ok(())
}

fn clear_stale_runtime_locks(profile_name: &str) -> Result<(), String> {
    let avd_root = home_path(".android/avd").ok_or("HOME is not set")?;
    clear_stale_runtime_locks_in(&avd_root, Path::new("/proc"), profile_name)
}

fn clear_stale_runtime_locks_in(
    avd_root: &Path,
    proc_root: &Path,
    profile_name: &str,
) -> Result<(), String> {
    let profile_dir = avd_root.join(format!("{profile_name}.avd"));
    let locks = [
        profile_dir.join("hardware-qemu.ini.lock"),
        profile_dir.join("multiinstance.lock"),
        profile_dir.join("snapshot.lock.lock"),
    ];
    let owner = locks.iter().find_map(|path| read_lock_pid(path));
    if owner.is_some_and(|pid| emulator_process_is_live(proc_root, pid, profile_name)) {
        return Err(format!(
            "{profile_name} already has a live emulator process; wait for it to finish starting"
        ));
    }
    for path in locks {
        if path.exists() {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "Could not remove stale emulator lock {}: {error}",
                    path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn nvidia_gpu_available() -> bool {
    Path::new("/proc/driver/nvidia/gpus").is_dir()
}

// Select both APIs: DRI_PRIME alone still lets gfxstream pick NVIDIA for Vulkan.
// Fail closed rather than silently falling back to a different GPU or the CPU.
fn intel_renderer() -> Result<(String, PathBuf), String> {
    let devices = fs::read_dir("/sys/bus/pci/devices")
        .map_err(|error| format!("Could not inspect Intel graphics: {error}"))?;
    let mut candidates = devices
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let vendor = fs::read_to_string(path.join("vendor")).ok()?;
            let class = fs::read_to_string(path.join("class")).ok()?;
            (vendor.trim() == "0x8086" && class.trim().starts_with("0x03"))
                .then(|| entry.file_name().to_string_lossy().replace([':', '.'], "_"))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    let device = candidates
        .first()
        .ok_or("Intel hardware graphics requested, but no Intel display adapter was found")?;
    let manifest = [
        "/usr/share/vulkan/icd.d/intel_icd.json",
        "/usr/share/vulkan/icd.d/intel_icd.x86_64.json",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
    .ok_or("Intel hardware graphics needs the Mesa Intel Vulkan driver manifest")?;
    Ok((format!("pci-{device}"), manifest))
}

fn configure_intel_renderer(command: &mut Command, device: &str, manifest: &Path) {
    command
        .env_remove("__NV_PRIME_RENDER_OFFLOAD")
        .env_remove("__NV_PRIME_RENDER_OFFLOAD_PROVIDER")
        .env_remove("__VK_LAYER_NV_optimus")
        .env("__GLX_VENDOR_LIBRARY_NAME", "mesa")
        .env("DRI_PRIME", device)
        .env("VK_DRIVER_FILES", manifest)
        .env("VK_ICD_FILENAMES", manifest);
}

fn read_lock_pid(path: &Path) -> Option<u32> {
    let bytes = fs::read(path).ok()?;
    let digits = bytes
        .into_iter()
        .take_while(|byte| byte.is_ascii_digit())
        .collect::<Vec<_>>();
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

fn emulator_process_is_live(proc_root: &Path, pid: u32, profile_name: &str) -> bool {
    let process = proc_root.join(pid.to_string());
    let stat = match fs::read_to_string(process.join("stat")) {
        Ok(stat) => stat,
        Err(_) => return false,
    };
    let state = stat
        .rsplit_once(") ")
        .and_then(|(_, tail)| tail.chars().next());
    if state == Some('Z') {
        return false;
    }
    fs::read(process.join("cmdline"))
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).replace('\0', " "))
        .is_some_and(|command| {
            command.contains(profile_name)
                && (command.contains("qemu-system") || command.contains("/emulator"))
        })
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
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

fn data_path(relative: &str) -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home_path(".local/share"))
        .map(|base| base.join(relative))
}

fn runtime_system_image(profile_name: &str) -> Option<PathBuf> {
    let config = home_path(&format!(".android/avd/{profile_name}.avd/config.ini"))?;
    let config = fs::read_to_string(config).ok()?;
    let image_sysdir = config.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "image.sysdir.1").then(|| value.trim())
    })?;
    let runtime_key = runtime_key_from_image_sysdir(image_sysdir)?;
    let image = data_path(&format!("emumi/runtime/{runtime_key}/system.img"))?;
    image.is_file().then_some(image)
}

fn emulator_launch_args(
    profile_name: &str,
    args: &[String],
    runtime_image: Option<PathBuf>,
) -> Vec<OsString> {
    let mut emulator_args = vec![OsString::from("-avd"), OsString::from(profile_name)];
    emulator_args.extend(args.iter().map(OsString::from));
    if let Some(image) = runtime_image {
        // Without -writable-system the emulator ignores -system and opens the
        // SDK's stock system.img read-only, so the compatibility runtime (and
        // its patched native bridge) never reaches the guest. With it, the
        // emulator boots the profile's small system.img.qcow2 overlay backed
        // by the private runtime.
        emulator_args.push(OsString::from("-system"));
        emulator_args.push(image.into_os_string());
        emulator_args.push(OsString::from("-writable-system"));
    }
    emulator_args
}

fn runtime_key_from_image_sysdir(image_sysdir: &str) -> Option<String> {
    let mut components = image_sysdir
        .trim_matches('/')
        .strip_prefix("system-images/")?
        .split('/');
    let api = components.next()?;
    let flavor = components.next()?;
    let architecture = components.next()?;
    if components.next().is_some()
        || [api, flavor, architecture]
            .iter()
            .any(|component| !safe_runtime_component(component))
    {
        return None;
    }
    Some(format!("{api}-{flavor}-{architecture}"))
}

fn safe_runtime_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn prepare_profile_runtime_overlay(profile_name: &str, runtime_image: &Path) -> Result<(), String> {
    validate_profile_name(profile_name)?;
    let avd_dir =
        home_path(&format!(".android/avd/{profile_name}.avd")).ok_or("HOME is not set")?;
    let metadata = fs::metadata(runtime_image)
        .map_err(|error| format!("Could not inspect EmuMi compatibility runtime: {error}"))?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let marker = format!(
        "{}\n{}\n{}\n",
        runtime_image.display(),
        metadata.len(),
        modified
    );
    let marker_path = avd_dir.join("emumi-system-runtime");
    if fs::read_to_string(&marker_path).ok().as_deref() == Some(marker.as_str()) {
        return Ok(());
    }

    let overlay = avd_dir.join("system.img.qcow2");
    if overlay.is_file() {
        let backup = unique_runtime_overlay_backup(&avd_dir);
        fs::rename(&overlay, &backup).map_err(|error| {
            format!(
                "Could not preserve the previous Android system overlay at {}: {error}",
                backup.display()
            )
        })?;
    }
    fs::write(&marker_path, marker)
        .map_err(|error| format!("Could not activate the EmuMi compatibility runtime: {error}"))
}

fn unique_runtime_overlay_backup(avd_dir: &Path) -> PathBuf {
    let base = avd_dir.join("system.img.qcow2.pre-emumi-runtime");
    if !base.exists() {
        return base;
    }
    for suffix in 2..=9999 {
        let candidate = avd_dir.join(format!("system.img.qcow2.pre-emumi-runtime-{suffix}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    avd_dir.join(format!(
        "system.img.qcow2.pre-emumi-runtime-{}",
        now_millis()
    ))
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
    fn intel_renderer_selects_both_graphics_apis_without_nvidia_offload() {
        let mut command = Command::new("emulator");
        command.env("__NV_PRIME_RENDER_OFFLOAD", "1");
        command.env("__VK_LAYER_NV_optimus", "NVIDIA_only");
        configure_intel_renderer(
            &mut command,
            "pci-0000_00_02_0",
            Path::new("/test/intel.json"),
        );
        let env: HashMap<_, _> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert_eq!(env["__GLX_VENDOR_LIBRARY_NAME"].as_deref(), Some("mesa"));
        assert_eq!(env["DRI_PRIME"].as_deref(), Some("pci-0000_00_02_0"));
        assert_eq!(env["VK_DRIVER_FILES"].as_deref(), Some("/test/intel.json"));
        assert_eq!(env["VK_ICD_FILENAMES"].as_deref(), Some("/test/intel.json"));
        assert_eq!(env["__NV_PRIME_RENDER_OFFLOAD"], None);
        assert_eq!(env["__VK_LAYER_NV_optimus"], None);
    }

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
    fn finds_only_enabled_display_shape_overlays() {
        let overlays = "[x] com.android.internal.emulation.pixel_7\n[ ] com.android.internal.emulation.pixel_8\n[x] com.android.systemui.emulation.pixel_7\n[x] com.android.systemui:accent\n[x] com.android.internal.display.cutout.emulation.hole\n";
        assert_eq!(
            enabled_display_shape_overlays(overlays),
            vec![
                "com.android.internal.emulation.pixel_7",
                "com.android.systemui.emulation.pixel_7",
                "com.android.internal.display.cutout.emulation.hole"
            ]
        );
    }

    #[test]
    fn lists_all_overlay_targets_when_normalizing_display_shape() {
        assert_eq!(
            overlay_list_args("emulator-5560"),
            [
                "-s",
                "emulator-5560",
                "shell",
                "cmd",
                "overlay",
                "list",
                "--user",
                "0"
            ]
        );
    }

    #[test]
    fn validates_safe_profile_names() {
        assert!(validate_profile_name("Pixel_7_work").is_ok());
        assert!(validate_profile_name("bad profile").is_err());
        assert!(validate_profile_name("../bad").is_err());
    }

    #[test]
    fn maps_sdk_images_to_private_runtime_keys() {
        assert_eq!(
            runtime_key_from_image_sysdir("system-images/android-36/google_apis_playstore/x86_64/")
                .as_deref(),
            Some("android-36-google_apis_playstore-x86_64")
        );
        assert!(runtime_key_from_image_sysdir("../outside").is_none());
        assert!(
            runtime_key_from_image_sysdir(
                "system-images/android-36/google_apis_playstore/x86_64/extra"
            )
            .is_none()
        );
    }

    #[test]
    fn launches_with_compatibility_runtime_boot_that_runtime() {
        let image = PathBuf::from("/data/emumi/runtime/android-36/system.img");
        let emulator_args = emulator_launch_args(
            "Device_3",
            &["-port".into(), "5558".into()],
            Some(image.clone()),
        );
        let direct = launch_spec(
            Path::new("/sdk/emulator"),
            &emulator_args,
            "Device_3",
            &ProfileOptions {
                host_memory_policy: false,
                ..ProfileOptions::default()
            },
        )
        .unwrap();
        let scoped = systemd_scope_launch_spec(
            PathBuf::from("/usr/bin/systemd-run"),
            Path::new("/sdk/emulator"),
            &emulator_args,
            "Device_3",
            12345,
            &ProfileOptions::default(),
        );

        for spec in [direct, scoped] {
            let system = spec
                .args
                .iter()
                .position(|arg| arg == "-system")
                .expect("compatibility runtime launch must pass -system");
            assert_eq!(
                spec.args.get(system + 1),
                Some(&image.clone().into_os_string())
            );
            assert!(spec.args.iter().any(|arg| arg == "-writable-system"));
        }
    }

    #[test]
    fn launches_without_compatibility_runtime_use_the_sdk_image() {
        let emulator_args = emulator_launch_args("Device_3", &[], None);
        assert!(
            !emulator_args
                .iter()
                .any(|arg| arg == "-system" || arg == "-writable-system")
        );
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

    #[test]
    fn wraps_only_one_emulator_in_its_own_memory_scope() {
        let options = ProfileOptions {
            host_memory_policy: true,
            ..ProfileOptions::default()
        };
        let spec = systemd_scope_launch_spec(
            PathBuf::from("/usr/bin/systemd-run"),
            Path::new("/sdk/emulator"),
            &[OsString::from("-avd"), OsString::from("Device_2")],
            "Device_2",
            12345,
            &options,
        );
        let args = spec
            .args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();

        assert_eq!(spec.program, PathBuf::from("/usr/bin/systemd-run"));
        assert!(args.contains(&std::borrow::Cow::Borrowed("--unit=emumi-Device_2-12345")));
        assert!(args.contains(&std::borrow::Cow::Borrowed("--property=MemoryHigh=5632M")));
        assert!(args.contains(&std::borrow::Cow::Borrowed("--property=MemoryMax=6656M")));
        assert!(args.contains(&std::borrow::Cow::Borrowed(
            "--property=MemorySwapMax=1024M"
        )));
        assert!(args.contains(&std::borrow::Cow::Borrowed("--property=CPUQuota=80%")));
        assert!(args.contains(&std::borrow::Cow::Borrowed("--property=CPUWeight=25")));
        assert!(args.contains(&std::borrow::Cow::Borrowed("--property=IOWeight=25")));
        assert!(args.contains(&std::borrow::Cow::Borrowed("/sdk/emulator")));
    }

    #[test]
    fn clones_avd_data_and_rewrites_registration() {
        let root = std::env::temp_dir().join(format!("emumi-clone-test-{}", now_millis()));
        let source = root.join("Source.avd");
        fs::create_dir_all(source.join("snapshots/default_boot")).unwrap();
        fs::create_dir_all(source.join("tmpAdbCmds")).unwrap();
        fs::write(
            source.join("config.ini"),
            "avd.id=Source\navd.name=Source\navd.ini.displayname=Source\n",
        )
        .unwrap();
        fs::write(source.join("userdata-qemu.img.qcow2"), b"android data").unwrap();
        fs::write(source.join("multiinstance.lock"), b"").unwrap();
        fs::write(
            root.join("Source.ini"),
            format!(
                "avd.ini.encoding=UTF-8\npath={}\npath.rel=avd/Source.avd\ntarget=android-35\n",
                source.display()
            ),
        )
        .unwrap();

        AndroidTools::default()
            .clone_profile_in_root("Source", "Clone", &root)
            .unwrap();

        let config = fs::read_to_string(root.join("Clone.avd/config.ini")).unwrap();
        let registration = fs::read_to_string(root.join("Clone.ini")).unwrap();
        assert!(config.contains("avd.id=Clone"));
        assert!(config.contains("avd.ini.displayname=Clone"));
        assert!(registration.contains(&format!("path={}", root.join("Clone.avd").display())));
        assert!(registration.contains("path.rel=avd/Clone.avd"));
        assert_eq!(
            fs::read(root.join("Clone.avd/userdata-qemu.img.qcow2")).unwrap(),
            b"android data"
        );
        assert!(!root.join("Clone.avd/multiinstance.lock").exists());
        assert!(!root.join("Clone.avd/snapshots").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removes_runtime_locks_left_by_a_dead_emulator() {
        let root = std::env::temp_dir().join(format!("emumi-stale-lock-test-{}", now_millis()));
        let avd_root = root.join("avd");
        let profile = avd_root.join("Device_4.avd");
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("hardware-qemu.ini.lock"), b"999999\0").unwrap();
        fs::write(profile.join("multiinstance.lock"), b"").unwrap();
        fs::write(profile.join("snapshot.lock.lock"), b"999999\0").unwrap();

        clear_stale_runtime_locks_in(&avd_root, &root.join("proc"), "Device_4").unwrap();

        assert!(!profile.join("hardware-qemu.ini.lock").exists());
        assert!(!profile.join("multiinstance.lock").exists());
        assert!(!profile.join("snapshot.lock.lock").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preserves_locks_owned_by_a_live_matching_emulator() {
        let root = std::env::temp_dir().join(format!("emumi-live-lock-test-{}", now_millis()));
        let avd_root = root.join("avd");
        let proc_root = root.join("proc");
        let profile = avd_root.join("Device_4.avd");
        let process = proc_root.join("1234");
        fs::create_dir_all(&profile).unwrap();
        fs::create_dir_all(&process).unwrap();
        fs::write(profile.join("hardware-qemu.ini.lock"), b"1234\0").unwrap();
        fs::write(process.join("stat"), b"1234 (qemu-system-x86) S 1 1 1").unwrap();
        fs::write(
            process.join("cmdline"),
            b"/sdk/qemu-system-x86_64\0-avd\0Device_4\0",
        )
        .unwrap();

        let error = clear_stale_runtime_locks_in(&avd_root, &proc_root, "Device_4").unwrap_err();

        assert!(error.contains("live emulator process"));
        assert!(profile.join("hardware-qemu.ini.lock").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
