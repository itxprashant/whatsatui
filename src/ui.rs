use chrono::{DateTime, Local, NaiveDate};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, List, ListItem, Paragraph, Wrap},
    Frame,
};

use ratatui_image::Image;
use ratatui_image::StatefulImage;

use crate::api::{Chat, Device, Message};
use crate::app::{App, Focus};
use crate::termimg;
use crate::theme;

struct ImageSlot {
    message_id: String,
    line_index: usize,
    cols: u16,
    rows: u16,
    from_me: bool,
    bubble_w: u16,
}

struct BubbleLayout {
    lines: Vec<Line<'static>>,
    image_slots: Vec<ImageSlot>,
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Block::default().style(theme::screen_bg()), area);

    let rows = Layout::vertical([
        Constraint::Length(2),
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
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    let cols = Layout::horizontal([Constraint::Min(0), Constraint::Length(20)]).split(rows[0]);

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

    let rule_w = area.width as usize;
    let rule = "─".repeat(rule_w);
    f.render_widget(
        Paragraph::new(Span::styled(rule, theme::dim())).style(theme::screen_bg()),
        rows[1],
    );
}

fn draw_body(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(32), Constraint::Min(0)]).split(area);

    // Messages before chats so native graphics (Sixel) cannot paint over the sidebar.
    let right = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(cols[1]);
    draw_messages(f, app, right[0]);
    draw_compose(f, app, right[1]);
    draw_chats(f, app, cols[0]);
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

