use eframe::egui::{self, TextureHandle, TextureOptions};

const LOGO_PNG: &[u8] = include_bytes!("../../../assets/brand/print-forge-logo.png");

pub fn app_icon() -> egui::IconData {
    let image = logo_image();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

pub fn logo_texture(ctx: &egui::Context) -> TextureHandle {
    let image = logo_image();
    let size = [image.width() as usize, image.height() as usize];
    ctx.load_texture(
        "print-forge-logo",
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        TextureOptions::LINEAR,
    )
}

fn logo_image() -> image::RgbaImage {
    image::load_from_memory(LOGO_PNG)
        .expect("embedded Print Forge logo should be a valid image")
        .into_rgba8()
}

#[cfg(test)]
mod tests {
    #[test]
    fn logo_is_square_rgba_with_transparency() {
        let icon = super::app_icon();

        assert_eq!(icon.width, 1024);
        assert_eq!(icon.height, 1024);
        assert_eq!(icon.rgba.len(), 1024 * 1024 * 4);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] == 0));
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] == 255));
    }
}
