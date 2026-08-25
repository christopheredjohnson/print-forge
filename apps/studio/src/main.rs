mod app;
mod model;
mod project;
mod system_fonts;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Print Forge Studio")
            .with_icon(app_icon())
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

fn app_icon() -> eframe::egui::IconData {
    let image =
        image::load_from_memory(include_bytes!("../../../assets/brand/print-forge-logo.png"))
            .expect("embedded Print Forge app icon should be a valid image")
            .into_rgba8();
    eframe::egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_icon_is_square_rgba_with_transparency() {
        let icon = super::app_icon();

        assert_eq!(icon.width, 1024);
        assert_eq!(icon.height, 1024);
        assert_eq!(icon.rgba.len(), 1024 * 1024 * 4);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] == 0));
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] == 255));
    }
}