fn draw_chats(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Chats || app.focus == Focus::Search;
    let block = panel_block(&chats_title(app), focused, false);
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

    let sel = app.selected.min(visible.len().saturating_sub(1));
    if app.chat_list_state.selected() != Some(sel) {
        app.chat_list_state.select(Some(sel));
    }
    f.render_stateful_widget(list, inner, &mut app.chat_list_state);
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
        let preview_disp = pad_truncate(preview, content_w.saturating_sub(3));
        lines.push(Line::from(vec![
            Span::raw("  "),
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
    let active = app.current_jid.is_some() && app.focus == Focus::Chats;
    let block = panel_block(&title, false, active);
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

    let layout = build_bubble_layout(app, &app.messages, inner.width as usize, app.media_pick);
    let total = layout.lines.len() as u16;
    let max_scroll = total.saturating_sub(inner.height);
    let scroll = max_scroll.saturating_sub(app.msg_scroll_from_bottom.min(max_scroll));

    let mut content = layout.lines;
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
    if app.native_inline_images {
        render_image_slots(f, app, inner, &layout.image_slots, scroll);
    }
}

fn render_image_slots(
    f: &mut Frame,
    app: &App,
    inner: Rect,
    slots: &[ImageSlot],
    scroll: u16,
) {
    for slot in slots {
        let Some(protocol) = app.image_protocol(&slot.message_id) else {
            continue;
        };
        if inner.width < slot.cols {
            continue;
        }
        let start = slot.line_index as u16;
        let end = start.saturating_add(slot.rows);
        if end <= scroll || start >= scroll.saturating_add(inner.height) {
            continue;
        }
        let y = inner.y + start.saturating_sub(scroll);
        let x = if slot.from_me {
            inner.x + inner.width.saturating_sub(slot.bubble_w)
        } else {
            inner.x
        };
        let rect = Rect::new(x, y, slot.cols, slot.rows);
        f.render_widget(Image::new(protocol), rect);
    }
}

fn build_bubble_layout(
    app: &App,
    messages: &[Message],
    inner_w: usize,
    media_pick: Option<usize>,
) -> BubbleLayout {
    let max_bubble = (inner_w * 7 / 10).max(12);
    let mut out: Vec<Line> = Vec::new();
    let mut image_slots: Vec<ImageSlot> = Vec::new();
    let mut last_date: Option<NaiveDate> = None;

    for (idx, m) in messages.iter().enumerate() {
        if let Some(ts) = m.timestamp.as_deref().and_then(parse_ts) {
            let msg_date = ts.date_naive();
            if last_date != Some(msg_date) {
                if let Some(divider) = format_date_divider(msg_date, Local::now()) {
                    if !out.is_empty() {
                        out.push(Line::raw(""));
                    }
                    out.push(centered_divider_line(&divider, inner_w));
                    out.push(Line::raw(""));
                }
                last_date = Some(msg_date);
            }
        }

        let picked = media_pick == Some(idx) && m.is_viewable_image();
        let bg = if picked {
            theme::BUBBLE_PICK
        } else if m.is_from_me {
            theme::BUBBLE_OUT
        } else {
            theme::BUBBLE_IN
        };
        let bubble_style = Style::default().bg(bg).fg(theme::TEXT);

        let mut bubble_lines: Vec<Line<'static>> = Vec::new();
        let mut image_slot: Option<(String, u16, u16)> = None;

        if m.is_viewable_image() {
            let inline = app.native_inline_images;
            let has_protocol = inline && app.image_protocol(&m.id).is_some();
            let preview_fits = inner_w >= termimg::INLINE_PREVIEW_COLS as usize;
            if has_protocol && preview_fits {
                image_slot = Some((
                    m.id.clone(),
                    termimg::INLINE_PREVIEW_COLS,
                    termimg::INLINE_PREVIEW_ROWS,
                ));
            } else if inline && app.thumbnail_loading(&m.id) {
                let placeholder = format!("{} image…", app.spinner_frame());
                bubble_lines.push(Line::from(Span::styled(placeholder, theme::dim())));
            }

            let caption = m.content.trim();
            if !caption.is_empty() {
                for line in wrap_text(caption, max_bubble.saturating_sub(2)) {
                    bubble_lines.push(Line::from(Span::styled(line, bubble_style)));
                }
            } else if !has_protocol && !(inline && app.thumbnail_loading(&m.id)) {
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

        let mut bubble_w = bubble_lines
            .iter()
            .map(line_width)
            .max()
            .unwrap_or(0);
        if let Some((_, preview_w, _)) = &image_slot {
            bubble_w = bubble_w.max(*preview_w as usize);
        }
        bubble_w = bubble_w.max(4);

        if picked {
            push_pick_marker(&mut out, m.is_from_me, inner_w);
        }

        let slot_line = out.len();
        if let Some((message_id, cols, rows)) = image_slot {
            // Blank lines only: colored bubble padding under a Sixel overlay leaves
            // skip cells that ratatui never clears, which show up as rogue blocks.
            for _ in 0..rows {
                out.push(Line::raw(""));
            }
            image_slots.push(ImageSlot {
                message_id,
                line_index: slot_line,
                cols,
                rows,
                from_me: m.is_from_me,
                bubble_w: bubble_w as u16,
            });
        }
        for line in bubble_lines {
            push_bubble_line(&mut out, line, bubble_w, bg, m.is_from_me, inner_w);
        }

        let time = m
            .timestamp
            .as_deref()
            .map(short_time)
            .unwrap_or_default();
        let ack = app.ack_for(m);
        let ticks = ack.display_ticks();
        if m.is_from_me {
            let time_part = format!("{} ", time);
            let tick_span = Span::styled(ticks, theme::ack_style(ack));
            let meta_len = time_part.chars().count() + ticks.chars().count();
            let lead = inner_w.saturating_sub(meta_len);
            out.push(Line::from(vec![
                Span::raw(" ".repeat(lead)),
                Span::styled(time_part, theme::dim()),
                tick_span,
            ]));
        } else {
            let sender = if m.sender_jid.is_empty() {
                String::new()
            } else {
                app.jid_display_name(&m.sender_jid)
            };
            let meta = if sender.is_empty() {
                time
            } else {
                format!("{} · {}", sender, time)
            };
            out.push(Line::from(Span::styled(meta, theme::dim())));
        }

        if idx + 1 < messages.len() {
            out.push(Line::raw(""));
        }
    }
    BubbleLayout {
        lines: out,
        image_slots,
    }
}

fn push_pick_marker(out: &mut Vec<Line<'static>>, from_me: bool, inner_w: usize) {
    let marker = Span::styled("▎", Style::default().fg(theme::ACCENT_BRIGHT));
    if from_me {
        let lead = inner_w.saturating_sub(1);
        out.push(Line::from(vec![Span::raw(" ".repeat(lead)), marker]));
    } else {
        out.push(Line::from(marker));
    }
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans.iter().map(|s| s.width()).sum()
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
    let block = panel_block(" Message ", focused, false);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.compose.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("Type a message…", theme::dim())),
            inner,
        );
    } else {
        let width = inner.width.max(1) as usize;
        let wrapped = wrap_compose_display(&app.compose, width, inner.height as usize);
        let lines: Vec<Line> = wrapped
            .into_iter()
            .map(|l| Line::from(Span::styled(l, theme::text())))
            .collect();
        f.render_widget(Paragraph::new(lines), inner);
    }

    if focused && !app.compose.is_empty() {
        let (cursor_x, cursor_y) = compose_cursor_pos(
            &app.compose,
            app.compose_cursor,
            inner.width.max(1) as usize,
            inner.height as usize,
        );
        f.set_cursor_position(Position::new(
            inner.x + cursor_x.min(inner.width.saturating_sub(1)),
            inner.y + cursor_y.min(inner.height.saturating_sub(1)),
        ));
    } else if focused {
        f.set_cursor_position(Position::new(inner.x, inner.y));
    }
}

fn wrap_compose_display(text: &str, width: usize, max_rows: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        lines.extend(wrap_text(paragraph, width));
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    if lines.len() > max_rows {
        lines.truncate(max_rows);
    }
    lines
}

fn compose_cursor_pos(text: &str, cursor: usize, width: usize, max_rows: usize) -> (u16, u16) {
    let mut row = 0u16;
    let mut col = 0u16;
    let mut idx = 0usize;
    for (para_i, paragraph) in text.split('\n').enumerate() {
        if para_i > 0 {
            if idx == cursor {
                return (col, row);
            }
            idx += 1;
            row += 1;
            col = 0;
            if row >= max_rows as u16 {
                return (0, (max_rows.saturating_sub(1)) as u16);
            }
        }
        for line in wrap_text(paragraph, width) {
            for _ch in line.chars() {
                if idx == cursor {
                    return (col, row);
                }
                idx += 1;
                col += 1;
            }
            if idx == cursor {
                return (col, row);
            }
            row += 1;
            col = 0;
            if row >= max_rows as u16 {
                return (0, (max_rows.saturating_sub(1)) as u16);
            }
        }
    }
    (col, row.min((max_rows.saturating_sub(1)) as u16))
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(60), Constraint::Min(28)]).split(area);

    let hints = footer_hints(app);
    f.render_widget(
        Paragraph::new(Line::from(hints)).style(theme::screen_bg()),
        cols[0],
    );

    if !app.status.is_empty() {
        let status = truncate_status(&app.status, cols[1].width as usize);
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{status} "),
                Style::default().fg(theme::ACCENT_BRIGHT),
            ))
            .alignment(Alignment::Right)
            .style(theme::screen_bg()),
            cols[1],
        );
    }
}

