use crate::{
    android::AndroidTools,
    config::AppConfig,
    cpu_policy::{CpuQuotaPolicy, ThermalDecision, startup_thermal_decision},
    manager_guard::ManagerGuard,
    model::{HostStats, LaunchAdmission, PicturePreset, ProfileOptions, SpeedPreset},
    monitor::{HostMonitor, sample_devices},
    netsimd_log::{self, GuardOutcome},
    snapshot_recovery::{self, RecoveryOutcome},
    supervisor::{DevicePhase, DeviceStatus, DeviceSupervisor, Observation, SupervisorAction},
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
const STARTUP_CPU_QUOTA_PERCENT: u16 = 80;
const CPU_CONTROL_SAMPLE_SECONDS: u64 = 1;
const NETSIMD_LOG_LIMIT_BYTES: u64 = 100 * 1024 * 1024;
const NETSIMD_LOG_CHECK_SECONDS: u64 = 5;

type Shared = Arc<Mutex<Runtime>>;
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiMessage>)>;

pub struct EmuMiApp;

struct Runtime {
    _manager_guard: ManagerGuard,
    config: AppConfig,
    logs: Vec<ApiLog>,
    monitor: HostMonitor,
    rectangular_watchers: BTreeSet<String>,
    starting_profiles: BTreeSet<String>,
    startup_generations: BTreeMap<String, u64>,
    queued_profiles: VecDeque<String>,
    memory_warnings: BTreeMap<String, String>,
    cpu_controller_scopes: BTreeSet<String>,
    profile_scopes: BTreeMap<String, String>,
    supervisor: DeviceSupervisor,
}

#[derive(Clone, Serialize)]
struct ApiLog {
    at: u64,
    level: &'static str,
    message: String,
}

#[derive(Serialize)]
struct AppState {
    manager_id: String,
    profiles: Vec<crate::model::AndroidProfile>,
    profile_options: BTreeMap<String, ProfileOptions>,
    starting_profiles: BTreeSet<String>,
    queued_profiles: Vec<String>,
    device_supervision: BTreeMap<String, DeviceStatus>,
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
        let manager_guard = ManagerGuard::acquire()?;
        let config = AppConfig::load();
        let supervisor = if config.lifecycle_states.is_empty() {
            DeviceSupervisor::new(config.desired_running_profiles.clone())
        } else {
            DeviceSupervisor::restore(config.lifecycle_states.clone())
        };
        let runtime = Arc::new(Mutex::new(Runtime {
            _manager_guard: manager_guard,
            config,
            logs: vec![ApiLog::new("success", "EmuMi is ready")],
            monitor: HostMonitor::default(),
            rectangular_watchers: BTreeSet::new(),
            starting_profiles: BTreeSet::new(),
            startup_generations: BTreeMap::new(),
            queued_profiles: VecDeque::new(),
            memory_warnings: BTreeMap::new(),
            cpu_controller_scopes: BTreeSet::new(),
            profile_scopes: BTreeMap::new(),
            supervisor,
        }));
        spawn_netsimd_log_guard(Arc::clone(&runtime));
        spawn_device_supervisor(Arc::clone(&runtime));

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
            .with_state(Arc::clone(&runtime));

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let url = format!("http://{address}");
        runtime
            .lock()
            .unwrap()
            ._manager_guard
            .publish_endpoint(&url)?;
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                eprintln!("EmuMi server stopped: {error}");
            }
        });
        Ok(url)
    }
}

