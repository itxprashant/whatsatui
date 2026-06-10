use image::DynamicImage;

/// Inline bubble preview size (terminal cells).
pub const INLINE_PREVIEW_COLS: u16 = 22;
pub const INLINE_PREVIEW_ROWS: u16 = 5;

/// Decode image bytes (JPEG/PNG/WebP) for terminal rendering.
pub fn decode_image(bytes: &[u8]) -> Result<DynamicImage, image::ImageError> {
    image::load_from_memory(bytes)
}
