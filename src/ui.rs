use chrono::{DateTime, Local};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::api::{Chat, Device, Message};
use crate::app::{App, Focus};
use crate::termimg;
use crate::theme;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Block::default().style(theme::screen_bg()), area);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    draw_header(f, app, rows[0]);
    draw_body(f, app, rows[1]);
    draw_footer(f, app, rows[2]);

    if app.focus == Focus::ImageViewer {
        draw_image_overlay(f, app, area);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::horizontal([Constraint::Min(0), Constraint::Length(20)]).split(area);

    let mut left = vec![
        Span::styled(
            " whatsatui ",
            Style::default()
                .fg(theme::BG)
                .bg(theme::ACCENT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    match &app.device {
        Some(d) => {
            left.push(Span::styled(format!("{} ", device_name(d)), theme::bold_text()));
            left.push(Span::styled(format!("· {}", short_jid(&d.jid)), theme::dim()));
        }
        None => left.push(Span::styled("connecting…", theme::dim())),
    }
    f.render_widget(
        Paragraph::new(Line::from(left)).style(theme::panel_bg()),
        cols[0],
    );

    let right = if app.is_loading() {
        Line::from(vec![
            Span::styled(app.spinner_frame(), Style::default().fg(theme::ACCENT_BRIGHT)),
            Span::styled(" syncing ", theme::dim()),
        ])
    } else {
        let online = app
            .device
            .as_ref()
            .map(|d| d.state == "logged_in")
            .unwrap_or(false);
        if online {
            Line::from(vec![
                Span::styled("● ", Style::default().fg(theme::ACCENT_BRIGHT)),
                Span::styled("online ", theme::dim()),
            ])
        } else {
            Line::from(Span::styled("offline ", theme::dim()))
        }
    };
    f.render_widget(
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .style(theme::panel_bg()),
        cols[1],
    );
}

fn draw_body(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(32), Constraint::Min(0)]).split(area);
    draw_chats(f, app, cols[0]);

    let right = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(cols[1]);
    draw_messages(f, app, right[0]);
    draw_compose(f, app, right[1]);
}

fn chats_title(app: &App) -> String {
    if app.focus == Focus::Search {
        if app.chat_filter.is_empty() {
            " Search… ".to_string()
        } else {
            format!(" /{} ", app.chat_filter)
        }
    } else if app.show_archived {
        " Chats (archived) ".to_string()
    } else {
        " Chats ".to_string()
    }
}

fn draw_chats(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Chats || app.focus == Focus::Search;
    let block = panel_block(&chats_title(app), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible = app.visible_chat_indices();
    if visible.is_empty() {
        let msg = if app.loading_chats {
            format!("{} loading chats…", app.spinner_frame())
        } else if !app.chat_filter.is_empty() {
            "no matches".to_string()
        } else if app.show_archived {
            "no archived chats".to_string()
        } else {
            "no chats".to_string()
        };
        f.render_widget(centered_hint(&msg), inner);
        return;
    }

    let content_w = inner.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = visible
        .iter()
        .filter_map(|&idx| app.chats.get(idx))
        .map(|c| chat_item(app, c, content_w))
        .collect();

    let list = List::new(items)
        .highlight_symbol("▎ ")
        .highlight_style(
            Style::default()
                .bg(theme::SEL_BG)
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );

    let mut state = ListState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(list, inner, &mut state);
}

fn chat_item(app: &App, chat: &Chat, content_w: usize) -> ListItem<'static> {
    let name = app.chat_display_name(chat);
    let time = chat
        .last_message_time
        .as_deref()
        .map(relative_time)
        .unwrap_or_default();
    let time_len = time.chars().count();

    let name_w = content_w.saturating_sub(5 + time_len);
    let name_disp = pad_truncate(&name, name_w);

    let badge = Span::styled(
        format!(" {} ", initial(&name)),
        Style::default()
            .bg(theme::badge_color(&chat.jid))
            .fg(theme::BG)
            .add_modifier(Modifier::BOLD),
    );

    let row1 = Line::from(vec![
        badge,
        Span::raw(" "),
        Span::styled(name_disp, theme::bold_text()),
        Span::raw(" "),
        Span::styled(time, theme::dim()),
    ]);

    let mut lines = vec![row1];
    if let Some(preview) = app.chat_previews.get(&chat.jid) {
        let prefix = if preview.starts_with('[') { "" } else { " " };
        let preview_disp = pad_truncate(preview, content_w.saturating_sub(2));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(format!("{prefix}{preview_disp}"), theme::dim()),
        ]));
    }

    ListItem::new(lines)
}

