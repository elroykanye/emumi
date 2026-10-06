use crate::{
    android::AndroidTools,
    config::AppConfig,
    model::{HostStats, LaunchAdmission, PicturePreset, ProfileOptions, SpeedPreset},
    monitor::{HostMonitor, sample_devices},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const MAX_RUNNING_PROFILES: usize = 4;
const MIN_AVAILABLE_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_LAUNCH_TEMPERATURE_C: f32 = 90.0;
const STARTUP_SETTLE_SECONDS: u64 = 20;
const POST_READY_CPU_BOOST_SECONDS: u64 = 120;
const SETTLED_CPU_QUOTA_PERCENT: u16 = 120;

type Shared = Arc<Mutex<Runtime>>;
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiMessage>)>;

pub struct EmuMiApp;

struct Runtime {
    config: AppConfig,
    logs: Vec<ApiLog>,
    monitor: HostMonitor,
    rectangular_watchers: BTreeSet<String>,
    starting_profiles: BTreeSet<String>,
    startup_generations: BTreeMap<String, u64>,
    next_startup_generation: u64,
    queued_profiles: VecDeque<String>,
    memory_warnings: BTreeMap<String, String>,
}

#[derive(Clone, Serialize)]
struct ApiLog {
    at: u64,
    level: &'static str,
    message: String,
}

#[derive(Serialize)]
struct AppState {
    profiles: Vec<crate::model::AndroidProfile>,
    profile_options: BTreeMap<String, ProfileOptions>,
    starting_profiles: BTreeSet<String>,
    queued_profiles: Vec<String>,
    system_images: Vec<ImageDto>,
    config: SettingsDto,
    health: HealthDto,
    stats: crate::model::HostStats,
    device_stats: Vec<crate::model::DeviceHostStats>,
    logs: Vec<ApiLog>,
}

#[derive(Serialize)]
struct ImageDto {
    package_id: String,
    label: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct SettingsDto {
    android_sdk_path: String,
    jdk_path: String,
}

#[derive(Serialize)]
struct HealthDto {
    emulator: bool,
    adb: bool,
    avd_manager: bool,
    kvm: bool,
    java: bool,
    systemd_run: bool,
}

#[derive(Serialize)]
struct ApiMessage {
    message: String,
}

#[derive(Serialize)]
struct PortReadiness {
    ready: bool,
    phase: &'static str,
    message: String,
}

#[derive(Deserialize)]
struct CreateRequest {
    name: String,
    device_id: String,
    package_id: String,
}

#[derive(Deserialize)]
struct CloneRequest {
    name: String,
}

impl EmuMiApp {
    pub async fn start_server() -> Result<String, Box<dyn std::error::Error>> {
        let config = AppConfig::load();
        let runtime = Arc::new(Mutex::new(Runtime {
            config,
            logs: vec![ApiLog::new("success", "EmuMi is ready")],
            monitor: HostMonitor::default(),
            rectangular_watchers: BTreeSet::new(),
            starting_profiles: BTreeSet::new(),
            startup_generations: BTreeMap::new(),
            next_startup_generation: 1,
            queued_profiles: VecDeque::new(),
            memory_warnings: BTreeMap::new(),
        }));

        let app = Router::new()
            .route("/", get(index))
            .route("/app.js", get(script))
            .route("/api/state", get(read_state))
            .route("/api/profiles", post(create_profile))
            .route("/api/profiles/{name}", delete(delete_profile))
            .route("/api/profiles/{name}/start", post(start_profile))
            .route("/api/profiles/{name}/stop", post(stop_profile))
            .route("/api/profiles/{name}/clone", post(clone_profile))
            .route("/api/profiles/{name}/settings", post(save_profile_settings))
            .route("/api/ports/{port}/start", post(start_port))
            .route("/api/ports/{port}/stop", post(stop_port))
            .route("/api/ports/{port}/readiness", get(port_readiness))
            .route("/api/settings", post(save_settings))
            .with_state(runtime);

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let url = format!("http://{address}");
        write_api_endpoint(&url)?;
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                eprintln!("EmuMi server stopped: {error}");
            }
        });
        Ok(url)
    }
}

impl ApiLog {
    fn new(level: &'static str, message: impl Into<String>) -> Self {
        Self {
            at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            level,
            message: message.into(),
        }
    }
}

impl Runtime {
    fn tools(&self) -> AndroidTools {
        AndroidTools::detect(&self.config.android_sdk_path, &self.config.jdk_path)
    }

    fn log(&mut self, level: &'static str, message: impl Into<String>) {
        self.logs.push(ApiLog::new(level, message));
        if self.logs.len() > 200 {
            self.logs.drain(..self.logs.len() - 200);
        }
    }

    fn ensure_profile_ports(&mut self, profile_names: &[String]) {
        let mut used = BTreeSet::new();
        for name in profile_names {
            let options = self.config.profile_options.entry(name.clone()).or_default();
            if let Some(port) = options.adb_port
                && (!(5554..=5682).contains(&port) || port % 2 != 0 || !used.insert(port))
            {
                options.adb_port = None;
            }
        }
        let mut changed = false;
        for name in profile_names {
            let options = self.config.profile_options.entry(name.clone()).or_default();
            if options.adb_port.is_none()
                && let Some(port) = (5554..=5682).step_by(2).find(|port| !used.contains(port))
            {
                options.adb_port = Some(port);
                used.insert(port);
                changed = true;
            }
        }
        if changed {
            let _ = self.config.save();
        }
    }