fn footer_hints(app: &App) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let add = |spans: &mut Vec<Span<'static>>, key: &str, desc: &str| {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key.to_string(), theme::key_hint()));
        spans.push(Span::styled(format!(" {desc}"), theme::dim()));
    };

    match app.focus {
        Focus::Chats if app.current_jid.is_some() => {
            add(&mut spans, "j/k", "chat");
            add(&mut spans, "[/]", "image");
            add(&mut spans, "v", "view");
            add(&mut spans, "PgUp/Dn", "scroll");
            add(&mut spans, "i", "compose");
            add(&mut spans, "r", "refresh");
            add(&mut spans, "q", "quit");
        }
        Focus::Chats => {
            add(&mut spans, "j/k", "chat");
            add(&mut spans, "/", "search");
            add(&mut spans, "a", "archived");
            add(&mut spans, "enter", "open");
            add(&mut spans, "r", "refresh");
            add(&mut spans, "q", "quit");
        }
        Focus::Search => {
            add(&mut spans, "type", "filter");
            add(&mut spans, "backspace", "edit");
            add(&mut spans, "j/k", "move");
            add(&mut spans, "enter", "open");
            add(&mut spans, "esc", "clear");
        }
        Focus::Compose => {
            add(&mut spans, "enter", "send");
            add(&mut spans, "shift+enter", "newline");
            add(&mut spans, "esc", "back");
        }
        Focus::ImageViewer => {
            add(&mut spans, "esc", "close");
            add(&mut spans, "v", "toggle");
            add(&mut spans, "q", "close");
        }
    }
    spans
}