fn draw_messages(f: &mut Frame, app: &App, area: Rect) {
    let title = match app.selected_chat() {
        Some(c) => format!(" {} ", app.chat_display_name(c)),
        None => " Messages ".to_string(),
    };
    let block = panel_block(&title, false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.current_jid.is_none() {
        f.render_widget(
            centered_hint("Select a chat and press Enter to open it"),
            inner,
        );
        return;
    }
    if app.loading_messages {
        f.render_widget(
            centered_hint(&format!("{} loading messages…", app.spinner_frame())),
            inner,
        );
        return;
    }
    if app.messages.is_empty() {
        f.render_widget(centered_hint("No messages yet"), inner);
        return;
    }

    let lines = build_bubbles(app, &app.messages, inner.width as usize, app.media_pick);
    let total = lines.len() as u16;
    let max_scroll = total.saturating_sub(inner.height);
    let scroll = max_scroll.saturating_sub(app.msg_scroll_from_bottom.min(max_scroll));

    let mut content = lines;
    if app.loading_older_messages && scroll == 0 {
        content.insert(
            0,
            Line::from(Span::styled(
                format!("{} loading older messages…", app.spinner_frame()),
                theme::dim(),
            )),
        );
        content.insert(1, Line::raw(""));
    }
    let paragraph = Paragraph::new(Text::from(content)).scroll((scroll, 0));
    f.render_widget(paragraph, inner);
}

fn build_bubbles(
    app: &App,
    messages: &[Message],
    inner_w: usize,
    media_pick: Option<usize>,
) -> Vec<Line<'static>> {
    let max_bubble = (inner_w * 7 / 10).max(12);
    let mut out: Vec<Line> = Vec::new();

    for (idx, m) in messages.iter().enumerate() {
        let picked = media_pick == Some(idx) && m.is_viewable_image();
        let bg = if picked {
            theme::SEL_BG
        } else if m.is_from_me {
            theme::BUBBLE_OUT
        } else {
            theme::BUBBLE_IN
        };
        let bubble_style = Style::default().bg(bg).fg(theme::TEXT);

        let mut bubble_lines: Vec<Line<'static>> = Vec::new();

        if m.is_viewable_image() {
            let has_thumb = app.message_thumbnail(&m.id).is_some();
            if let Some(thumb) = app.message_thumbnail(&m.id) {
                let preview_w = termimg::INLINE_PREVIEW_COLS
                    .min(max_bubble as u16)
                    .max(8);
                let preview =
                    termimg::render_image(thumb, preview_w, termimg::INLINE_PREVIEW_ROWS);
                bubble_lines.extend(preview_lines_with_bg(&preview, bg));
            }

            let caption = m.content.trim();
            if !caption.is_empty() {
                for line in wrap_text(caption, max_bubble.saturating_sub(2)) {
                    bubble_lines.push(Line::from(Span::styled(line, bubble_style)));
                }
            } else if !has_thumb && !app.thumbnail_loading(&m.id) {
                for line in wrap_text(&m.body_for_display(), max_bubble.saturating_sub(2)) {
                    bubble_lines.push(Line::from(Span::styled(line, bubble_style)));
                }
            }
        } else {
            for line in wrap_text(&m.body_for_display(), max_bubble.saturating_sub(2)) {
                bubble_lines.push(Line::from(Span::styled(line, bubble_style)));
            }
        }

        if bubble_lines.is_empty() {
            bubble_lines.push(Line::from(Span::styled(
                m.body_for_display(),
                bubble_style,
            )));
        }

        let bubble_w = bubble_lines
            .iter()
            .map(line_width)
            .max()
            .unwrap_or(0)
            .max(4);

        for line in bubble_lines {
            push_bubble_line(&mut out, line, bubble_w, bg, m.is_from_me, inner_w);
        }

        let time = m
            .timestamp
            .as_deref()
            .map(short_time)
            .unwrap_or_default();
        let meta = if m.is_from_me {
            format!("{} ✓", time)
        } else {
            let sender = if m.sender_jid.is_empty() {
                String::new()
            } else {
                app.jid_display_name(&m.sender_jid)
            };
            if sender.is_empty() {
                time
            } else {
                format!("{} · {}", sender, time)
            }
        };
        let meta_span = Span::styled(meta.clone(), theme::dim());
        if m.is_from_me {
            let lead = inner_w.saturating_sub(meta.chars().count());
            out.push(Line::from(vec![Span::raw(" ".repeat(lead)), meta_span]));
        } else {
            out.push(Line::from(meta_span));
        }

        if idx + 1 < messages.len() {
            out.push(Line::raw(""));
        }
    }
    out
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans.iter().map(|s| s.width()).sum()
}

