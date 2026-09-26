use crate::{
    android::AndroidTools,
    config::AppConfig,
    model::{PicturePreset, ProfileOptions},
    monitor::HostMonitor,
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
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");

type Shared = Arc<Mutex<Runtime>>;
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiMessage>)>;

pub struct EmuMiApp;

struct Runtime {
    config: AppConfig,
    logs: Vec<ApiLog>,
    monitor: HostMonitor,
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
    system_images: Vec<ImageDto>,
    config: SettingsDto,
    health: HealthDto,
    stats: crate::model::HostStats,
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
}

#[derive(Serialize)]
struct ApiMessage {
    message: String,
}

#[derive(Deserialize)]
struct CreateRequest {
    name: String,
    device_id: String,
    package_id: String,
}

impl EmuMiApp {
    pub async fn start_server() -> Result<String, Box<dyn std::error::Error>> {
        let config = AppConfig::load();
        let runtime = Arc::new(Mutex::new(Runtime {
            config,
            logs: vec![ApiLog::new("success", "EmuMi is ready")],
            monitor: HostMonitor::default(),
        }));

        let app = Router::new()
            .route("/", get(index))
            .route("/app.js", get(script))
            .route("/api/state", get(read_state))
            .route("/api/profiles", post(create_profile))
            .route("/api/profiles/{name}", delete(delete_profile))
            .route("/api/profiles/{name}/start", post(start_profile))
            .route("/api/profiles/{name}/stop", post(stop_profile))
            .route("/api/profiles/{name}/settings", post(save_profile_settings))
            .route("/api/settings", post(save_settings))
            .with_state(runtime);

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let url = format!("http://{address}");
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
    let system_images = tools
        .installed_system_images()
        .into_iter()
        .map(|image| ImageDto {
            label: image.label(),
            package_id: image.package_id,
        })
        .collect();
    let state = AppState {
        profiles,
        profile_options: runtime.config.profile_options.clone(),
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
        },
        stats: runtime.monitor.sample(),
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
    runtime.log("success", format!("Created {}", request.name.trim()));
    Ok(message("Android created"))
}

async fn start_profile(
    State(shared): State<Shared>,
    Path(name): Path<String>,
) -> ApiResult<ApiMessage> {
    let mut runtime = lock(&shared)?;
    let tools = runtime.tools();
    let profile = tools
        .discover_profiles()
        .into_iter()
        .find(|profile| profile.name == name)
        .ok_or_else(|| error_tuple(StatusCode::NOT_FOUND, "Profile not found"))?;
    if profile.is_running() {
        return Ok(message("Android is already running"));
    }
    let options = runtime
        .config
        .profile_options
        .entry(name.clone())
        .or_default()
        .clone();
    tools
        .configure_input_and_window(&name, options.host_keyboard, options.device_frame)
        .and_then(|_| {
            if let Some(scale) = options.window_scale {
                tools.configure_window_scale(&name, scale)
            } else {
                Ok(())
            }
        })
        .and_then(|_| tools.start(&profile, &launch_args(&options)))
        .map_err(|message| error_tuple(StatusCode::BAD_REQUEST, message))?;
    runtime.log("success", format!("Starting {name}"));
    Ok(message("Android is starting"))
}

async fn stop_profile(
    State(shared): State<Shared>,
    Path(name): Path<String>,
) -> ApiResult<ApiMessage> {
    let mut runtime = lock(&shared)?;
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
    let mut args = vec![
        "-cores".into(),
        options.cores.to_string(),
        "-memory".into(),
        options.memory_mb.to_string(),
        "-gpu".into(),
        options.gpu_mode.clone(),
        "-skin".into(),
        match options.picture {
            PicturePreset::Compact => "540x960",
            PicturePreset::Phone => "720x1280",
            PicturePreset::Sharp => "1080x1920",
        }
        .into(),
        "-dpi-device".into(),
        options.dpi.to_string(),
    ];
    if let Some(port) = options.adb_port {
        args.extend(["-port".into(), port.to_string()]);
    }
    if options.cold_boot {
        args.push("-no-snapshot-load".into());
    }
    if options.mute_audio {
        args.push("-no-audio".into());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::launch_args;
    use crate::model::ProfileOptions;

    #[test]
    fn disables_emulator_audio_when_profile_is_muted() {
        let options = ProfileOptions {
            mute_audio: true,
            ..ProfileOptions::default()
        };

        assert!(launch_args(&options).contains(&"-no-audio".to_owned()));
    }

    #[test]
    fn uses_frostguard_friendly_resolution_by_default() {
        let args = launch_args(&ProfileOptions::default());
        let skin = args.iter().position(|arg| arg == "-skin").unwrap();
        let dpi = args.iter().position(|arg| arg == "-dpi-device").unwrap();

        assert_eq!(args.get(skin + 1).map(String::as_str), Some("720x1280"));
        assert_eq!(args.get(dpi + 1).map(String::as_str), Some("240"));
    }
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
