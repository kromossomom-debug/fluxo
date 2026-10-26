#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod engine;
mod store;
mod theme;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Fluxo — Downloader de vídeos")
            .with_app_id("fluxo")
            .with_inner_size([1240.0, 800.0])
            .with_min_inner_size([880.0, 560.0])
            .with_icon(std::sync::Arc::new(theme::app_icon(128))),
        ..Default::default()
    };
    eframe::run_native(
        "Fluxo",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(app::FluxoApp::new(&cc.egui_ctx)))
        }),
    )
}
