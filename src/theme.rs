use ratatui::style::{Color, Modifier, Style};

// WhatsApp-inspired palette on a dark neutral base.
pub const ACCENT: Color = Color::Rgb(0, 168, 132);
pub const ACCENT_BRIGHT: Color = Color::Rgb(37, 211, 102);
pub const BG: Color = Color::Rgb(11, 20, 26);
pub const PANEL: Color = Color::Rgb(23, 33, 39);
pub const SEL_BG: Color = Color::Rgb(0, 92, 75);
pub const BUBBLE_IN: Color = Color::Rgb(33, 44, 51);
pub const BUBBLE_OUT: Color = Color::Rgb(0, 92, 75);
pub const TEXT: Color = Color::Rgb(233, 237, 239);
pub const DIM: Color = Color::Rgb(140, 154, 163);

const BADGE_COLORS: [Color; 6] = [
    ACCENT,
    Color::Rgb(83, 189, 235),
    Color::Rgb(247, 178, 103),
    Color::Rgb(229, 115, 115),
    Color::Rgb(149, 117, 205),
    Color::Rgb(77, 182, 172),
];

/// Deterministically pick an avatar-badge color from a key (the chat jid).
pub fn badge_color(key: &str) -> Color {
    let h = key.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    BADGE_COLORS[(h as usize) % BADGE_COLORS.len()]
}

pub fn dim() -> Style {
    Style::default().fg(DIM)
}

pub fn text() -> Style {
    Style::default().fg(TEXT)
}

pub fn bold_text() -> Style {
    Style::default().fg(TEXT).add_modifier(Modifier::BOLD)
}

pub fn screen_bg() -> Style {
    Style::default().bg(BG)
}

pub fn panel_bg() -> Style {
    Style::default().bg(PANEL)
}
