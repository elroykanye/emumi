use crate::{
    android::{AndroidTools, scope_belongs_to_profile},
    config::AppConfig,
    manager_guard::ManagerGuard,
    model::{PicturePreset, ProfileOptions, SpeedPreset},
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
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const NETSIMD_LOG_LIMIT_BYTES: u64 = 100 * 1024 * 1024;
const NETSIMD_LOG_CHECK_SECONDS: u64 = 5;

type Shared = Arc<Mutex<Runtime>>;
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiMessage>)>;

pub struct EmuMiApp;

struct Runtime {
    _manager_guard: ManagerGuard,
    tools: AndroidTools,
    config: AppConfig,
    logs: Vec<ApiLog>,
    rectangular_watchers: BTreeSet<String>,
    starting_profiles: BTreeSet<String>,
    startup_generations: BTreeMap<String, u64>,
    profile_scopes: BTreeMap<String, String>,
    cached_profiles: Vec<crate::model::AndroidProfile>,
    cached_system_images: Vec<ImageDto>,
    discovery_in_flight: bool,
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
    device_supervision: BTreeMap<String, DeviceStatus>,
    system_images: Vec<ImageDto>,
    config: SettingsDto,
    health: HealthDto,
    logs: Vec<ApiLog>,
}

#[derive(Clone, Serialize)]
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
        let tools = AndroidTools::detect(&config.android_sdk_path, &config.jdk_path);
        let supervisor = if config.lifecycle_states.is_empty() {
            DeviceSupervisor::new(config.desired_running_profiles.clone())
        } else {
            DeviceSupervisor::restore(config.lifecycle_states.clone())
        };
        let runtime = Arc::new(Mutex::new(Runtime {
            _manager_guard: manager_guard,
            tools,
            config,
            logs: vec![ApiLog::new("success", "EmuMi is ready")],
            rectangular_watchers: BTreeSet::new(),
            starting_profiles: BTreeSet::new(),
            startup_generations: BTreeMap::new(),
            profile_scopes: BTreeMap::new(),
            cached_profiles: Vec::new(),
            cached_system_images: Vec::new(),
            discovery_in_flight: false,
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
            let now = now_seconds();
            let mut observations = Vec::new();
            let mut restored_startups = Vec::new();
            for name in desired {
                let running = profiles
                    .iter()
                    .find(|profile| profile.name == name && profile.is_running());
                let scope = known_scopes.get(&name).cloned().or_else(|| {
                    owned_scopes
                        .iter()
                        .filter(|unit| scope_belongs_to_profile(unit, &name))
                        .max()
                        .cloned()
                });
                let phase = phases.get(&name).map(|status| status.phase);
                let running = running.is_some();
                let boot_completed = running && tools.profile_boot_completed(&name);
                if boot_completed
                    && !starting.contains(&name)
                    && matches!(
                        phase,
                        Some(DevicePhase::Admitting)
                            | Some(DevicePhase::Launching)
                            | Some(DevicePhase::Booting)
                            | Some(DevicePhase::Settling)
                            | Some(DevicePhase::HalfOpen)
                    )
                    && let Some(status) = phases.get(&name)
                {
                    restored_startups.push((
                        name.clone(),
                        status.generation,
                        scope.clone(),
                        boot_completed,
                    ));
                } else if scope.is_some()
                    && !starting.contains(&name)
                    && matches!(
                        phase,
                        Some(DevicePhase::Admitting)
                            | Some(DevicePhase::Launching)
                            | Some(DevicePhase::Booting)
                            | Some(DevicePhase::Settling)
                            | Some(DevicePhase::HalfOpen)
                    )
                    && let Some(status) = phases.get(&name)
                {
                    restored_startups.push((name.clone(), status.generation, scope.clone(), false));
                }
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

            let had_observations = !observations.is_empty();
            let (actions, config_to_save, resumed) = match runtime.lock() {
                Ok(mut runtime) => {
                    for (name, scope, observation) in observations {
                        if let Some(scope) = scope {
                            runtime.profile_scopes.insert(name.clone(), scope);
                            if let Some(owned_scope) = runtime.profile_scopes.get(&name).cloned() {
                                runtime.supervisor.adopt_scope(&name, owned_scope);
                            }
                        }
                        let was_ready =
                            runtime
                                .supervisor
                                .status_snapshot(&name)
                                .is_some_and(|status| {
                                    status.desired_running && status.phase == DevicePhase::Ready
                                });
                        runtime.supervisor.observe(&name, observation, now);
                        if was_ready && !runtime.supervisor.is_desired(&name) {
                            runtime.profile_scopes.remove(&name);
                            runtime.log(
                                "info",
                                format!(
                                    "{name} was closed outside EmuMi; keeping it stopped until the next Start request"
                                ),
                            );
                        }
                    }
                    let mut resumed = Vec::new();
                    for (name, generation, scope, boot_completed) in restored_startups {
                        if runtime.starting_profiles.contains(&name)
                            || runtime.supervisor.generation(&name) != Some(generation)
                            || !runtime.supervisor.is_desired(&name)
                        {
                            continue;
                        }
                        runtime.starting_profiles.insert(name.clone());
                        runtime.startup_generations.insert(name.clone(), generation);
                        if boot_completed {
                            runtime.supervisor.mark_settling(&name, generation);
                        }
                        resumed.push((name, generation, scope));
                    }
                    let actions = runtime.supervisor.due_actions(now);
                    let config = (had_observations || !actions.is_empty() || !resumed.is_empty())
                        .then(|| runtime.lifecycle_config_snapshot());
                    (actions, config, resumed)
                }
                Err(_) => return,
            };
            if let Some(config) = config_to_save {
                let _ = config.save();
            }
            for (profile, generation, scope) in resumed {
                watch_profile_start(Arc::clone(&runtime), profile, generation, scope);
            }

            for action in actions {
                let SupervisorAction::Recover { profile, scope } = action;
                let (tools, status, port) = match runtime.lock() {
                    Ok(runtime) => (
                        runtime.tools(),
                        runtime.supervisor.status_snapshot(&profile),
                        runtime
                            .config
                            .profile_options
                            .get(&profile)
                            .and_then(|options| options.adb_port),
                    ),
                    Err(_) => return,
                };
                let Some(status) = status else {
                    continue;
                };
                let generation = status.generation;
                let owned_scope = scope.or(status.scope.clone());
                let cleanup = (|| {
                    if let Some(scope) = &owned_scope {
                        tools.terminate_scope_and_wait(scope, false)?;
                    }
                    if let Some(pid) = status.pid {
                        tools.wait_pid_gone(pid)?;
                    }
                    Ok::<(), String>(())
                })();
                if let Err(error) = cleanup {
                    if let Ok(mut runtime) = runtime.lock() {
                        runtime.supervisor.record_failure(
                            &profile,
                            format!("Recovery could not stop the previous scope: {error}"),
                            owned_scope,
                            now_seconds(),
                        );
                        runtime.log(
                            "error",
                            format!("Recovery for {profile} is delayed: {error}"),
                        );
                        let config = runtime.lifecycle_config_snapshot();
                        drop(runtime);
                        let _ = config.save();
                    }
                    continue;
                }
                if let Some(port) = port {
                    tools.disconnect_port(port);
                }
                let _ = tools.cleanup_profile_runtime(&profile);
                if let Ok(mut runtime) = runtime.lock() {
                    runtime.supervisor.confirm_ownership_gone(
                        &profile,
                        generation,
                        status.scope.as_deref(),
                        status.pid,
                    );
                    runtime.profile_scopes.remove(&profile);
                }
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
        return if matches!(previous_phase, Some(DevicePhase::Ready)) {
            Some(Observation::Unresponsive { scope })
        } else {
            Some(Observation::Booting { scope })
        };
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
        self.tools.clone()
    }

    fn log(&mut self, level: &'static str, message: impl Into<String>) {
        self.logs.push(ApiLog::new(level, message));
        if self.logs.len() > 200 {
            self.logs.drain(..self.logs.len() - 200);
        }
    }

    fn lifecycle_config_snapshot(&mut self) -> AppConfig {
        self.config.desired_running_profiles = self.supervisor.desired_profiles();
        self.config.lifecycle_states = self.supervisor.persisted_states();
        self.config.clone()
    }

    fn ensure_profile_ports(&mut self, profile_names: &[String]) -> bool {
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
        changed
    }

    fn next_profile_port(&self) -> Option<u16> {
        let active_profiles = self
            .cached_profiles
            .iter()
            .map(|profile| profile.name.clone())
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
    let (tools, should_refresh) = {
        let mut runtime = lock(&shared)?;
        let should_refresh = !runtime.discovery_in_flight;
        if should_refresh {
            runtime.discovery_in_flight = true;
        }
        (runtime.tools(), should_refresh)
    };
    if should_refresh {
        let discovery_tools = tools.clone();
        let refresh_shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let profiles = discovery_tools.discover_profiles();
                let system_images = discovery_tools
                    .installed_system_images()
                    .into_iter()
                    .map(|image| ImageDto {
                        label: image.label(),
                        package_id: image.package_id,
                    })
                    .collect();
                (profiles, system_images)
            })
            .await;
            if let Ok(mut runtime) = refresh_shared.lock() {
                runtime.discovery_in_flight = false;
                match result {
                    Ok((profiles, system_images)) => {
                        let names = profiles
                            .iter()
                            .map(|profile| profile.name.clone())
                            .collect::<Vec<_>>();
                        let changed = runtime.ensure_profile_ports(&names);
                        runtime.cached_profiles = profiles;
                        runtime.cached_system_images = system_images;
                        if changed {
                            let config = runtime.config.clone();
                            drop(runtime);
                            let _ = config.save();
                        }
                    }
                    Err(error) => runtime.log(
                        "warning",
                        format!("Device discovery worker failed: {error}"),
                    ),
                }
            }
        });
    }
    let mut runtime = lock(&shared)?;
    let profiles = runtime.cached_profiles.clone();
    let system_images = runtime.cached_system_images.clone();
    let running_names = profiles
        .iter()
        .filter(|profile| profile.is_running())
        .map(|profile| profile.name.clone())
        .collect::<BTreeSet<_>>();
    runtime
        .rectangular_watchers
        .retain(|name| running_names.contains(name));
    let mut rectangular_starts = Vec::new();
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
            rectangular_starts.push(profile.name.clone());
        }
    }
    let state = AppState {
        manager_id: runtime._manager_guard.manager_id().to_owned(),
        profiles,
        profile_options: runtime.config.profile_options.clone(),
        starting_profiles: runtime.starting_profiles.clone(),
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
        logs: runtime.logs.clone(),
    };
    drop(runtime);
    for profile_name in rectangular_starts {
        tools.watch_rectangular_display_after_start(&profile_name);
    }
    Ok(Json(state))
}