    fn next_profile_port(&self) -> Option<u16> {
        let active_profiles = self
            .tools()
            .discover_profiles()
            .into_iter()
            .map(|profile| profile.name)
            .collect::<BTreeSet<_>>();
        let used = self
            .config
            .profile_options
            .iter()
            .filter(|(name, _)| active_profiles.contains(*name))
            .filter_map(|(_, options)| options.adb_port)
            .collect::<BTreeSet<_>>();
        (5554..=5682).step_by(2).find(|port| !used.contains(port))
    }
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
}

async fn read_state(State(shared): State<Shared>) -> ApiResult<AppState> {
    let mut runtime = lock(&shared)?;
    let tools = runtime.tools();
    let profiles = tools.discover_profiles();
    runtime.ensure_profile_ports(
        &profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>(),
    );
    let running_names = profiles
        .iter()
        .filter(|profile| profile.is_running())
        .map(|profile| profile.name.clone())
        .collect::<BTreeSet<_>>();
    runtime
        .rectangular_watchers
        .retain(|name| running_names.contains(name));
    for profile in &profiles {
        let rectangular = runtime
            .config
            .profile_options
            .get(&profile.name)
            .cloned()
            .unwrap_or_default()
            .rectangular_display;
        if profile.is_running()
            && rectangular
            && runtime.rectangular_watchers.insert(profile.name.clone())
        {
            tools.watch_rectangular_display_after_start(&profile.name);
        }
    }
    let system_images = tools
        .installed_system_images()
        .into_iter()
        .map(|image| ImageDto {
            label: image.label(),
            package_id: image.package_id,
        })
        .collect();
    let device_stats = sample_devices(&profiles);
    let running_profile_names = device_stats
        .iter()
        .map(|sample| sample.profile_name.clone())
        .collect::<BTreeSet<_>>();
    runtime
        .memory_warnings
        .retain(|name, _| running_profile_names.contains(name));
    for sample in &device_stats {
        match &sample.warning {
            Some(warning) if runtime.memory_warnings.get(&sample.profile_name) != Some(warning) => {
                runtime
                    .memory_warnings
                    .insert(sample.profile_name.clone(), warning.clone());
                runtime.log("warning", format!("{}: {warning}", sample.profile_name));
            }
            None => {
                runtime.memory_warnings.remove(&sample.profile_name);
            }
            _ => {}
        }
    }
    let state = AppState {
        profiles,
        profile_options: runtime.config.profile_options.clone(),
        starting_profiles: runtime.starting_profiles.clone(),
        queued_profiles: runtime.queued_profiles.iter().cloned().collect(),
        system_images,
        config: SettingsDto {
            android_sdk_path: runtime.config.android_sdk_path.clone(),
            jdk_path: runtime.config.jdk_path.clone(),
        },
        health: HealthDto {
            emulator: tools.emulator.is_some(),
            adb: tools.adb.is_some(),
            avd_manager: tools.avd_manager.is_some(),
            kvm: tools.kvm_available,
            java: tools.java_home.is_some(),
            systemd_run: tools.systemd_run_available,
        },
        stats: runtime.monitor.sample(),
        device_stats,
        logs: runtime.logs.clone(),
    };
    Ok(Json(state))
}

async fn create_profile(
    State(shared): State<Shared>,
    Json(request): Json<CreateRequest>,
) -> ApiResult<ApiMessage> {
    let mut runtime = lock(&shared)?;
    let tools = runtime.tools();
    if tools
        .discover_profiles()
        .iter()
        .any(|profile| profile.name == request.name)
    {
        return api_error(
            StatusCode::CONFLICT,
            "A profile with that name already exists",
        );
    }
    let image = tools
        .installed_system_images()
        .into_iter()
        .find(|image| image.package_id == request.package_id)
        .ok_or_else(|| {
            error_tuple(
                StatusCode::BAD_REQUEST,
                "That system image is not installed",
            )
        })?;
    tools
        .create_profile(request.name.trim(), &request.device_id, &image)
        .map_err(|message| error_tuple(StatusCode::BAD_REQUEST, message))?;
    let options = ProfileOptions {
        adb_port: runtime.next_profile_port(),
        ..ProfileOptions::default()
    };
    runtime
        .config
        .profile_options
        .insert(request.name.trim().to_owned(), options);
    let _ = runtime.config.save();
    runtime.log("success", format!("Created {}", request.name.trim()));
    Ok(message("Android created"))
}

async fn start_profile(
    State(shared): State<Shared>,
    Path(name): Path<String>,
) -> ApiResult<ApiMessage> {
    start_named_profile(&shared, &name)
}

