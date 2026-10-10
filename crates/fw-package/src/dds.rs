//! Album art: any common image format -> DXT1 DDS at 64, 128 and 256 px.

use image::imageops::FilterType;

use crate::Error;

pub const SIZES: [u32; 3] = [64, 128, 256];

fn header(size: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(128);
    let mut u = |v: u32| h.extend_from_slice(&v.to_le_bytes());
    u(0x2053_4444); // "DDS "
    u(124); // header size
    u(0x0008_1007); // caps | height | width | pixel format | linear size
    u(size); // height
    u(size); // width
    u(size * size / 2); // linear size: 8 bytes per 4x4 block
    u(0); // depth
    u(1); // mipmap count
    for i in 0..11 {
        u(if i == 0 { 0x5445_5246 } else if i == 1 { 0x0000_5257 } else { 0 }); // "FRETWR" tag in reserved space
    }
    u(32); // pixel format size
    u(4); // FOURCC
    u(0x3154_5844); // "DXT1"
    for _ in 0..5 {
        u(0);
    }
    u(0x1000); // texture
    for _ in 0..4 {
        u(0);
    }
    h
}

fn encode(img: &image::RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let format = texpresso::Format::Bc1;
    let mut out = vec![0u8; format.compressed_size(w as usize, h as usize)];
    let params = texpresso::Params { algorithm: texpresso::Algorithm::ClusterFit, ..Default::default() };
    format.compress(img.as_raw(), w as usize, h as usize, params, &mut out);
    out
}

/// Word-wraps `text` to lines no wider than `max` pixels at `scale`.
fn wrap(font: &impl ab_glyph::Font, scale: f32, text: &str, max: f32) -> Vec<String> {
    use ab_glyph::ScaleFont;
    let f = font.as_scaled(scale);
    let width = |s: &str| s.chars().map(|c| f.h_advance(f.glyph_id(c))).sum::<f32>();
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if width(&candidate) > max && !line.is_empty() {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// A 512 px cover for songs without art: artist and title in a green frame.
pub fn placeholder(artist: &str, title: &str) -> image::RgbaImage {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
    const SIZE: u32 = 512;
    let bg = image::Rgba([24, 26, 33, 255]);
    let mut img = image::RgbaImage::from_pixel(SIZE, SIZE, bg);
    let green = image::Rgba([88, 194, 125, 255]);
    for y in 24..=488u32 {
        for x in 24..=488u32 {
            let edge = !(30..=482).contains(&x) || !(30..=482).contains(&y);
            if edge {
                img.put_pixel(x, y, green);
            }
        }
    }
    let Ok(font) = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT) else { return img };
    let scale = PxScale::from(40.0);
    let sf = font.as_scaled(scale);
    let mut lines = wrap(&font, 40.0, artist, 420.0);
    lines.push(String::new());
    lines.extend(wrap(&font, 40.0, title, 420.0));
    lines.retain(|l| !l.is_empty() || !artist.is_empty());
    let line_h = sf.height() + sf.line_gap();
    let mut y = SIZE as f32 / 2.0 - line_h * lines.len() as f32 / 2.0 + sf.ascent();
    for line in &lines {
        let w: f32 = line.chars().map(|c| sf.h_advance(sf.glyph_id(c))).sum();
        let mut x = (SIZE as f32 - w) / 2.0;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            let glyph = id.with_scale_and_position(scale, point(x, y));
            x += sf.h_advance(id);
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, cov| {
                    let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if (0..SIZE as i32).contains(&px) && (0..SIZE as i32).contains(&py) {
                        let p = img.get_pixel_mut(px as u32, py as u32);
                        for k in 0..3 {
                            p.0[k] = (p.0[k] as f32 + (230.0 - p.0[k] as f32) * cov.min(1.0)) as u8;
                        }
                    }
                });
            }
        }
        y += line_h;
    }
    img
}

/// The three album art sizes as DDS files, from an image file, or a
/// placeholder showing the artist and title when there is none.
pub fn album_art(path: Option<&std::path::Path>, artist: &str, title: &str) -> Result<Vec<(u32, Vec<u8>)>, Error> {
    let source = match path {
        Some(p) => image::open(p).map_err(|e| Error::Image(format!("{}: {e}", p.display())))?.to_rgba8(),
        None => placeholder(artist, title),
    };
    let out = SIZES
        .iter()
        .map(|&size| {
            let img = image::imageops::resize(&source, size, size, FilterType::Lanczos3);
            let mut dds = header(size);
            dds.extend(encode(&img));
            (size, dds)
        })
        .collect();
    Ok(out)
}

#[cfg(test)]
mod placeholder_tests {
    #[test]
    fn placeholder_draws_text_inside_the_frame() {
        let img = super::placeholder("Fluid Love", "Dirty Water");
        assert_eq!(img.dimensions(), (512, 512));
        let bright = img.pixels().filter(|p| p.0[0] > 150 && p.0[1] > 150 && p.0[2] > 150).count();
        assert!(bright > 500, "text pixels: {bright}");
        assert_eq!(img.get_pixel(26, 256).0, [88, 194, 125, 255]);
    }
}
