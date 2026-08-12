mod app;
mod model;
mod project;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Print Forge Studio")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1100.0, 700.0]),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "print-forge-studio",
        options,
        Box::new(|creation| Ok(Box::new(app::StudioApp::new(creation)))),
    )
}