fn start_named_profile(shared: &Shared, name: &str) -> ApiResult<ApiMessage> {
    let mut runtime = lock(shared)?;
    let tools = runtime.tools();
    let profile = tools
        .discover_profiles()
        .into_iter()
        .find(|profile| profile.name == name)
        .ok_or_else(|| error_tuple(StatusCode::NOT_FOUND, "Profile not found"))?;
    if runtime.starting_profiles.contains(name) {
        return Ok(message("Android is already starting"));
    }
    if runtime.queued_profiles.iter().any(|queued| queued == name) {
        return Ok(message("Android is already queued"));
    }
    if profile.is_running() {
        return Ok(message("Android is already running"));
    }
    let running = tools
        .discover_profiles()
        .into_iter()
        .filter(|candidate| candidate.is_running())
        .count();
    if running + runtime.starting_profiles.len() + runtime.queued_profiles.len()
        >= MAX_RUNNING_PROFILES
    {
        return api_error(
            StatusCode::CONFLICT,
            "This computer is limited to four running Androids. Stop an idle one first.",
        );
    }
    if !runtime.starting_profiles.is_empty() {
        runtime.queued_profiles.push_back(name.to_owned());
        runtime.log(
            "info",
            format!("Queued {name} until the current Android settles"),
        );
        return Ok(message("Android is queued for an adaptive start"));
    }
    let launch_admission = enforce_launch_admission(&mut runtime.monitor)
        .map_err(|reason| error_tuple(StatusCode::CONFLICT, reason))?;
    let options = runtime
        .config
        .profile_options
        .entry(name.to_owned())
        .or_default()
        .clone();
    runtime.starting_profiles.insert(name.to_owned());
    let startup_generation = runtime.next_startup_generation;
    runtime.next_startup_generation = runtime.next_startup_generation.wrapping_add(1).max(1);
    runtime
        .startup_generations
        .insert(name.to_owned(), startup_generation);
    let (display_width, display_height) = display_dimensions(options.picture);
    let start_result = tools
        .configure_input_and_window(
            name,
            options.host_keyboard,
            options.device_frame,
            display_width,
            display_height,
            options.dpi,
            options.speed == SpeedPreset::LeanGaming,
        )
        .and_then(|_| {
            if let Some(scale) = options.window_scale {
                tools.configure_window_scale(name, scale)
            } else {
                Ok(())
            }
        })
        .and_then(|_| tools.start(&profile, &launch_args(&options), &options, launch_admission));
    let scope_unit = match start_result {
        Ok(scope_unit) => scope_unit,
        Err(message) => {
            runtime.starting_profiles.remove(name);
            runtime.startup_generations.remove(name);
            return Err(error_tuple(StatusCode::BAD_REQUEST, message));
        }
    };
    if options.rectangular_display {
        runtime.rectangular_watchers.insert(name.to_owned());
        tools.watch_rectangular_display_after_start(name);
    }
    if options.speed == SpeedPreset::LeanGaming {
        tools.watch_lean_profile_after_start(name, options.suspend_store_during_automation);
    }
    if options.cold_boot
        && let Some(saved) = runtime.config.profile_options.get_mut(name)
    {
        saved.cold_boot = false;
        let _ = runtime.config.save();
    }
    if options.speed == SpeedPreset::LeanGaming {
        runtime.log(
            "success",
            format!("Starting {name} with a clean Lean Gaming boot"),
        );
    } else {
        runtime.log("success", format!("Starting {name}"));
    }
    watch_profile_start(
        Arc::clone(shared),
        name.to_owned(),
        startup_generation,
        scope_unit,
    );
    Ok(message("Android is starting"))
}

fn watch_profile_start(
    shared: Shared,
    name: String,
    startup_generation: u64,
    scope_unit: Option<String>,
) {
    tokio::spawn(async move {
        let mut booted = false;
        for _ in 0..240 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let Ok(runtime) = shared.lock() else {
                return;
            };
            if runtime.tools().profile_boot_completed(&name) {
                booted = true;
                break;
            }
        }

        let completion = if booted {
            // Give login, Play services and the automation target time to warm up.
            // Then require three calm host samples before admitting the next AVD.
            let settled = settle_after_boot().await;
            let mut monitor = HostMonitor::default();
            let _ = monitor.sample();
            let mut calm_samples = 0;
            for _ in 0..45 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if monitor.sample().cpu_percent < 85.0 {
                    calm_samples += 1;
                    if calm_samples >= 3 {
                        break;
                    }
                } else {
                    calm_samples = 0;
                }
            }
            StartupCompletion::Settled(settled)
        } else {
            StartupCompletion::TimedOut
        };

        if let Ok(mut runtime) = shared.lock() {
            if runtime.startup_generations.get(&name) != Some(&startup_generation) {
                return;
            }
            runtime.starting_profiles.remove(&name);
            runtime.startup_generations.remove(&name);
            if booted {
                runtime.log("success", format!("{name} is ready"));
            } else {
                runtime.log("warning", format!("{name} did not finish starting"));
            }
        }
        start_next_queued_profile(&shared, completion);
        schedule_settled_cpu_quota(shared, name, scope_unit);
    });
}

#[derive(Debug)]
struct StartupSettled;