fn preview_lines_with_bg(lines: &[Line<'static>], bg: Color) -> Vec<Line<'static>> {
    lines
        .iter()
        .map(|line| {
            let mut spans = vec![Span::styled(" ", Style::default().bg(bg))];
            for s in &line.spans {
                let style = s.style.patch(Style::default().bg(bg));
                spans.push(Span::styled(s.content.clone(), style));
            }
            spans.push(Span::styled(" ", Style::default().bg(bg)));
            Line::from(spans)
        })
        .collect()
}

fn push_bubble_line(
    out: &mut Vec<Line<'static>>,
    line: Line<'static>,
    bubble_w: usize,
    bg: Color,
    from_me: bool,
    inner_w: usize,
) {
    let line_w = line_width(&line);
    let pad = bubble_w.saturating_sub(line_w);
    let pad_left = pad / 2;
    let pad_right = pad - pad_left;

    let mut spans = Vec::new();
    if pad_left > 0 {
        spans.push(Span::styled(
            " ".repeat(pad_left),
            Style::default().bg(bg),
        ));
    }
    spans.extend(line.spans);
    if pad_right > 0 {
        spans.push(Span::styled(
            " ".repeat(pad_right),
            Style::default().bg(bg),
        ));
    }

    let w = line_width(&Line::from(spans.clone()));
    if from_me {
        let lead = inner_w.saturating_sub(w);
        let mut row = vec![Span::raw(" ".repeat(lead))];
        row.extend(spans);
        out.push(Line::from(row));
    } else {
        out.push(Line::from(spans));
    }
}

fn draw_compose(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Compose;
    let block = panel_block(" Message ", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.compose.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("Type a message…", theme::dim())),
            inner,
        );
    } else {
        f.render_widget(Paragraph::new(Span::styled(app.compose.clone(), theme::text())), inner);
    }

    if focused {
        let cursor_x = inner.x + (app.compose.chars().count() as u16).min(inner.width.saturating_sub(1));
        f.set_cursor_position(Position::new(cursor_x, inner.y));
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::horizontal([Constraint::Min(0), Constraint::Length(28)]).split(area);

    let hints = match app.focus {
        Focus::Chats if app.current_jid.is_some() => {
            " j/k chat   [/] image   v view   PgUp/Dn scroll   i compose   r refresh   q quit "
        }
        Focus::Chats => {
            " j/k chat   / search   a archived   enter open   r refresh   q quit "
        }
        Focus::Search => " type filter   j/k move   enter open   esc clear ",
        Focus::Compose => " type message   enter send   esc cancel ",
        Focus::ImageViewer => " esc close   v toggle ",
    };
    f.render_widget(
        Paragraph::new(Span::styled(hints, theme::dim())).style(theme::screen_bg()),
        cols[0],
    );

    if !app.status.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{} ", app.status),
                Style::default().fg(theme::ACCENT_BRIGHT),
            ))
            .alignment(Alignment::Right)
            .style(theme::screen_bg()),
            cols[1],
        );
    }
}

