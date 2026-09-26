mod android;
mod app;
mod config;
mod model;
mod monitor;

use app::EmuMiApp;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 620.0]),
        ..Default::default()
    };

    eframe::run_native(
        "EmuMi",
        options,
        Box::new(|cc| Ok(Box::new(EmuMiApp::new(cc)))),
    )
}