#[derive(Debug)]
enum StartupCompletion {
    Settled(StartupSettled),
    TimedOut,
}

async fn settle_after_boot() -> StartupSettled {
    settle_after_boot_with(tokio::time::sleep).await
}

async fn settle_after_boot_with<F, Fut>(sleep: F) -> StartupSettled
where
    F: FnOnce(std::time::Duration) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    sleep(std::time::Duration::from_secs(STARTUP_SETTLE_SECONDS)).await;
    StartupSettled
}

fn schedule_settled_cpu_quota(shared: Shared, name: String, scope_unit: Option<String>) {
    let Some(scope_unit) = scope_unit else {
        return;
    };
    tokio::spawn(async move {
        let result = settle_cpu_quota_with(tokio::time::sleep, || {
            let tools = shared
                .lock()
                .map_err(|_| "EmuMi state is unavailable".to_owned())?
                .tools();
            tools.set_scope_cpu_quota(&scope_unit, SETTLED_CPU_QUOTA_PERCENT)
        })
        .await;
        if let Ok(mut runtime) = shared.lock() {
            match result {
                Ok(()) => runtime.log(
                    "success",
                    format!(
                        "Reduced {name} CPU quota to {SETTLED_CPU_QUOTA_PERCENT}% after startup"
                    ),
                ),
                Err(error) => runtime.log(
                    "warning",
                    format!("Could not reduce {name} CPU quota after startup: {error}"),
                ),
            }
        }
    });
}

async fn settle_cpu_quota_with<F, Fut, A>(sleep: F, apply: A) -> Result<(), String>
where
    F: FnOnce(std::time::Duration) -> Fut,
    Fut: std::future::Future<Output = ()>,
    A: FnOnce() -> Result<(), String>,
{
    sleep(std::time::Duration::from_secs(POST_READY_CPU_BOOST_SECONDS)).await;
    apply()
}

fn start_next_queued_profile(shared: &Shared, completion: StartupCompletion) {
    match completion {
        StartupCompletion::Settled(_proof) => {}
        StartupCompletion::TimedOut => {}
    }
    loop {
        let next = match shared.lock() {
            Ok(mut runtime) => runtime.queued_profiles.pop_front(),
            Err(_) => return,
        };
        let Some(next) = next else {
            return;
        };
        match start_named_profile(shared, &next) {
            Ok(_) => {
                let is_now_starting = shared
                    .lock()
                    .map(|runtime| runtime.starting_profiles.contains(&next))
                    .unwrap_or(false);
                if is_now_starting {
                    return;
                }
                // The queued profile may have been started outside EmuMi while it waited.
                // In that case no watcher will drain the rest of this queue, so keep going.
            }
            Err((_, Json(error))) => {
                if let Ok(mut runtime) = shared.lock() {
                    runtime.log(
                        "error",
                        format!("Could not start queued {next}: {}", error.message),
                    );
                    runtime.starting_profiles.remove(&next);
                    runtime.startup_generations.remove(&next);
                }
            }
        }
    }
}

async fn stop_profile(
    State(shared): State<Shared>,
    Path(name): Path<String>,
) -> ApiResult<ApiMessage> {
    stop_named_profile(&shared, &name)
}

fn stop_named_profile(shared: &Shared, name: &str) -> ApiResult<ApiMessage> {
    let mut runtime = lock(shared)?;
    runtime.starting_profiles.remove(name);
    runtime.startup_generations.remove(name);
    runtime.queued_profiles.retain(|queued| queued != name);
    let tools = runtime.tools();
    let profile = tools
        .discover_profiles()
        .into_iter()
        .find(|profile| profile.name == name)
        .ok_or_else(|| error_tuple(StatusCode::NOT_FOUND, "Profile not found"))?;
    let Some(serial) = profile.running_serial else {
        return Ok(message("Android is already stopped"));
    };
    tools
        .stop(&serial)
        .map_err(|message| error_tuple(StatusCode::BAD_REQUEST, message))?;
    runtime.log("info", format!("Stopped {name}"));
    Ok(message("Android stopped"))
}

async fn start_port(State(shared): State<Shared>, Path(port): Path<u16>) -> ApiResult<ApiMessage> {
    let name = profile_name_for_port(&shared, port)?;
    start_named_profile(&shared, &name)
}

async fn stop_port(State(shared): State<Shared>, Path(port): Path<u16>) -> ApiResult<ApiMessage> {
    let name = profile_name_for_port(&shared, port)?;
    stop_named_profile(&shared, &name)
}

