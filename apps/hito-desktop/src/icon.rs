//! The app icon: Henry Gantt, who drew the first progress charts, from a public-domain
//! photograph (Wikimedia Commons), in black and white on Hito's green. The same PNG is the window icon and
//! the logo drawn in the app.

use egui::{ColorImage, TextureHandle, TextureOptions};

/// 256 px, the size the window and the in-app logo use.
const PNG: &[u8] =
    include_bytes!("../../../assets/icons/hicolor/256x256/apps/dev.danielmiguelt.Hito.png");

/// Who the portrait is and where it comes from, for About.
pub const CREDIT: &str = "Icon: Henry L. Gantt, from a photograph of the 1910s (public domain).";

/// Decode the icon into RGBA pixels: `(width, height, rgba)`.
pub fn rgba() -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(PNG));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[g, a]| [*g, *g, *g, *a])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((info.width, info.height, rgba))
}

pub fn window_icon() -> Option<egui::IconData> {
    let (width, height, rgba) = rgba()?;
    Some(egui::IconData {
        rgba,
        width,
        height,
    })
}

/// The icon as a texture, decoded once per context.
pub fn texture(ctx: &egui::Context) -> TextureHandle {
    let id = egui::Id::unique("hito-icon-texture");
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return texture;
    }
    let image = match rgba() {
        Some((w, h, rgba)) => ColorImage::from_rgba_unmultiplied(
            [
                usize::try_from(w).unwrap_or(0),
                usize::try_from(h).unwrap_or(0),
            ],
            &rgba,
        ),
        None => ColorImage::new([1, 1], vec![egui::Color32::from_rgb(0x0F, 0x7C, 0x80)]),
    };
    let texture = ctx.load_texture("hito-icon", image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    texture
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_embedded_icon_decodes() {
        let (w, h, rgba) = super::rgba().unwrap();
        assert_eq!((w, h), (256, 256));
        assert_eq!(rgba.len(), 256 * 256 * 4);
    }
}