fn panel_block(title: &str, focused: bool) -> Block<'static> {
    let border_color = if focused {
        theme::ACCENT_BRIGHT
    } else {
        theme::DIM
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            title.to_string(),
            Style::default()
                .fg(if focused { theme::ACCENT_BRIGHT } else { theme::TEXT })
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme::panel_bg())
}

fn draw_image_overlay(f: &mut Frame, app: &App, area: Rect) {
    f.render_widget(Block::default().style(theme::screen_bg()), area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::ACCENT_BRIGHT))
        .title(Span::styled(
            " Image ",
            Style::default()
                .fg(theme::ACCENT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme::panel_bg());

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.loading_image {
        f.render_widget(
            centered_hint(&format!("{} loading image…", app.spinner_frame())),
            inner,
        );
        return;
    }

    let Some(view) = &app.image_view else {
        f.render_widget(centered_hint(&app.status), inner);
        return;
    };

    let caption_h = if view.caption.is_empty() { 0 } else { 2 };
    let rows = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(caption_h),
    ])
    .split(inner);

    let lines = termimg::render_image(&view.image, rows[0].width, rows[0].height);
    if lines.is_empty() {
        f.render_widget(centered_hint("Image too small to render"), rows[0]);
    } else {
        let paragraph = Paragraph::new(Text::from(lines));
        f.render_widget(paragraph, rows[0]);
    }

    if caption_h > 0 {
        f.render_widget(
            Paragraph::new(Span::styled(
                view.caption.clone(),
                theme::dim(),
            ))
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Center),
            rows[1],
        );
    }
}

fn centered_hint(text: &str) -> Paragraph<'static> {
    Paragraph::new(text.to_string())
        .style(theme::dim())
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
}

fn device_name(d: &Device) -> String {
    if d.display_name.trim().is_empty() {
        short_jid(&d.jid)
    } else {
        d.display_name.clone()
    }
}

fn short_jid(jid: &str) -> String {
    jid.split('@').next().unwrap_or(jid).to_string()
}

fn initial(name: &str) -> char {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('#')
}

fn pad_truncate(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count > width {
        if width <= 1 {
            return s.chars().take(width).collect();
        }
        let mut t: String = s.chars().take(width - 1).collect();
        t.push('…');
        t
    } else {
        format!("{:<width$}", s, width = width)
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();

    for raw in text.split('\n') {
        if raw.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for word in raw.split_whitespace() {
            let wlen = word.chars().count();
            if wlen > width {
                if !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                }
                let chars: Vec<char> = word.chars().collect();
                for chunk in chars.chunks(width) {
                    lines.push(chunk.iter().collect());
                }
                continue;
            }
            if cur.is_empty() {
                cur = word.to_string();
            } else if cur.chars().count() + 1 + wlen <= width {
                cur.push(' ');
                cur.push_str(word);
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn parse_ts(ts: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&Local))
}

fn short_time(ts: &str) -> String {
    match parse_ts(ts) {
        Some(dt) => dt.format("%H:%M").to_string(),
        None => String::new(),
    }
}

fn relative_time(ts: &str) -> String {
    let Some(dt) = parse_ts(ts) else {
        return String::new();
    };
    let now = Local::now();
    let secs = now.signed_duration_since(dt).num_seconds();
    if secs < 0 {
        return dt.format("%H:%M").to_string();
    }
    if secs < 60 {
        "now".to_string()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else if secs < 7 * 86_400 {
        dt.format("%a").to_string()
    } else {
        dt.format("%d/%m").to_string()
    }
}