async fn create_profile(
    State(shared): State<Shared>,
    Json(request): Json<CreateRequest>,
) -> ApiResult<ApiMessage> {
    let tools = lock(&shared)?.tools();
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
    let mut runtime = lock(&shared)?;
    let options = ProfileOptions {
        adb_port: runtime.next_profile_port(),
        ..ProfileOptions::default()
    };
    runtime
        .config
        .profile_options
        .insert(request.name.trim().to_owned(), options);
    runtime.log("success", format!("Created {}", request.name.trim()));
    let config = runtime.config.clone();
    drop(runtime);
    config.save().map_err(|error| {
        error_tuple(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save the profile: {error}"),
        )
    })?;
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
    let (options, startup_generation, config_to_save) = {
        let mut runtime = lock(shared)?;
        if !user_requested && !runtime.supervisor.is_desired(name) {
            return Ok(message("Recovery was cancelled by an intentional stop"));
        }
        if runtime.starting_profiles.contains(name) {
            return Ok(message("Android is already starting"));
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
    let mut launch_options = options.clone();
    if needs_rectangular_hardware_migration(&profile.device_name, options.rectangular_display) {
        launch_options.cold_boot = true;
    }
    let start_result = tools
        .configure_input_and_window(
            name,
            options.host_keyboard,
            options.device_frame,
            options.rectangular_display,
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
        .and_then(|_| tools.start(&profile, &launch_args(&launch_options), &options));
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
    tools.watch_profile_after_start(
        name,
        options.speed == SpeedPreset::LeanGaming,
        options.suspend_store_during_automation,
    );
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
        let _ = scope_unit;
    });
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
    let tools = runtime.tools();
    let prior_status = runtime.supervisor.status_snapshot(name).unwrap_or_default();
    runtime.supervisor.request_stop(name);
    let generation = runtime.supervisor.generation(name).unwrap_or_default();
    let scope = prior_status
        .scope
        .clone()
        .or_else(|| runtime.profile_scopes.get(name).cloned());
    let pid = prior_status.pid;
    let graceful = graceful_stop_allowed(prior_status.phase);
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
        let cleanup_current = || {
            shared.lock().is_ok_and(|runtime| {
                runtime
                    .supervisor
                    .cleanup_is_current(&name, generation, scope.as_deref(), pid)
            })
        };
        if !cleanup_current() {
            return;
        }
        let cleanup_result = (|| {
            if let Some(scope) = &scope {
                tools.terminate_scope_and_wait(scope, graceful)?;
            }
            if let Some(pid) = pid {
                tools.wait_pid_gone(pid)?;
            }
            Ok::<(), String>(())
        })();
        if let Ok(mut runtime) = shared.lock() {
            let changed = match cleanup_result {
                Ok(()) => {
                    let cleaned = runtime.supervisor.complete_cleanup(
                        &name,
                        generation,
                        scope.as_deref(),
                        pid,
                    );
                    if cleaned {
                        runtime.profile_scopes.remove(&name);
                        runtime.log("info", format!("Stopped and cleaned up {name}"));
                    }
                    cleaned
                }
                Err(error) => {
                    let retained = runtime.supervisor.mark_cleanup_failed(
                        &name,
                        generation,
                        scope.as_deref(),
                        pid,
                        error.clone(),
                    );
                    if retained {
                        runtime.log(
                            "error",
                            format!("Could not finish stopping {name}: {error}"),
                        );
                    }
                    retained
                }
            };
            if !changed {
                return;
            }
            let config = runtime.lifecycle_config_snapshot();
            drop(runtime);
            let _ = config.save();
        }
    });
    Ok(message("Android stop requested"))
}

