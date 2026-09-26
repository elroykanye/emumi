mod android;
mod app;
mod config;
mod model;
mod monitor;

use app::EmuMiApp;
use tauri::{WebviewUrl, WebviewWindowBuilder};

#[tokio::main]
async fn main() {
    let url = match EmuMiApp::start_server().await {
        Ok(url) => url,
        Err(error) => {
            eprintln!("EmuMi could not start: {error}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(url.parse().expect("valid local EmuMi URL")),
            )
            .title("EmuMi")
            .inner_size(1280.0, 820.0)
            .min_inner_size(900.0, 620.0)
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running EmuMi");
}