fn spawn_netsimd_log_guard(runtime: Shared) {
    let Some(log_path) = netsimd_log::default_log_path() else {
        runtime.lock().unwrap().log(
            "warning",
            "Disk guard could not determine the per-user netsimd log path",
        );
        return;
    };
    tokio::spawn(async move {
        let mut last_error = None;
        loop {
            match netsimd_log::enforce_log_limit(&log_path, NETSIMD_LOG_LIMIT_BYTES) {
                Ok(GuardOutcome::Truncated { previous_bytes }) => {
                    last_error = None;
                    runtime.lock().unwrap().log(
                        "warning",
                        format!(
                            "Disk guard truncated runaway netsimd log at {} MiB",
                            previous_bytes / (1024 * 1024)
                        ),
                    );
                }
                Ok(GuardOutcome::Refused(reason)) => {
                    let message = format!("Disk guard refused {}: {reason}", log_path.display());
                    if last_error.as_deref() != Some(message.as_str()) {
                        runtime.lock().unwrap().log("warning", message.clone());
                        last_error = Some(message);
                    }
                }
                Ok(GuardOutcome::Absent | GuardOutcome::Unchanged) => {
                    last_error = None;
                }
                Err(error) => {
                    let message =
                        format!("Disk guard could not limit {}: {error}", log_path.display());
                    if last_error.as_deref() != Some(message.as_str()) {
                        runtime.lock().unwrap().log("warning", message.clone());
                        last_error = Some(message);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(NETSIMD_LOG_CHECK_SECONDS)).await;
        }
    });
}

fn spawn_device_supervisor(runtime: Shared) {
    tokio::spawn(async move {
        loop {
            let (tools, desired, phases, known_scopes, starting) = match runtime.lock() {
                Ok(runtime) => (
                    runtime.tools(),
                    runtime.supervisor.desired_profiles(),
                    runtime.supervisor.statuses(),
                    runtime.profile_scopes.clone(),
                    runtime.starting_profiles.clone(),
                ),
                Err(_) => return,
            };
            let profiles = tools.discover_profiles();
            let owned_scopes = tools.owned_scopes();
            let host_temperature = HostMonitor::default().sample().cpu_temperature_c;
            let now = now_seconds();
            let mut observations = Vec::new();
            for name in desired {
                let running = profiles
                    .iter()
                    .find(|profile| profile.name == name && profile.is_running());
                let scope = known_scopes.get(&name).cloned().or_else(|| {
                    let prefix = format!("emumi-{name}-");
                    owned_scopes
                        .iter()
                        .filter(|unit| unit.starts_with(&prefix))
                        .max()
                        .cloned()
                });
                let phase = phases.get(&name).map(|status| status.phase);
                let running = running.is_some();
                let boot_completed = running && tools.profile_boot_completed(&name);
                let Some(observation) = classify_supervisor_observation(
                    running,
                    boot_completed,
                    starting.contains(&name),
                    phase,
                    scope.clone(),
                ) else {
                    continue;
                };
                observations.push((name, scope, observation));
            }

            let (actions, config_to_save) = match runtime.lock() {
                Ok(mut runtime) => {
                    for (name, scope, observation) in observations {
                        if let Some(scope) = scope {
                            runtime.profile_scopes.insert(name.clone(), scope);
                        }
                        runtime.supervisor.observe(&name, observation, now);
                    }
                    let mut thermal_changed = false;
                    if let Some(temperature) = host_temperature {
                        for (name, status) in &phases {
                            if status.phase == DevicePhase::ThermalHold {
                                thermal_changed |=
                                    runtime
                                        .supervisor
                                        .observe_cool_sample(name, temperature, now);
                            }
                        }
                    }
                    let actions = runtime.supervisor.due_actions(now);
                    let config = (thermal_changed || !actions.is_empty())
                        .then(|| runtime.lifecycle_config_snapshot());
                    (actions, config)
                }
                Err(_) => return,
            };
            if let Some(config) = config_to_save {
                let _ = config.save();
            }

            for action in actions {
                let SupervisorAction::Recover { profile, scope } = action;
                let (tools, port) = match runtime.lock() {
                    Ok(runtime) => (
                        runtime.tools(),
                        runtime
                            .config
                            .profile_options
                            .get(&profile)
                            .and_then(|options| options.adb_port),
                    ),
                    Err(_) => return,
                };
                if let Some(scope) = scope.or_else(|| tools.owned_scope_for_profile(&profile)) {
                    let _ = tools.stop_scope_and_wait(&scope);
                }
                if let Some(port) = port {
                    tools.disconnect_port(port);
                }
                let _ = tools.cleanup_profile_runtime(&profile);
                if let Err((_, Json(error))) =
                    start_named_profile_internal(&runtime, &profile, false)
                    && let Ok(mut runtime) = runtime.lock()
                {
                    runtime.log(
                        "warning",
                        format!("Recovery for {profile} is delayed: {}", error.message),
                    );
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
}

fn classify_supervisor_observation(
    running: bool,
    boot_completed: bool,
    starting: bool,
    previous_phase: Option<DevicePhase>,
    scope: Option<String>,
) -> Option<Observation> {
    if running && boot_completed {
        return Some(Observation::Healthy);
    }
    if running && matches!(previous_phase, Some(DevicePhase::Ready)) {
        return Some(Observation::Unresponsive { scope });
    }
    if running || starting {
        return Some(Observation::Booting { scope });
    }
    if scope.is_some() {
        return Some(Observation::Unresponsive { scope });
    }
    if matches!(previous_phase, Some(DevicePhase::Ready)) {
        return Some(Observation::Absent { scope: None });
    }
    if matches!(
        previous_phase,
        Some(
            DevicePhase::Admitting
                | DevicePhase::Launching
                | DevicePhase::Booting
                | DevicePhase::Settling
                | DevicePhase::HalfOpen
        )
    ) {
        return Some(Observation::Exited { scope: None });
    }
    None
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

    fn persist_desired_profiles(&mut self) -> Result<(), String> {
        self.lifecycle_config_snapshot()
            .save()
            .map(|_| ())
            .map_err(|error| format!("Could not save device lifecycle state: {error}"))
    }

    fn lifecycle_config_snapshot(&mut self) -> AppConfig {
        self.config.desired_running_profiles = self.supervisor.desired_profiles();
        self.config.lifecycle_states = self.supervisor.persisted_states();
        self.config.clone()
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
    let tools = lock(&shared)?.tools();
    let discovery_tools = tools.clone();
    let (profiles, system_images, device_stats) = tokio::task::spawn_blocking(move || {
        let profiles = discovery_tools.discover_profiles();
        let system_images = discovery_tools
            .installed_system_images()
            .into_iter()
            .map(|image| ImageDto {
                label: image.label(),
                package_id: image.package_id,
            })
            .collect();
        let device_stats = sample_devices(&profiles);
        (profiles, system_images, device_stats)
    })
    .await
    .map_err(|error| {
        error_tuple(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Device discovery worker failed: {error}"),
        )
    })?;
    let mut runtime = lock(&shared)?;
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
    let active_scopes = device_stats
        .iter()
        .filter_map(|sample| sample.scope_name.clone())
        .collect::<BTreeSet<_>>();
    for sample in &device_stats {
        if let Some(scope) = &sample.scope_name {
            runtime
                .profile_scopes
                .insert(sample.profile_name.clone(), scope.clone());
        }
    }
    runtime
        .cpu_controller_scopes
        .retain(|scope| active_scopes.contains(scope));
    let starting_profiles = runtime.starting_profiles.clone();
    let new_cpu_controllers = new_cpu_controller_candidates(
        &device_stats,
        &starting_profiles,
        &mut runtime.cpu_controller_scopes,
    );
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
        manager_id: runtime._manager_guard.manager_id().to_owned(),
        profiles,
        profile_options: runtime.config.profile_options.clone(),
        starting_profiles: runtime.starting_profiles.clone(),
        queued_profiles: runtime.queued_profiles.iter().cloned().collect(),
        device_supervision: runtime.supervisor.statuses(),
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
    drop(runtime);
    for (profile_name, scope_unit) in new_cpu_controllers {
        schedule_adaptive_cpu_quota(Arc::clone(&shared), profile_name, Some(scope_unit));
    }
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
    start_named_profile_internal(shared, name, true)
}

fn start_named_profile_internal(
    shared: &Shared,
    name: &str,
    user_requested: bool,
) -> ApiResult<ApiMessage> {
    let tools = {
        let runtime = lock(shared)?;
        if !user_requested && !runtime.supervisor.is_desired(name) {
            return Ok(message("Recovery was cancelled by an intentional stop"));
        }
        runtime.tools()
    };
    let profile = tools
        .discover_profiles()
        .into_iter()
        .find(|profile| profile.name == name)
        .ok_or_else(|| error_tuple(StatusCode::NOT_FOUND, "Profile not found"))?;
    let discovered_profiles = tools.discover_profiles();
    let running = discovered_profiles
        .into_iter()
        .filter(|candidate| candidate.is_running())
        .count();
    let host_stats = HostMonitor::default().sample();

    let (options, startup_generation, config_to_save) = {
        let mut runtime = lock(shared)?;
        if !user_requested && !runtime.supervisor.is_desired(name) {
            return Ok(message("Recovery was cancelled by an intentional stop"));
        }
        if runtime.starting_profiles.contains(name) {
            return Ok(message("Android is already starting"));
        }
        if runtime.queued_profiles.iter().any(|queued| queued == name) {
            return Ok(message("Android is already queued"));
        }
        if profile.is_running() {
            if user_requested {
                let generation = runtime
                    .supervisor
                    .begin_admission(name, now_seconds(), true)
                    .map_err(|message| error_tuple(StatusCode::CONFLICT, message))?;
                runtime.supervisor.admission_succeeded(name, generation);
                runtime.supervisor.complete_ready(name, generation);
                let config = runtime.lifecycle_config_snapshot();
                drop(runtime);
                config.save().map_err(|error| {
                    error_tuple(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Could not save device lifecycle state: {error}"),
                    )
                })?;
            }
            return Ok(message("Android is already running"));
        }
        if running + runtime.starting_profiles.len() + runtime.queued_profiles.len()
            >= MAX_RUNNING_PROFILES
        {
            return api_error(
                StatusCode::CONFLICT,
                "This computer is limited to four running Androids. Stop an idle one first.",
            );
        }
        let generation = if user_requested {
            runtime
                .supervisor
                .begin_admission(name, now_seconds(), true)
                .map_err(|message| error_tuple(StatusCode::CONFLICT, message))?
        } else {
            runtime.supervisor.generation(name).ok_or_else(|| {
                error_tuple(StatusCode::CONFLICT, "Recovery generation is unavailable")
            })?
        };
        if !runtime.starting_profiles.is_empty() {
            runtime.queued_profiles.push_back(name.to_owned());
            runtime.supervisor.mark_queued(name);
            runtime.log(
                "info",
                format!("Queued {name} until the current Android settles"),
            );
            let config = runtime.lifecycle_config_snapshot();
            drop(runtime);
            config.save().map_err(|error| {
                error_tuple(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Could not save device lifecycle state: {error}"),
                )
            })?;
            return Ok(message("Android is queued for an adaptive start"));
        }
        if let Some(reason) = launch_block_reason(&host_stats) {
            runtime
                .supervisor
                .reject_admission(name, reason.clone(), now_seconds());
            let config = runtime.lifecycle_config_snapshot();
            drop(runtime);
            let _ = config.save();
            return Err(error_tuple(StatusCode::CONFLICT, reason));
        }
        runtime.supervisor.admission_succeeded(name, generation);
        let options = runtime
            .config
            .profile_options
            .entry(name.to_owned())
            .or_default()
            .clone();
        runtime.starting_profiles.insert(name.to_owned());
        runtime
            .startup_generations
            .insert(name.to_owned(), generation);
        let config = runtime.lifecycle_config_snapshot();
        (options, generation, config)
    };
    config_to_save.save().map_err(|error| {
        error_tuple(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save device lifecycle state: {error}"),
        )
    })?;

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
        .and_then(|_| {
            tools.start(
                &profile,
                &launch_args(&options),
                &options,
                LaunchAdmission::granted(),
            )
        });
    let launch_handle = match start_result {
        Ok(launch_handle) => launch_handle,
        Err(message) => {
            if let Ok(mut runtime) = shared.lock() {
                runtime.starting_profiles.remove(name);
                runtime.startup_generations.remove(name);
                runtime
                    .supervisor
                    .record_failure(name, message.clone(), None, now_seconds());
                let config = runtime.lifecycle_config_snapshot();
                drop(runtime);
                let _ = config.save();
            }
            return Err(error_tuple(StatusCode::BAD_REQUEST, message));
        }
    };
    let scope_unit = launch_handle.scope_unit.clone();
    let config_to_save = {
        let mut runtime = lock(shared)?;
        if runtime.startup_generations.get(name) != Some(&startup_generation)
            || !runtime.supervisor.is_desired(name)
        {
            drop(runtime);
            if let Some(scope) = &scope_unit {
                let _ = tools.stop_scope_and_wait(scope);
            }
            let _ = tools.cleanup_profile_runtime(name);
            return Ok(message("Android start was cancelled"));
        }
        if let Some(scope) = &scope_unit {
            runtime
                .profile_scopes
                .insert(name.to_owned(), scope.clone());
            runtime.supervisor.mark_launching(
                name,
                startup_generation,
                scope.clone(),
                launch_handle.launcher_pid,
            );
            runtime.supervisor.mark_booting(name, startup_generation);
        }
        if options.rectangular_display {
            runtime.rectangular_watchers.insert(name.to_owned());
        }
        if options.cold_boot
            && let Some(saved) = runtime.config.profile_options.get_mut(name)
        {
            saved.cold_boot = false;
        }
        if options.speed == SpeedPreset::LeanGaming {
            runtime.log(
                "success",
                format!("Starting {name} with a clean Lean Gaming boot"),
            );
        } else {
            runtime.log("success", format!("Starting {name}"));
        }
        runtime.lifecycle_config_snapshot()
    };
    let _ = config_to_save.save();
    if options.rectangular_display {
        tools.watch_rectangular_display_after_start(name);
    }
    if options.speed == SpeedPreset::LeanGaming {
        tools.watch_lean_profile_after_start(name, options.suspend_store_during_automation);
    }
    register_adaptive_cpu_controller(shared, name.to_owned(), scope_unit.clone());
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
            let tools = {
                let Ok(runtime) = shared.lock() else {
                    return;
                };
                if runtime.startup_generations.get(&name) != Some(&startup_generation)
                    || !runtime.supervisor.is_desired(&name)
                {
                    return;
                }
                runtime.tools()
            };
            if tools.profile_boot_completed(&name) {
                booted = true;
                break;
            }
        }

        let completion = if booted {
            if let Ok(mut runtime) = shared.lock()
                && runtime.startup_generations.get(&name) == Some(&startup_generation)
            {
                runtime.supervisor.mark_settling(&name, startup_generation);
            }
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

        let snapshot_recovered = if booted {
            false
        } else {
            matches!(
                snapshot_recovery::recover_owned_snapshot_failure(&name, now_seconds()),
                Ok(RecoveryOutcome::Quarantined)
            )
        };

        let mut timed_out_cleanup = None;
        let mut config_to_save = None;
        if let Ok(mut runtime) = shared.lock() {
            if runtime.startup_generations.get(&name) != Some(&startup_generation) {
                return;
            }
            runtime.starting_profiles.remove(&name);
            runtime.startup_generations.remove(&name);
            if booted {
                if runtime.supervisor.complete_ready(&name, startup_generation) {
                    runtime.log("success", format!("{name} is ready"));
                }
            } else {
                let cleanup_scope = scope_unit
                    .clone()
                    .or_else(|| runtime.profile_scopes.get(&name).cloned());
                let snapshot_retry = snapshot_recovered
                    && runtime.supervisor.schedule_snapshot_recovery(
                        &name,
                        startup_generation,
                        now_seconds(),
                    );
                if snapshot_retry {
                    if let Some(options) = runtime.config.profile_options.get_mut(&name) {
                        options.cold_boot = true;
                    }
                    runtime.log(
                        "warning",
                        format!(
                            "{name} snapshot failed; quarantined default_boot for one cold retry"
                        ),
                    );
                } else {
                    runtime.supervisor.record_failure(
                        &name,
                        "Emulator exceeded its 120 second boot deadline",
                        cleanup_scope.clone(),
                        now_seconds(),
                    );
                }
                timed_out_cleanup = Some((
                    runtime.tools(),
                    cleanup_scope,
                    runtime
                        .config
                        .profile_options
                        .get(&name)
                        .and_then(|options| options.adb_port),
                ));
                runtime.log("warning", format!("{name} did not finish starting"));
            }
            config_to_save = Some(runtime.lifecycle_config_snapshot());
        }
        if let Some(config) = config_to_save {
            let _ = config.save();
        }
        if let Some((tools, scope, port)) = timed_out_cleanup {
            if let Some(scope) = scope {
                let _ = tools.stop_scope_and_wait(&scope);
            }
            if let Some(port) = port {
                tools.disconnect_port(port);
            }
            let _ = tools.cleanup_profile_runtime(&name);
        }
        start_next_queued_profile(&shared, completion);
        let _ = scope_unit;
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

fn schedule_adaptive_cpu_quota(shared: Shared, name: String, scope_unit: Option<String>) {
    let Some(scope_unit) = scope_unit else {
        return;
    };
    tokio::spawn(async move {
        let logical_cpus = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        let mut policy = CpuQuotaPolicy::new(STARTUP_CPU_QUOTA_PERCENT);
        let mut host_monitor = HostMonitor::default();
        let mut cool_startup_samples = 0_u8;
        loop {
            let (tools, status, port) = {
                let Ok(runtime) = shared.lock() else {
                    return;
                };
                (
                    runtime.tools(),
                    runtime.supervisor.statuses().get(&name).cloned(),
                    runtime
                        .config
                        .profile_options
                        .get(&name)
                        .and_then(|options| options.adb_port),
                )
            };
            let Some(status) = status else {
                break;
            };
            let generation = status.generation;
            let profiles = tools.discover_profiles();
            let profile_is_running = profiles
                .iter()
                .any(|profile| profile.name == name && profile.is_running());
            let running_emulators = profiles
                .iter()
                .filter(|profile| profile.is_running())
                .count();
            let sample = match tools.scope_cpu_stat(&scope_unit) {
                Ok(sample) => sample,
                Err(error) => {
                    if !profile_is_running {
                        break;
                    }
                    if let Ok(mut runtime) = shared.lock() {
                        runtime.log(
                            "warning",
                            format!("Stopped adaptive CPU control for {name}: {error}"),
                        );
                    }
                    break;
                }
            };
            let host_stats = host_monitor.sample();
            let starting = matches!(
                status.phase,
                DevicePhase::Admitting
                    | DevicePhase::Launching
                    | DevicePhase::Booting
                    | DevicePhase::Settling
            );
            let startup_decision =
                startup_thermal_decision(sample.quota_percent, host_stats.cpu_temperature_c);
            if startup_decision == ThermalDecision::Stop
                || host_stats
                    .cpu_temperature_c
                    .is_some_and(|temperature| temperature.is_finite() && temperature >= 90.0)
            {
                let _ = tools.stop_scope_and_wait(&scope_unit);
                if let Some(port) = port {
                    tools.disconnect_port(port);
                }
                let _ = tools.cleanup_profile_runtime(&name);
                if let Ok(mut runtime) = shared.lock() {
                    runtime.starting_profiles.remove(&name);
                    runtime.startup_generations.remove(&name);
                    runtime.profile_scopes.remove(&name);
                    runtime.supervisor.enter_thermal_hold(
                        &name,
                        generation,
                        host_stats.cpu_temperature_c.unwrap_or(90.0),
                        now_seconds(),
                    );
                    runtime.log(
                        "warning",
                        format!("Stopped {name} at the 90 C thermal limit"),
                    );
                    let config = runtime.lifecycle_config_snapshot();
                    drop(runtime);
                    let _ = config.save();
                }
                break;
            }

            let desired_quota = if starting {
                if host_stats
                    .cpu_temperature_c
                    .is_some_and(|temperature| temperature < 75.0)
                {
                    cool_startup_samples = cool_startup_samples.saturating_add(1);
                } else {
                    cool_startup_samples = 0;
                }
                match startup_decision {
                    ThermalDecision::Quota(percent) if percent > sample.quota_percent => {
                        if cool_startup_samples >= 5 {
                            cool_startup_samples = 0;
                            Some(percent)
                        } else {
                            None
                        }
                    }
                    ThermalDecision::Quota(percent) => Some(percent),
                    ThermalDecision::Keep | ThermalDecision::Stop => None,
                }
            } else {
                policy.observe(
                    sample,
                    running_emulators,
                    logical_cpus,
                    host_stats.cpu_percent,
                    host_stats.cpu_temperature_c,
                )
            };

            if let Some(percent) = desired_quota {
                let weight = if percent > 120 { 100 } else { 25 };
                match tools.set_scope_cpu_policy(&scope_unit, percent, weight) {
                    Ok(()) => {
                        if let Ok(mut runtime) = shared.lock() {
                            runtime.supervisor.update_telemetry(
                                &name,
                                generation,
                                percent,
                                host_stats.cpu_temperature_c,
                            );
                            runtime.log(
                                "success",
                                format!(
                                    "Adjusted {name} CPU quota to {percent}% with weight {weight}"
                                ),
                            );
                        }
                    }
                    Err(error) => {
                        if let Ok(mut runtime) = shared.lock() {
                            runtime.log(
                                "warning",
                                format!("Could not adjust {name} CPU quota: {error}"),
                            );
                        }
                    }
                }
            } else if let Ok(mut runtime) = shared.lock() {
                runtime.supervisor.update_telemetry(
                    &name,
                    generation,
                    sample.quota_percent,
                    host_stats.cpu_temperature_c,
                );
            }
            tokio::time::sleep(std::time::Duration::from_secs(CPU_CONTROL_SAMPLE_SECONDS)).await;
        }
        if let Ok(mut runtime) = shared.lock() {
            runtime.cpu_controller_scopes.remove(&scope_unit);
        }
    });
}

fn register_adaptive_cpu_controller(shared: &Shared, name: String, scope_unit: Option<String>) {
    let Some(scope_unit) = scope_unit else {
        return;
    };
    let should_start = shared
        .lock()
        .map(|mut runtime| runtime.cpu_controller_scopes.insert(scope_unit.clone()))
        .unwrap_or(false);
    if should_start {
        schedule_adaptive_cpu_quota(Arc::clone(shared), name, Some(scope_unit));
    }
}

fn new_cpu_controller_candidates(
    samples: &[crate::model::DeviceHostStats],
    starting_profiles: &BTreeSet<String>,
    registered_scopes: &mut BTreeSet<String>,
) -> Vec<(String, String)> {
    samples
        .iter()
        .filter(|sample| !starting_profiles.contains(&sample.profile_name))
        .filter_map(|sample| {
            let scope = sample.scope_name.clone()?;
            registered_scopes
                .insert(scope.clone())
                .then(|| (sample.profile_name.clone(), scope))
        })
        .collect()
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
        match start_named_profile_internal(shared, &next, false) {
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
    // Persist cancellation before any ADB, systemd or filesystem operation so a
    // blocked probe cannot delay or reverse an intentional stop.
    runtime.starting_profiles.remove(name);
    runtime.startup_generations.remove(name);
    runtime.queued_profiles.retain(|queued| queued != name);
    let tools = runtime.tools();
    runtime.supervisor.request_stop(name);
    let scope = runtime.profile_scopes.remove(name);
    let port = runtime
        .config
        .profile_options
        .get(name)
        .and_then(|options| options.adb_port);
    let config = runtime.lifecycle_config_snapshot();
    drop(runtime);
    config.save().map_err(|error| {
        error_tuple(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save device lifecycle state: {error}"),
        )
    })?;

    let shared = Arc::clone(shared);
    let name = name.to_owned();
    tokio::task::spawn_blocking(move || {
        let scope = scope.or_else(|| tools.owned_scope_for_profile(&name));
        let serial = tools
            .discover_profiles()
            .into_iter()
            .find(|profile| profile.name == name)
            .and_then(|profile| profile.running_serial);
        if let Some(serial) = &serial {
            let _ = tools.stop(serial);
            std::thread::sleep(std::time::Duration::from_millis(750));
        }
        if let Some(scope) = &scope {
            let _ = tools.stop_scope_and_wait(scope);
        }
        if let Some(port) = port {
            tools.disconnect_port(port);
        }
        let _ = tools.cleanup_profile_runtime(&name);
        if let Ok(mut runtime) = shared.lock() {
            runtime.supervisor.mark_cleaned(&name);
            runtime.log("info", format!("Stopped and cleaned up {name}"));
            let config = runtime.lifecycle_config_snapshot();
            drop(runtime);
            let _ = config.save();
        }
    });
    Ok(message("Android stop requested"))
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

    let readiness = port_readiness_from_state(
        &name,
        runtime.queued_profiles.iter().any(|queued| queued == &name),
        runtime.starting_profiles.contains(&name),
        runtime.supervisor.statuses().remove(&name),
    );

    Ok(Json(readiness))
}

fn port_readiness_from_state(
    name: &str,
    queued: bool,
    starting: bool,
    status: Option<DeviceStatus>,
) -> PortReadiness {
    if queued {
        return PortReadiness {
            ready: false,
            phase: "queued",
            message: format!("{name} is queued for startup"),
        };
    }
    if starting {
        return PortReadiness {
            ready: false,
            phase: "starting",
            message: format!("{name} is still completing its clean boot"),
        };
    }
    let Some(status) = status else {
        return PortReadiness {
            ready: false,
            phase: "stopped",
            message: format!("{name} is stopped"),
        };
    };
    match status.phase {
        DevicePhase::Ready => PortReadiness {
            ready: true,
            phase: "ready",
            message: format!("{name} is ready for app launch"),
        },
        DevicePhase::Queued => PortReadiness {
            ready: false,
            phase: "queued",
            message: format!("{name} is queued for startup"),
        },
        DevicePhase::Admitting
        | DevicePhase::Launching
        | DevicePhase::Booting
        | DevicePhase::Settling
        | DevicePhase::HalfOpen => PortReadiness {
            ready: false,
            phase: "starting",
            message: format!("{name} is still completing its clean boot"),
        },
        DevicePhase::Backoff | DevicePhase::SnapshotRecovery | DevicePhase::ThermalHold => {
            PortReadiness {
                ready: false,
                phase: "recovering",
                message: status
                    .last_failure
                    .unwrap_or_else(|| format!("{name} is recovering")),
            }
        }
        DevicePhase::CircuitOpen => PortReadiness {
            ready: false,
            phase: "degraded",
            message: status
                .last_failure
                .unwrap_or_else(|| format!("{name} needs attention")),
        },
        DevicePhase::Stopping | DevicePhase::Cleaned | DevicePhase::Stopped => PortReadiness {
            ready: false,
            phase: "stopped",
            message: format!("{name} is stopped"),
        },
    }
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
    runtime.supervisor.request_stop(&name);
    runtime.profile_scopes.remove(&name);
    runtime.config.profile_options.remove(&name);
    runtime
        .persist_desired_profiles()
        .map_err(|message| error_tuple(StatusCode::INTERNAL_SERVER_ERROR, message))?;
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
        if matches!(options.gpu_mode.as_str(), "host-intel" | "host-nvidia") {
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

#[cfg(test)]
fn enforce_launch_admission(monitor: &mut HostMonitor) -> Result<LaunchAdmission, String> {
    enforce_launch_admission_with(|| monitor.sample())
}

#[cfg(test)]
fn enforce_launch_admission_with(
    sample: impl FnOnce() -> HostStats,
) -> Result<LaunchAdmission, String> {
    launch_block_reason(&sample()).map_or_else(|| Ok(LaunchAdmission::granted()), Err)
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
        CPU_CONTROL_SAMPLE_SECONDS, STARTUP_CPU_QUOTA_PERCENT, STARTUP_SETTLE_SECONDS,
        classify_supervisor_observation, enforce_launch_admission, launch_args,
        launch_block_reason, new_cpu_controller_candidates, port_readiness_from_state,
        settle_after_boot_with, validate_profile_options,
    };
    use crate::model::{HostStats, PicturePreset, ProfileOptions, SpeedPreset};
    use crate::monitor::HostMonitor;
    use std::{collections::BTreeSet, fs};

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

    #[test]
    fn adaptive_cpu_control_starts_conservatively_and_reacts_quickly() {
        assert_eq!(STARTUP_CPU_QUOTA_PERCENT, 80);
        assert_eq!(CPU_CONTROL_SAMPLE_SECONDS, 1);
    }

    #[test]
    fn a_running_device_whose_boot_probe_stalls_is_unresponsive_not_booting() {
        assert_eq!(
            classify_supervisor_observation(
                true,
                false,
                false,
                Some(crate::supervisor::DevicePhase::Ready),
                Some("emumi-Device_3-123.scope".into()),
            ),
            Some(crate::supervisor::Observation::Unresponsive {
                scope: Some("emumi-Device_3-123.scope".into()),
            })
        );
    }

    #[test]
    fn readiness_uses_cached_supervision_without_waiting_for_adb() {
        let running = crate::supervisor::DeviceStatus {
            phase: crate::supervisor::DevicePhase::Ready,
            desired_running: true,
            ..Default::default()
        };
        let ready = port_readiness_from_state("Device_3", false, false, Some(running));
        assert!(ready.ready);
        assert_eq!(ready.phase, "ready");

        let recovering = crate::supervisor::DeviceStatus {
            phase: crate::supervisor::DevicePhase::Backoff,
            desired_running: true,
            last_failure: Some("health probe timed out".into()),
            ..Default::default()
        };
        let not_ready = port_readiness_from_state("Device_3", false, false, Some(recovering));
        assert!(!not_ready.ready);
        assert_eq!(not_ready.phase, "recovering");
    }

    #[test]
    fn running_scopes_are_adopted_once_after_manager_restart() {
        let samples = vec![
            crate::model::DeviceHostStats {
                profile_name: "Device_1".into(),
                scope_name: Some("emumi-Device_1-100.scope".into()),
                ..Default::default()
            },
            crate::model::DeviceHostStats {
                profile_name: "Device_3".into(),
                scope_name: Some("emumi-Device_3-300.scope".into()),
                ..Default::default()
            },
        ];
        let starting = BTreeSet::from(["Device_3".to_owned()]);
        let mut registered = BTreeSet::new();

        assert_eq!(
            new_cpu_controller_candidates(&samples, &starting, &mut registered),
            vec![("Device_1".to_owned(), "emumi-Device_1-100.scope".to_owned())]
        );
        assert!(new_cpu_controller_candidates(&samples, &starting, &mut registered).is_empty());

        let no_starting = BTreeSet::new();
        assert_eq!(
            new_cpu_controller_candidates(&samples, &no_starting, &mut registered),
            vec![("Device_3".to_owned(), "emumi-Device_3-300.scope".to_owned())]
        );
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
    fn explicit_nvidia_choice_uses_the_emulator_host_backend() {
        let options = ProfileOptions {
            gpu_mode: "host-nvidia".into(),
            ..ProfileOptions::default()
        };
        let args = launch_args(&options);

        assert!(args.windows(2).any(|pair| pair == ["-gpu", "host"]));
        assert!(!args.iter().any(|arg| arg == "host-nvidia"));
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