fn graceful_stop_allowed(phase: DevicePhase) -> bool {
    phase == DevicePhase::Ready
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
        runtime.starting_profiles.contains(&name),
        runtime.supervisor.statuses().remove(&name),
    );

    Ok(Json(readiness))
}

fn port_readiness_from_state(
    name: &str,
    starting: bool,
    status: Option<DeviceStatus>,
) -> PortReadiness {
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
    let profiles = tools.discover_profiles();
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
    runtime.log("success", format!("Cloned {source_name} as {clone_name}"));
    let config = runtime.config.clone();
    drop(runtime);
    config
        .save()
        .map_err(|error| error_tuple(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(message(format!("{clone_name} cloned successfully")))
}

fn runtime_port(shared: &Shared) -> Result<Option<u16>, (StatusCode, Json<ApiMessage>)> {
    Ok(lock(shared)?.next_profile_port())
}

async fn delete_profile(
    State(shared): State<Shared>,
    Path(name): Path<String>,
) -> ApiResult<ApiMessage> {
    let tools = lock(&shared)?.tools();
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
    let mut runtime = lock(&shared)?;
    runtime.supervisor.request_stop(&name);
    runtime.profile_scopes.remove(&name);
    runtime.config.profile_options.remove(&name);
    runtime.log("info", format!("Deleted {name}"));
    let config = runtime.lifecycle_config_snapshot();
    drop(runtime);
    config.save().map_err(|error| {
        error_tuple(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not save device lifecycle state: {error}"),
        )
    })?;
    Ok(message("Android deleted"))
}

async fn save_profile_settings(
    State(shared): State<Shared>,
    Path(name): Path<String>,
    Json(options): Json<ProfileOptions>,
) -> ApiResult<ApiMessage> {
    validate_profile_options(&options)
        .map_err(|message| error_tuple(StatusCode::BAD_REQUEST, message))?;
    if options.window_scale.is_none() {
        let tools = { lock(&shared)?.tools() };
        tools
            .forget_window_scale(&name)
            .map_err(|message| error_tuple(StatusCode::BAD_REQUEST, message))?;
    }
    let mut runtime = lock(&shared)?;
    runtime.config.profile_options.insert(name.clone(), options);
    runtime.log("success", format!("Saved settings for {name}"));
    let config = runtime.config.clone();
    drop(runtime);
    config
        .save()
        .map_err(|error| error_tuple(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(message("Profile settings saved"))
}

async fn save_settings(
    State(shared): State<Shared>,
    Json(settings): Json<SettingsDto>,
) -> ApiResult<ApiMessage> {
    let android_sdk_path = settings.android_sdk_path.trim().to_owned();
    let jdk_path = settings.jdk_path.trim().to_owned();
    let tools = AndroidTools::detect(&android_sdk_path, &jdk_path);
    let mut runtime = lock(&shared)?;
    runtime.config.android_sdk_path = android_sdk_path;
    runtime.config.jdk_path = jdk_path;
    runtime.tools = tools;
    runtime.log("success", "Tool paths saved");
    let config = runtime.config.clone();
    drop(runtime);
    config
        .save()
        .map_err(|error| error_tuple(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
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
    Ok(())
}

fn display_dimensions(_picture: PicturePreset) -> (u16, u16) {
    // Frostguard's screenshots, OCR regions and tap coordinates use this exact canvas.
    // Keep the serialized picture choice for backwards-compatible config parsing, but
    // never let an older profile launch at a different automation resolution.
    (720, 1280)
}

fn needs_rectangular_hardware_migration(device_name: &str, rectangular_display: bool) -> bool {
    rectangular_display && !device_name.eq_ignore_ascii_case("pixel 2")
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
        classify_supervisor_observation, graceful_stop_allowed, launch_args,
        needs_rectangular_hardware_migration, port_readiness_from_state,
    };
    use crate::model::{PicturePreset, ProfileOptions, SpeedPreset};
    use crate::supervisor::DevicePhase;

    #[test]
    fn snapshot_saving_stop_is_allowed_only_after_ready() {
        assert!(graceful_stop_allowed(DevicePhase::Ready));
        for phase in [
            DevicePhase::Admitting,
            DevicePhase::Launching,
            DevicePhase::Booting,
            DevicePhase::Settling,
            DevicePhase::ThermalHold,
            DevicePhase::Backoff,
        ] {
            assert!(!graceful_stop_allowed(phase), "{phase:?}");
        }
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
    fn restored_owned_scope_keeps_booting_while_adb_is_still_starting() {
        let scope = Some("emumi-Device_1-456.scope".into());
        assert_eq!(
            classify_supervisor_observation(
                false,
                false,
                false,
                Some(crate::supervisor::DevicePhase::Backoff),
                scope.clone(),
            ),
            Some(crate::supervisor::Observation::Booting { scope })
        );
    }

    #[test]
    fn readiness_uses_cached_supervision_without_waiting_for_adb() {
        let running = crate::supervisor::DeviceStatus {
            phase: crate::supervisor::DevicePhase::Ready,
            desired_running: true,
            ..Default::default()
        };
        let ready = port_readiness_from_state("Device_3", false, Some(running));
        assert!(ready.ready);
        assert_eq!(ready.phase, "ready");

        let recovering = crate::supervisor::DeviceStatus {
            phase: crate::supervisor::DevicePhase::Backoff,
            desired_running: true,
            last_failure: Some("health probe timed out".into()),
            ..Default::default()
        };
        let not_ready = port_readiness_from_state("Device_3", false, Some(recovering));
        assert!(!not_ready.ready);
        assert_eq!(not_ready.phase, "recovering");
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
        assert_eq!(options.refresh_rate_hz, 30);
        assert!(options.suspend_store_during_automation);
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
    fn rectangular_hardware_migration_gets_one_clean_boot() {
        assert!(needs_rectangular_hardware_migration("pixel 7", true));
        assert!(!needs_rectangular_hardware_migration("pixel 2", true));
        assert!(!needs_rectangular_hardware_migration("pixel 7", false));
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
}