fn panel_block(title: &str, focused: bool, active: bool) -> Block<'static> {
    let border_color = if focused {
        theme::BORDER_FOCUS
    } else if active {
        theme::BORDER_ACTIVE
    } else {
        theme::BORDER
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            title.to_string(),
            Style::default()
                .fg(if focused {
                    theme::ACCENT_BRIGHT
                } else {
                    theme::TEXT
                })
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme::panel_bg())
}

fn draw_image_overlay(f: &mut Frame, app: &mut App, area: Rect) {
    f.render_widget(Block::default().style(theme::screen_bg()), area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER_FOCUS))
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

    if !app.native_images {
        f.render_widget(
            centered_hint(crate::terminal::unsupported_images_hint()),
            inner,
        );
        return;
    }

    let Some(view) = &mut app.image_view else {
        f.render_widget(centered_hint(&app.status), inner);
        return;
    };

    let caption_h = if view.caption.is_empty() { 0 } else { 2 };
    let rows = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(caption_h),
    ])
    .split(inner);

    if rows[0].width == 0 || rows[0].height == 0 {
        f.render_widget(centered_hint("Image too small to render"), rows[0]);
    } else {
        f.render_stateful_widget(StatefulImage::default(), rows[0], &mut view.protocol);
    }

    if caption_h > 0 {
        let caption = view.caption.clone();
        f.render_widget(
            Paragraph::new(Span::styled(caption, theme::dim()))
                .wrap(Wrap { trim: true })
                .alignment(Alignment::Center),
            rows[1],
        );
    }
}

fn centered_hint(text: &str) -> Paragraph<'static> {
    Paragraph::new(format!("— {text} —"))
        .style(theme::date_divider())
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
}

fn centered_divider_line(label: &str, width: usize) -> Line<'static> {
    let label_len = label.chars().count();
    if width <= label_len {
        return Line::from(Span::styled(
            pad_truncate(label, width),
            theme::date_divider(),
        ));
    }
    let pad = width - label_len;
    let left = pad / 2;
    let right = pad - left;
    Line::from(vec![
        Span::styled("─".repeat(left), theme::date_divider()),
        Span::styled(label.to_string(), theme::date_divider()),
        Span::styled("─".repeat(right), theme::date_divider()),
    ])
}

pub fn format_date_divider(date: NaiveDate, now: DateTime<Local>) -> Option<String> {
    let today = now.date_naive();
    let yesterday = today.pred_opt()?;
    let label = if date == today {
        "Today".to_string()
    } else if date == yesterday {
        "Yesterday".to_string()
    } else {
        date.format("%a %-d %b").to_string()
    };
    Some(format!("── {label} ──"))
}

fn truncate_status(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max <= 1 {
        return s.chars().take(max).collect();
    }
    let mut t: String = s.chars().take(max - 1).collect();
    t.push('…');
    t
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn date_divider_today() {
        let now = Local.with_ymd_and_hms(2025, 6, 10, 12, 0, 0).unwrap();
        let today = now.date_naive();
        assert_eq!(
            format_date_divider(today, now),
            Some("── Today ──".to_string())
        );
    }

    #[test]
    fn date_divider_yesterday() {
        let now = Local.with_ymd_and_hms(2025, 6, 10, 12, 0, 0).unwrap();
        let yesterday = now.date_naive().pred_opt().unwrap();
        assert_eq!(
            format_date_divider(yesterday, now),
            Some("── Yesterday ──".to_string())
        );
    }

    #[test]
    fn date_divider_older() {
        let now = Local.with_ymd_and_hms(2025, 6, 10, 12, 0, 0).unwrap();
        let older = Local.with_ymd_and_hms(2025, 5, 3, 9, 0, 0).unwrap().date_naive();
        let label = format_date_divider(older, now).unwrap();
        assert!(label.contains("May") || label.contains("3"));
    }
}
