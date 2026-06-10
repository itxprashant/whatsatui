use image::{DynamicImage, GenericImageView, Rgb};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

// Unicode braille dot bit masks (2 wide × 4 tall per character).
const DOT: [u8; 8] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];

/// Inline bubble preview size (terminal cells).
pub const INLINE_PREVIEW_COLS: u16 = 22;
pub const INLINE_PREVIEW_ROWS: u16 = 5;

/// Render an image with braille characters (2×4 pixels per cell — much sharper than half-blocks).
pub fn render_image(img: &DynamicImage, max_cols: u16, max_rows: u16) -> Vec<Line<'static>> {
    if max_cols == 0 || max_rows == 0 {
        return Vec::new();
    }

    let (iw, ih) = img.dimensions();
    if iw == 0 || ih == 0 {
        return Vec::new();
    }

    // Braille gives 2 horizontal and 4 vertical pixels per terminal cell.
    let max_pw = (max_cols as u32) * 2;
    let max_ph = (max_rows as u32) * 4;
    let scale = (max_pw as f32 / iw as f32).min(max_ph as f32 / ih as f32);
    let mut rw = (iw as f32 * scale).round().max(2.0) as u32;
    let mut rh = (ih as f32 * scale).round().max(4.0) as u32;
    rw &= !1;
    rh = (rh / 4) * 4;

    let resized = img.resize_exact(rw, rh, image::imageops::FilterType::Lanczos3);
    let rgb = resized.to_rgb8();

    let term_w = (rw / 2) as u16;
    let term_h = (rh / 4) as u16;
    let pad_left = max_cols.saturating_sub(term_w) / 2;
    let pad_top = max_rows.saturating_sub(term_h) / 2;

    let mut lines = Vec::with_capacity(pad_top as usize + term_h as usize);
    for _ in 0..pad_top {
        lines.push(Line::from(""));
    }

    for y in (0..rh).step_by(4) {
        let mut spans = Vec::with_capacity(pad_left as usize + term_w as usize);
        if pad_left > 0 {
            spans.push(Span::raw(" ".repeat(pad_left as usize)));
        }
        for x in (0..rw).step_by(2) {
            let (ch, color) = braille_cell(&rgb, x, y, rw, rh);
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(Color::Rgb(color[0], color[1], color[2])),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn braille_cell(rgb: &image::RgbImage, x: u32, y: u32, w: u32, h: u32) -> (char, Rgb<u8>) {
    let mut pixels = [[0u8; 3]; 8];
    let coords: [(u32, u32); 8] = [
        (x, y),
        (x, y + 1),
        (x, y + 2),
        (x + 1, y),
        (x + 1, y + 1),
        (x + 1, y + 2),
        (x, y + 3),
        (x + 1, y + 3),
    ];
    let mut lums = [0f32; 8];
    let mut sum = [0u32; 3];
    for (i, &(px, py)) in coords.iter().enumerate() {
        let p = sample(rgb, px, py, w, h);
        pixels[i] = [p[0], p[1], p[2]];
        lums[i] = luminance(p);
        sum[0] += p[0] as u32;
        sum[1] += p[1] as u32;
        sum[2] += p[2] as u32;
    }
    let mean_lum = lums.iter().sum::<f32>() / 8.0;
    let mut bits = 0u8;
    for (i, &lum) in lums.iter().enumerate() {
        if lum > mean_lum * 0.92 {
            bits |= DOT[i];
        }
    }
    // Ensure very dark cells still show structure.
    if bits == 0 {
        let darkest = lums
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        bits = DOT[darkest];
    }
    let avg = Rgb([
        (sum[0] / 8) as u8,
        (sum[1] / 8) as u8,
        (sum[2] / 8) as u8,
    ]);
    let ch = char::from_u32(0x2800 + bits as u32).unwrap_or(' ');
    (ch, avg)
}

fn sample(rgb: &image::RgbImage, x: u32, y: u32, w: u32, h: u32) -> Rgb<u8> {
    *rgb.get_pixel(x.min(w.saturating_sub(1)), y.min(h.saturating_sub(1)))
}

fn luminance(p: Rgb<u8>) -> f32 {
    0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
}

/// Downscale for inline chat previews (saves memory in the thumbnail cache).
pub fn thumbnail_image(img: &DynamicImage) -> DynamicImage {
    let max_pw = (INLINE_PREVIEW_COLS as u32) * 2;
    let max_ph = (INLINE_PREVIEW_ROWS as u32) * 4;
    let (iw, ih) = img.dimensions();
    let scale = (max_pw as f32 / iw as f32).min(max_ph as f32 / ih as f32).min(1.0);
    let rw = ((iw as f32 * scale).round().max(2.0) as u32) & !1;
    let rh = (((ih as f32 * scale).round().max(4.0) as u32) / 4) * 4;
    DynamicImage::ImageRgb8(img.resize_exact(rw, rh, image::imageops::FilterType::Triangle).to_rgb8())
}

/// Decode image bytes (JPEG/PNG/WebP) for terminal rendering.
pub fn decode_image(bytes: &[u8]) -> Result<DynamicImage, image::ImageError> {
    image::load_from_memory(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    #[test]
    fn braille_output_fits_terminal_bounds() {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(100, 200));
        let lines = render_image(&img, 40, 20);
        assert!(lines.len() <= 20);
        if let Some(line) = lines.last() {
            assert!(line.width() <= 40);
        }
    }
}
