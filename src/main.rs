#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod engine;
mod i18n;
mod store;
mod theme;

fn main() -> eframe::Result {
    i18n::set(store::load_settings().language.unwrap_or_else(i18n::system));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(i18n::tr().window_title)
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
