mod app;
mod shell_session;
mod terminal_buffer;
mod window_manager;

use app::MultiCliApp;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Multi-CLI")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Multi-CLI",
        native_options,
        Box::new(|cc| Box::new(MultiCliApp::new(cc))),
    )
}