async fn port_readiness(
    State(shared): State<Shared>,
    Path(port): Path<u16>,
) -> ApiResult<PortReadiness> {
    let runtime = lock(&shared)?;
    let name = runtime
        .config
        .profile_options
        .iter()
        .find_map(|(name, options)| (options.adb_port == Some(port)).then(|| name.clone()))
        .ok_or_else(|| error_tuple(StatusCode::NOT_FOUND, "No profile uses that ADB port"))?;

    let readiness = if runtime.queued_profiles.iter().any(|queued| queued == &name) {
        PortReadiness {
            ready: false,
            phase: "queued",
            message: format!("{name} is queued for startup"),
        }
    } else if runtime.starting_profiles.contains(&name) {
        PortReadiness {
            ready: false,
            phase: "starting",
            message: format!("{name} is still completing its clean boot"),
        }
    } else {
        let tools = runtime.tools();
        let profile = tools
            .discover_profiles()
            .into_iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| error_tuple(StatusCode::NOT_FOUND, "Profile not found"))?;
        if !profile.is_running() {
            PortReadiness {
                ready: false,
                phase: "stopped",
                message: format!("{name} is stopped"),
            }
        } else if !tools.profile_boot_completed(&name) {
            PortReadiness {
                ready: false,
                phase: "booting",
                message: format!("{name} is visible to ADB but Android has not completed boot"),
            }
        } else {
            PortReadiness {
                ready: true,
                phase: "ready",
                message: format!("{name} is ready for app launch"),
            }
        }
    };

    Ok(Json(readiness))
}

fn profile_name_for_port(
    shared: &Shared,
    port: u16,
) -> Result<String, (StatusCode, Json<ApiMessage>)> {
    lock(shared)?
        .config
        .profile_options
        .iter()
        .find_map(|(name, options)| (options.adb_port == Some(port)).then(|| name.clone()))
        .ok_or_else(|| {
            error_tuple(
                StatusCode::NOT_FOUND,
                format!("No Android uses ADB port {port}"),
            )
        })
}

