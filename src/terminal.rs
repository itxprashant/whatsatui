//! Terminal graphics capability probing.
//!
//! `ratatui-image` blacklists Sixel and Kitty on Konsole (buggy Kitty placeholder support),
//! which leaves only `Halfblocks`. Konsole does support Sixel when enabled in settings, so we
//! re-probe for Sixel after the library picker runs.

use std::env;
use std::io::{self, Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ratatui_image::picker::{Picker, ProtocolType};

const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Terminal graphics probe result: picker, fullscreen viewer, and inline bubble previews.
pub struct TerminalImages {
    pub picker: Option<Picker>,
    /// Full-screen image viewer (`v`).
    pub native_images: bool,
    /// Inline previews inside scrollable message bubbles.
    pub native_inline_images: bool,
}

/// Detect image picker and native rendering capabilities.
pub fn probe_terminal_picker() -> TerminalImages {
    let mut picker = Picker::from_query_stdio().ok();
    let Some(mut picker) = picker.take() else {
        return TerminalImages {
            picker: None,
            native_images: false,
            native_inline_images: false,
        };
    };

    picker = apply_konsole_sixel_fixup(picker);
    picker = apply_protocol_override(picker);

    let native_images = !matches!(picker.protocol_type(), ProtocolType::Halfblocks);
    let native_inline_images = native_images && inline_images_enabled();
    TerminalImages {
        picker: Some(picker),
        native_images,
        native_inline_images,
    }
}

/// Inline previews on by default; set `WHATSATUI_INLINE_IMAGES=0` to disable (e.g. Konsole scroll ghosts).
fn inline_images_enabled() -> bool {
    match env::var("WHATSATUI_INLINE_IMAGES").ok() {
        Some(v) => matches!(v.trim(), "1" | "true" | "yes"),
        None => true,
    }
}

/// True when running inside KDE Konsole (`KONSOLE_VERSION` is set).
pub fn is_konsole() -> bool {
    env::var("KONSOLE_VERSION").is_ok_and(|s| !s.is_empty())
}

/// User-facing hint when native images are unavailable.
pub fn unsupported_images_hint() -> &'static str {
    if is_konsole() {
        "Enable Sixel in Konsole: Settings → General → Use Sixel graphics"
    } else {
        "Terminal does not support inline images"
    }
}

fn apply_konsole_sixel_fixup(mut picker: Picker) -> Picker {
    if !is_konsole() || !matches!(picker.protocol_type(), ProtocolType::Halfblocks) {
        return picker;
    }
    if query_sixel_support() {
        picker.set_protocol_type(ProtocolType::Sixel);
    }
    picker
}

fn apply_protocol_override(mut picker: Picker) -> Picker {
    let Some(raw) = env::var("WHATSATUI_IMAGE_PROTOCOL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    else {
        return picker;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "sixel" => picker.set_protocol_type(ProtocolType::Sixel),
        "kitty" => picker.set_protocol_type(ProtocolType::Kitty),
        "iterm2" | "iterm" => picker.set_protocol_type(ProtocolType::Iterm2),
        _ => {}
    }
    picker
}

fn query_sixel_support() -> bool {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(query_sixel_support_inner());
    });
    rx.recv_timeout(PROBE_TIMEOUT).unwrap_or(false)
}

fn query_sixel_support_inner() -> bool {
    // Primary device attributes (`ESC [ c`) + DSR (`ESC [ 5 n`) so we know when to stop reading.
    if io::stdout().write_all(b"\x1b[c\x1b[5n").is_err() {
        return false;
    }
    if io::stdout().flush().is_err() {
        return false;
    }

    let mut data = String::new();
    let mut buf = [0u8; 32];
    loop {
        let Ok(n) = io::stdin().read(&mut buf) else {
            break;
        };
        if n == 0 {
            continue;
        }
        data.push_str(&String::from_utf8_lossy(&buf[..n]));
        if data.ends_with("[0n") || data.contains("[0n") {
            break;
        }
        if data.len() > 512 {
            break;
        }
    }
    sixel_in_da_response(&data)
}

/// Parse a primary device-attributes response for Sixel capability (DA cap `4`).
fn sixel_in_da_response(data: &str) -> bool {
    let Some(idx) = data.find("\x1b[?") else {
        return false;
    };
    let rest = &data[idx + 3..];
    let Some(end) = rest.find('c') else {
        return false;
    };
    rest[..end].split(';').any(|cap| cap == "4")
}

#[cfg(test)]
mod tests {
    use super::sixel_in_da_response;

    #[test]
    fn detects_sixel_capability_in_da_response() {
        assert!(sixel_in_da_response("\x1b[?1;2;4c\x1b[0n"));
        assert!(!sixel_in_da_response("\x1b[?1;2c\x1b[0n"));
    }
}
