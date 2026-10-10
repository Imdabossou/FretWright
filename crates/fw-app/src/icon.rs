//! The app icon, drawn in code so every size (window, .desktop, .ico, .icns)
//! comes from one source: a dark rounded tile with the four bass strings in
//! Rocksmith's colours and a few notes on them.

use crate::editor::STRING_COLORS;

fn rounded_rect_cov(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r && x >= x0 && x <= x1 && y >= y0 && y <= y1
}

/// RGBA pixels of the icon at `size` x `size`.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4; // supersampling per axis
    let s = size as f32;
    let notes = [(0.30, 0usize), (0.47, 1), (0.64, 2), (0.80, 3), (0.55, 0)];
    let mut out = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / s;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / s;
                    let mut c: Option<[f32; 3]> = None;
                    if rounded_rect_cov(x, y, 0.03, 0.03, 0.97, 0.97, 0.18) {
                        c = Some([27.0, 29.0, 35.0]);
                        // green frame
                        if !rounded_rect_cov(x, y, 0.07, 0.07, 0.93, 0.93, 0.14) {
                            c = Some([63.0, 143.0, 99.0]);
                        }
                        // strings, G on top
                        for (k, col) in STRING_COLORS.iter().enumerate() {
                            let sy_ = 0.27 + (3 - k) as f32 * 0.155;
                            if (y - sy_).abs() < 0.012 && x > 0.12 && x < 0.88 {
                                c = Some([col.r() as f32 * 0.6, col.g() as f32 * 0.6, col.b() as f32 * 0.6]);
                            }
                        }
                        for (nx, k) in notes {
                            let ny = 0.27 + (3 - k) as f32 * 0.155;
                            if rounded_rect_cov(x, y, nx - 0.055, ny - 0.05, nx + 0.055, ny + 0.05, 0.025) {
                                let col = STRING_COLORS[k];
                                c = Some([col.r() as f32, col.g() as f32, col.b() as f32]);
                            }
                        }
                    }
                    if let Some(c) = c {
                        acc[0] += c[0];
                        acc[1] += c[1];
                        acc[2] += c[2];
                        acc[3] += 255.0;
                    }
                }
            }
            let n = (SS * SS) as f32;
            let i = ((py * size + px) * 4) as usize;
            let a = acc[3] / n;
            // un-premultiply the averaged colour
            for k in 0..3 {
                out[i + k] = if a > 0.0 { (acc[k] / n * 255.0 / a).round() as u8 } else { 0 };
            }
            out[i + 3] = a.round() as u8;
        }
    }
    out
}

pub fn icon_data(size: u32) -> eframe::egui::IconData {
    eframe::egui::IconData { rgba: rgba(size), width: size, height: size }
}

/// Writes the icon for packaging (`fretwright --write-icon <file> [size]`):
/// a PNG at `size`, or a Windows .ico with the standard sizes.
pub fn write(path: &std::path::Path, size: u32) -> Result<(), String> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ico")) {
        let frames = [16u32, 24, 32, 48, 64, 128, 256]
            .iter()
            .map(|&s| image::codecs::ico::IcoFrame::as_png(&rgba(s), s, s, image::ExtendedColorType::Rgba8))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
        return image::codecs::ico::IcoEncoder::new(file).encode_images(&frames).map_err(|e| e.to_string());
    }
    image::RgbaImage::from_raw(size, size, rgba(size)).ok_or("bad icon size")?.save(path).map_err(|e| e.to_string())
}