async fn clone_profile(
    State(shared): State<Shared>,
    Path(source_name): Path<String>,
    Json(request): Json<CloneRequest>,
) -> ApiResult<ApiMessage> {
    let clone_name = request.name.trim().to_owned();
    let (tools, mut options) = {
        let runtime = lock(&shared)?;
        let profiles = runtime.tools().discover_profiles();
        let source = profiles
            .iter()
            .find(|profile| profile.name == source_name)
            .ok_or_else(|| error_tuple(StatusCode::NOT_FOUND, "Source Android not found"))?;
        if source.is_running() {
            return api_error(
                StatusCode::CONFLICT,
                "Stop the source Android before cloning it",
            );
        }
        if profiles.iter().any(|profile| profile.name == clone_name) {
            return api_error(StatusCode::CONFLICT, "That clone name is already in use");
        }
        let options = runtime
            .config
            .profile_options
            .get(&source_name)
            .cloned()
            .unwrap_or_default();
        let mut options = options;
        if options.window_scale.is_none() {
            options.window_scale = ProfileOptions::default().window_scale;
        }
        (runtime.tools(), options)
    };

    let source_for_copy = source_name.clone();
    let clone_for_copy = clone_name.clone();
    tokio::task::spawn_blocking(move || tools.clone_profile(&source_for_copy, &clone_for_copy))
        .await
        .map_err(|error| {
            error_tuple(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Clone worker failed: {error}"),
            )
        })?
        .map_err(|message| error_tuple(StatusCode::BAD_REQUEST, message))?;

    options.adb_port = runtime_port(&shared)?;
    options.cold_boot = true;
    let mut runtime = lock(&shared)?;
    runtime
        .config
        .profile_options
        .insert(clone_name.clone(), options);
    runtime
        .config
        .save()
        .map_err(|error| error_tuple(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    runtime.log("success", format!("Cloned {source_name} as {clone_name}"));
    Ok(message(format!("{clone_name} cloned successfully")))
}

fn runtime_port(shared: &Shared) -> Result<Option<u16>, (StatusCode, Json<ApiMessage>)> {
    Ok(lock(shared)?.next_profile_port())
}

async fn delete_profile(
    State(shared): State<Shared>,
    Path(name): Path<String>,
) -> ApiResult<ApiMessage> {
    let mut runtime = lock(&shared)?;
    let tools = runtime.tools();
    if tools
        .discover_profiles()
        .iter()
        .any(|profile| profile.name == name && profile.is_running())
    {
        return api_error(StatusCode::CONFLICT, "Stop this Android before deleting it");
    }
    tools
        .delete_profile(&name)
        .map_err(|message| error_tuple(StatusCode::BAD_REQUEST, message))?;
    runtime.config.profile_options.remove(&name);
    let _ = runtime.config.save();
    runtime.log("info", format!("Deleted {name}"));
    Ok(message("Android deleted"))
}

async fn save_profile_settings(
    State(shared): State<Shared>,
    Path(name): Path<String>,
    Json(options): Json<ProfileOptions>,
) -> ApiResult<ApiMessage> {
    let mut runtime = lock(&shared)?;
    validate_profile_options(&options)
        .map_err(|message| error_tuple(StatusCode::BAD_REQUEST, message))?;
    if options.window_scale.is_none() {
        runtime
            .tools()
            .forget_window_scale(&name)
            .map_err(|message| error_tuple(StatusCode::BAD_REQUEST, message))?;
    }
    runtime.config.profile_options.insert(name.clone(), options);
    runtime
        .config
        .save()
        .map_err(|error| error_tuple(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    runtime.log("success", format!("Saved settings for {name}"));
    Ok(message("Profile settings saved"))
}

async fn save_settings(
    State(shared): State<Shared>,
    Json(settings): Json<SettingsDto>,
) -> ApiResult<ApiMessage> {
    let mut runtime = lock(&shared)?;
    runtime.config.android_sdk_path = settings.android_sdk_path.trim().to_owned();
    runtime.config.jdk_path = settings.jdk_path.trim().to_owned();
    runtime
        .config
        .save()
        .map_err(|error| error_tuple(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    runtime.log("success", "Tool paths saved");
    Ok(message("Settings saved"))
}

fn launch_args(options: &ProfileOptions) -> Vec<String> {
    let (display_width, display_height) = display_dimensions(options.picture);
    let mut args = vec![
        "-cores".into(),
        options.cores.to_string(),
        "-memory".into(),
        options.memory_mb.to_string(),
        "-gpu".into(),
        if options.gpu_mode == "host-intel" {
            "host".into()
        } else {
            options.gpu_mode.clone()
        },
        "-skin".into(),
        format!("{display_width}x{display_height}"),
        "-vsync-rate".into(),
        options.refresh_rate_hz.to_string(),
    ];
    if let Some(port) = options.adb_port {
        args.extend(["-port".into(), port.to_string()]);
    }
    // Lean Gaming profiles favor a deterministic guest boot over Quick Boot.
    // This skips saved RAM state without touching userdata, installed apps, or accounts.
    if options.cold_boot || options.speed == SpeedPreset::LeanGaming {
        args.push("-no-snapshot-load".into());
    }
    if options.mute_audio {
        args.push("-no-audio".into());
    }
    if options.headless_automation {
        args.extend([
            "-no-window".into(),
            "-camera-back".into(),
            "none".into(),
            "-camera-front".into(),
            "none".into(),
            "-no-boot-anim".into(),
        ]);
    }
    if options.disable_vulkan {
        args.extend(["-feature".into(), "-Vulkan".into()]);
    }
    args
}

fn validate_profile_options(options: &ProfileOptions) -> Result<(), String> {
    if options.cores == 0 || options.cores > 16 {
        return Err("CPU cores must be between 1 and 16".into());
    }
    if options.memory_mb < 512 || options.memory_mb > 16_384 {
        return Err("Android memory must be between 512 and 16384 MB".into());
    }
    if !(15..=120).contains(&options.refresh_rate_hz) {
        return Err("Refresh rate must be between 15 and 120 Hz".into());
    }
    if options.host_memory_policy {
        if options.memory_high_mb < 4096 {
            return Err("MemoryHigh must be at least 4096 MB for safe initial testing".into());
        }
        if options.memory_max_mb < options.memory_high_mb {
            return Err("MemoryMax must be greater than or equal to MemoryHigh".into());
        }
        if options.memory_max_mb < options.memory_mb {
            return Err("MemoryMax cannot be lower than Android guest memory".into());
        }
    }
    Ok(())
}

fn launch_block_reason(host: &HostStats) -> Option<String> {
    if host.memory_available_bytes < MIN_AVAILABLE_MEMORY_BYTES {
        return Some(
            "EmuMi needs at least 8 GB of available host memory before starting another Android"
                .into(),
        );
    }
    match host.cpu_temperature_c {
        Some(temperature) if temperature.is_finite() && temperature < MAX_LAUNCH_TEMPERATURE_C => {}
        Some(temperature) if temperature.is_finite() => {
            return Some("CPU temperature is at or above 90 C; let the computer cool before starting another Android".into());
        }
        _ => {
            return Some(
                "EmuMi could not verify CPU temperature; restore hardware monitoring before starting another Android"
                    .into(),
            );
        }
    }
    None
}

fn enforce_launch_admission(monitor: &mut HostMonitor) -> Result<LaunchAdmission, String> {
    enforce_launch_admission_with(|| monitor.sample())
}

fn enforce_launch_admission_with(
    sample: impl FnOnce() -> HostStats,
) -> Result<LaunchAdmission, String> {
    launch_block_reason(&sample()).map_or_else(|| Ok(LaunchAdmission::granted()), Err)
}

fn write_api_endpoint(url: &str) -> std::io::Result<()> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or_else(|| std::io::Error::other("HOME is not set"))?;
    let directory = base.join("emumi");
    fs::create_dir_all(&directory)?;
    fs::write(directory.join("api-url"), format!("{url}\n"))
}

fn display_dimensions(_picture: PicturePreset) -> (u16, u16) {
    // Frostguard's screenshots, OCR regions and tap coordinates use this exact canvas.
    // Keep the serialized picture choice for backwards-compatible config parsing, but
    // never let an older profile launch at a different automation resolution.
    (720, 1280)
}

fn lock(
    shared: &Shared,
) -> Result<std::sync::MutexGuard<'_, Runtime>, (StatusCode, Json<ApiMessage>)> {
    shared.lock().map_err(|_| {
        error_tuple(
            StatusCode::INTERNAL_SERVER_ERROR,
            "EmuMi state is unavailable",
        )
    })
}

fn message(text: impl Into<String>) -> Json<ApiMessage> {
    Json(ApiMessage {
        message: text.into(),
    })
}

fn api_error<T>(status: StatusCode, text: impl Into<String>) -> ApiResult<T> {
    Err(error_tuple(status, text))
}

fn error_tuple(status: StatusCode, text: impl Into<String>) -> (StatusCode, Json<ApiMessage>) {
    (status, message(text))
}

#[cfg(test)]
mod tests {
    use super::{
        POST_READY_CPU_BOOST_SECONDS, SETTLED_CPU_QUOTA_PERCENT, STARTUP_SETTLE_SECONDS,
        enforce_launch_admission, launch_args, launch_block_reason, settle_after_boot_with,
        settle_cpu_quota_with, validate_profile_options,
    };
    use crate::model::{HostStats, PicturePreset, ProfileOptions, SpeedPreset};
    use crate::monitor::HostMonitor;
    use std::fs;

    #[test]
    fn launch_admission_protects_host_memory_and_temperature() {
        let safe = HostStats {
            memory_available_bytes: 9 * 1024 * 1024 * 1024,
            cpu_temperature_c: Some(89.9),
            ..HostStats::default()
        };
        assert_eq!(launch_block_reason(&safe), None);

        let unavailable_or_exhausted = HostStats {
            memory_available_bytes: 0,
            ..safe.clone()
        };
        assert!(
            launch_block_reason(&unavailable_or_exhausted)
                .unwrap()
                .contains("8 GB")
        );

        let exact_boundary = HostStats {
            memory_available_bytes: 8 * 1024 * 1024 * 1024,
            ..safe.clone()
        };
        assert_eq!(launch_block_reason(&exact_boundary), None);

        let low_memory = HostStats {
            memory_available_bytes: 7 * 1024 * 1024 * 1024,
            ..safe.clone()
        };
        assert!(launch_block_reason(&low_memory).unwrap().contains("8 GB"));

        let too_hot = HostStats {
            memory_available_bytes: 12 * 1024 * 1024 * 1024,
            cpu_temperature_c: Some(90.0),
            ..safe
        };
        assert!(launch_block_reason(&too_hot).unwrap().contains("90"));

        let missing_temperature = HostStats {
            memory_available_bytes: 12 * 1024 * 1024 * 1024,
            cpu_temperature_c: None,
            ..HostStats::default()
        };
        assert!(
            launch_block_reason(&missing_temperature)
                .unwrap()
                .contains("could not verify")
        );

        let invalid_temperature = HostStats {
            cpu_temperature_c: Some(f32::NAN),
            ..missing_temperature
        };
        assert!(
            launch_block_reason(&invalid_temperature)
                .unwrap()
                .contains("could not verify")
        );
    }

    #[test]
    fn launch_entry_samples_and_enforces_admission() {
        let root =
            std::env::temp_dir().join(format!("emumi-admission-test-{}", std::process::id()));
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        fs::create_dir_all(&proc_root).unwrap();
        fs::create_dir_all(&sys_root).unwrap();
        fs::write(proc_root.join("stat"), "cpu 100 0 100 800 0 0 0 0\n").unwrap();
        fs::write(
            proc_root.join("meminfo"),
            "MemTotal: 16777216 kB\nMemAvailable: 0 kB\n",
        )
        .unwrap();
        let mut monitor = HostMonitor::for_roots(proc_root, sys_root);

        let result = enforce_launch_admission(&mut monitor);

        assert!(result.unwrap_err().contains("8 GB"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn staged_start_waits_twenty_seconds_after_boot() {
        let observed = std::cell::Cell::new(std::time::Duration::ZERO);
        settle_after_boot_with(|duration| {
            observed.set(duration);
            async {}
        })
        .await;
        assert_eq!(
            observed.get(),
            std::time::Duration::from_secs(STARTUP_SETTLE_SECONDS)
        );
        assert_eq!(STARTUP_SETTLE_SECONDS, 20);
    }

    #[tokio::test]
    async fn startup_cpu_boost_covers_the_post_ready_game_launch_window() {
        let observed_delay = std::cell::Cell::new(std::time::Duration::ZERO);
        let applied = std::cell::Cell::new(false);

        settle_cpu_quota_with(
            |duration| {
                observed_delay.set(duration);
                async {}
            },
            || {
                applied.set(true);
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(POST_READY_CPU_BOOST_SECONDS, 120);
        assert_eq!(SETTLED_CPU_QUOTA_PERCENT, 120);
        assert_eq!(observed_delay.get(), std::time::Duration::from_secs(120));
        assert!(applied.get());
    }

    #[test]
    fn intel_choice_uses_hardware_not_an_unknown_emulator_gpu_mode() {
        let options = ProfileOptions {
            gpu_mode: "host-intel".into(),
            ..ProfileOptions::default()
        };
        let args = launch_args(&options);
        assert!(args.windows(2).any(|pair| pair == ["-gpu", "host"]));
        assert!(!args.iter().any(|arg| arg == "host-intel"));
        assert!(args.contains(&"-no-snapshot-load".into()));
    }

    #[test]
    fn disables_emulator_audio_when_profile_is_muted() {
        let options = ProfileOptions {
            mute_audio: true,
            ..ProfileOptions::default()
        };

        assert!(launch_args(&options).contains(&"-no-audio".to_owned()));
    }

    #[test]
    fn lean_gaming_uses_small_hardware_accelerated_devices() {
        let options = ProfileOptions::default();
        let args = launch_args(&options);
        assert_eq!(options.speed, SpeedPreset::LeanGaming);
        assert_eq!(options.cores, 2);
        assert_eq!(options.memory_mb, 4096);
        assert_eq!(options.gpu_mode, "host-intel");
        assert!(options.disable_vulkan);
        assert!(options.host_memory_policy);
        assert_eq!(options.refresh_rate_hz, 30);
        assert!(options.suspend_store_during_automation);
        assert_eq!(options.memory_high_mb, 5632);
        assert_eq!(options.memory_max_mb, 6656);
        assert!(args.windows(2).any(|pair| pair == ["-gpu", "host"]));
        assert!(args.windows(2).any(|pair| pair == ["-feature", "-Vulkan"]));
        assert!(args.windows(2).any(|pair| pair == ["-vsync-rate", "30"]));
        assert!(args.contains(&"-no-audio".to_owned()));
        assert!(args.contains(&"-no-snapshot-load".to_owned()));
    }

    #[test]
    fn non_lean_profiles_only_clean_boot_when_requested() {
        let normal = ProfileOptions {
            speed: SpeedPreset::Efficient,
            cold_boot: false,
            ..ProfileOptions::default()
        };
        assert!(!launch_args(&normal).contains(&"-no-snapshot-load".to_owned()));

        let recovery = ProfileOptions {
            cold_boot: true,
            ..normal
        };
        assert!(launch_args(&recovery).contains(&"-no-snapshot-load".to_owned()));
    }

    #[test]
    fn uses_frostguard_friendly_resolution_by_default() {
        let args = launch_args(&ProfileOptions::default());
        let skin = args.iter().position(|arg| arg == "-skin").unwrap();

        assert_eq!(args.get(skin + 1).map(String::as_str), Some("720x1280"));
    }

    #[test]
    fn enforces_frostguard_resolution_for_every_profile() {
        for picture in [
            PicturePreset::Compact,
            PicturePreset::Phone,
            PicturePreset::Sharp,
        ] {
            let options = ProfileOptions {
                picture,
                ..ProfileOptions::default()
            };
            let args = launch_args(&options);
            let skin = args.iter().position(|arg| arg == "-skin").unwrap();

            assert_eq!(args.get(skin + 1).map(String::as_str), Some("720x1280"));
        }
    }

    #[test]
    fn automation_safe_peripherals_are_disabled_by_default() {
        let options = ProfileOptions::default();
        let args = launch_args(&options);

        assert!(options.headless_automation);
        assert!(options.mute_audio);
        assert!(args.contains(&"-no-window".to_owned()));
        assert!(args.contains(&"-no-audio".to_owned()));
        assert!(args.contains(&"-no-boot-anim".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-camera-back", "none"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-camera-front", "none"])
        );
    }

    #[test]
    fn launches_with_the_profiles_stable_adb_slot() {
        let options = ProfileOptions {
            adb_port: Some(5558),
            ..ProfileOptions::default()
        };
        let args = launch_args(&options);
        let port = args.iter().position(|arg| arg == "-port").unwrap();

        assert_eq!(args.get(port + 1).map(String::as_str), Some("5558"));
    }

    #[test]
    fn headless_automation_keeps_adb_and_disables_window_peripherals() {
        let options = ProfileOptions {
            adb_port: Some(5554),
            headless_automation: true,
            ..ProfileOptions::default()
        };
        let args = launch_args(&options);

        assert!(args.contains(&"-no-window".to_owned()));
        assert!(args.contains(&"-no-boot-anim".to_owned()));
        assert!(args.windows(2).any(|pair| pair == ["-camera-back", "none"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-camera-front", "none"])
        );
        assert!(args.windows(2).any(|pair| pair == ["-port", "5554"]));
    }

    #[test]
    fn vulkan_is_disabled_by_default_but_can_be_enabled_explicitly() {
        assert!(
            launch_args(&ProfileOptions::default())
                .windows(2)
                .any(|pair| pair == ["-feature", "-Vulkan"])
        );
        let options = ProfileOptions {
            disable_vulkan: false,
            ..ProfileOptions::default()
        };
        assert!(!launch_args(&options).contains(&"-Vulkan".to_owned()));
    }

    #[test]
    fn validates_conservative_per_emulator_memory_limits() {
        let safe = ProfileOptions {
            host_memory_policy: true,
            ..ProfileOptions::default()
        };
        assert!(validate_profile_options(&safe).is_ok());
        let unsafe_high = ProfileOptions {
            memory_high_mb: 3840,
            ..safe.clone()
        };
        assert!(validate_profile_options(&unsafe_high).is_err());
        let inverted = ProfileOptions {
            memory_high_mb: 6144,
            memory_max_mb: 5120,
            ..safe
        };
        assert!(validate_profile_options(&inverted).is_err());
    }
}
