use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{self, UnboundedSender};

use whatsatui::api::ApiClient;
use whatsatui::app::{App, AppEvent, Focus, ImageView, MESSAGES_PAGE};
use whatsatui::media::{self, MediaRow};
use whatsatui::termimg;
use whatsatui::archive;
use whatsatui::config::Config;
use whatsatui::{ui, webhook};

type Tx = UnboundedSender<AppEvent>;

/// Minimum gap between live-triggered chat-list refetches, to coalesce bursts.
const CHAT_REFRESH_DEBOUNCE: Duration = Duration::from_millis(600);

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::from_env();
    let client = ApiClient::new(cfg.clone())?;

    let mut terminal = ratatui::init();
    install_panic_hook();

    let result = run(&mut terminal, client, cfg).await;

    ratatui::restore();
    result
}

async fn run(terminal: &mut DefaultTerminal, client: ApiClient, cfg: Config) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut app = App::new();

    // Blocking terminal-input reader on its own OS thread.
    {
        let itx = tx.clone();
        std::thread::spawn(move || input_loop(itx));
    }

    // Live updates: local webhook receiver the gateway pushes events to.
    if let Err(e) = webhook::spawn(&cfg, tx.clone()) {
        let _ = tx.send(AppEvent::Error(format!("webhook listener: {e}")));
    }

    // Animation tick for the loading spinner.
    {
        let ttx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(120));
            loop {
                interval.tick().await;
                if ttx.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        });
    }

    // Initial data load.
    app.loading_chats = true;
    spawn_devices(&client, &tx);
    spawn_chats(
        &client,
        &tx,
        &cfg,
        app.show_archived,
        app.next_chats_generation(),
    );
    spawn_contacts(&client, &tx);
    spawn_groups(&client, &tx);

    terminal.draw(|f| ui::draw(f, &app))?;

    while let Some(ev) = rx.recv().await {
        match ev {
            AppEvent::Tick => {
                if app.is_loading() {
                    app.spinner = app.spinner.wrapping_add(1);
                } else {
                    continue; // avoid redrawing when idle
                }
            }
            AppEvent::Input(input) => handle_input(&mut app, input, &client, &tx, &cfg),
            AppEvent::Devices(devices) => app.set_devices(devices),
            AppEvent::Chats {
                chats,
                generation,
                show_archived,
            } => {
                let had_open = app.current_jid.is_some();
                app.set_chats(chats, generation, show_archived);
                let jids: Vec<String> = app
                    .visible_chat_indices()
                    .into_iter()
                    .take(40)
                    .filter_map(|i| app.chats.get(i).map(|c| c.jid.clone()))
                    .collect();
                spawn_chat_previews(&client, &tx, jids);
                if !had_open {
                    if let Some(jid) = app.selected_jid() {
                        app.open_jid(jid.clone());
                        let gen = app.next_messages_generation();
                        spawn_messages(&client, &tx, jid, gen);
                    }
                }
            }
            AppEvent::Messages {
                jid,
                messages,
                generation,
                total,
            } => {
                app.set_messages(jid, messages, generation, total);
                maybe_spawn_thumbnails(&mut app, &client, &tx, &cfg);
            }
            AppEvent::OlderMessages {
                jid,
                messages,
                generation,
                total,
                offset,
            } => {
                app.prepend_older_messages(jid, messages, generation, total, offset);
                maybe_spawn_thumbnails(&mut app, &client, &tx, &cfg);
            }
            AppEvent::ChatPreview { jid, preview } => app.set_chat_preview(jid, preview),
            AppEvent::Sent { jid, text } => {
                app.on_sent();
                let gen = app.next_messages_generation();
                spawn_messages_after_send(&client, &tx, jid, text, gen);
            }
            AppEvent::SendFailed { text } => app.remove_pending_outgoing(&text),
            AppEvent::Contacts(contacts) => app.set_contacts(contacts),
            AppEvent::Groups(groups) => app.set_groups(groups),
            AppEvent::Webhook {
                event,
                chat_id,
                is_from_me: _,
            } => handle_webhook(&mut app, &client, &tx, &cfg, &event, chat_id),
            AppEvent::ImageReady { message_id, view } => {
                app.set_image_view(message_id, view);
            }
            AppEvent::ImageFailed { message_id, error } => {
                app.image_view_failed(message_id, error);
            }
            AppEvent::ThumbnailReady { message_id, image } => {
                app.set_message_thumbnail(message_id, image);
            }
            AppEvent::ThumbnailFailed { message_id } => {
                app.thumbnail_fetch_done(&message_id);
            }
            AppEvent::Error(e) => {
                app.loading_older_messages = false;
                app.loading_messages = false;
                app.loading_chats = false;
                app.loading_image = false;
                app.status = e;
            }
        }

        if app.should_quit {
            break;
        }
        terminal.draw(|f| ui::draw(f, &app))?;
    }

    Ok(())
}

fn handle_input(app: &mut App, input: Event, client: &ApiClient, tx: &Tx, cfg: &Config) {
    let key = match input {
        Event::Key(k) if k.kind != KeyEventKind::Release => k,
        _ => return,
    };

    // Global: Ctrl-C always quits.
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        app.should_quit = true;
        return;
    }

    if app.focus == Focus::ImageViewer {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('v') => app.close_image_viewer(),
            _ => {}
        }
        return;
    }

    match app.focus {
        Focus::Chats => match key.code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => app.select_next(),
            KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
            KeyCode::Char('/') => app.enter_search(),
            KeyCode::Char('a') => {
                app.toggle_archived();
                spawn_chats(
                    client,
                    tx,
                    cfg,
                    app.show_archived,
                    app.next_chats_generation(),
                );
            }
            KeyCode::Enter => {
                if let Some(jid) = app.selected_jid() {
                    app.open_jid(jid.clone());
                    let gen = app.next_messages_generation();
                    spawn_messages(client, tx, jid, gen);
                }
            }
            KeyCode::Char('i') | KeyCode::Tab => {
                if app.current_jid.is_some() {
                    app.focus = Focus::Compose;
                }
            }
            KeyCode::Char('r') => {
                app.loading_chats = true;
                app.status = "Refreshing…".to_string();
                spawn_chats(
                    client,
                    tx,
                    cfg,
                    app.show_archived,
                    app.next_chats_generation(),
                );
                spawn_contacts(client, tx);
                spawn_groups(client, tx);
                if let Some(jid) = app.current_jid.clone() {
                    app.loading_messages = true;
                    let gen = app.next_messages_generation();
                    spawn_messages(client, tx, jid, gen);
                }
            }
            KeyCode::PageUp | KeyCode::Home => {
                handle_message_scroll_up(app, client, tx, key.code);
            }
            KeyCode::PageDown | KeyCode::End => handle_message_scroll_down(app, key.code),
            KeyCode::Char('v') if app.current_jid.is_some() => {
                if let (Some(jid), Some(msg)) =
                    (app.current_jid.clone(), app.picked_media_message().cloned())
                {
                    app.begin_image_view(msg.id.clone());
                    spawn_image_view(&client, tx, cfg, jid, msg);
                } else {
                    app.status = "No image in this chat".to_string();
                }
            }
            KeyCode::Char('[') if app.current_jid.is_some() => {
                app.step_media_pick(-1);
            }
            KeyCode::Char(']') if app.current_jid.is_some() => {
                app.step_media_pick(1);
            }
            _ => {}
        },
        Focus::Search => match key.code {
            KeyCode::Esc => app.exit_search(),
            KeyCode::Char('j') | KeyCode::Down => app.select_next(),
            KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
            KeyCode::Enter => {
                if let Some(jid) = app.selected_jid() {
                    app.focus = Focus::Chats;
                    app.open_jid(jid.clone());
                    let gen = app.next_messages_generation();
                    spawn_messages(client, tx, jid, gen);
                }
            }
            KeyCode::Backspace => {
                app.chat_filter.pop();
                app.clamp_selection();
            }
            KeyCode::Char(c) => {
                app.chat_filter.push(c);
                app.clamp_selection();
            }
            _ => {}
        },
        Focus::ImageViewer => {}
        Focus::Compose => match key.code {
            KeyCode::Esc | KeyCode::Tab => app.focus = Focus::Chats,
            KeyCode::Enter => {
                let text = app.compose.trim().to_string();
                if !text.is_empty() {
                    if let Some(jid) = app.current_jid.clone() {
                        let sender = app
                            .device
                            .as_ref()
                            .map(|d| d.jid.clone())
                            .unwrap_or_else(|| jid.clone());
                        app.append_outgoing(text.clone(), &sender);
                        app.status = "Sending…".to_string();
                        spawn_send(client, tx, jid, text);
                        app.compose.clear();
                    }
                }
            }
            KeyCode::Backspace => {
                app.compose.pop();
            }
            KeyCode::Char(c) => app.compose.push(c),
            _ => {}
        },
    }
}

fn input_loop(tx: Tx) {
    loop {
        match event::poll(Duration::from_millis(200)) {
            Ok(true) => match event::read() {
                Ok(ev) => {
                    if tx.send(AppEvent::Input(ev)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            Ok(false) => {}
            Err(_) => break,
        }
    }
}

/// React to a live webhook event: refresh the open chat and/or the chat list.
fn handle_webhook(
    app: &mut App,
    client: &ApiClient,
    tx: &Tx,
    cfg: &Config,
    event: &str,
    chat_id: Option<String>,
) {
    // Only message-related events affect what we render.
    if !event.starts_with("message") {
        return;
    }

    // If the event targets the currently open chat, refetch its messages.
    if let (Some(open), Some(target)) = (app.current_jid.clone(), chat_id.as_ref()) {
        if &open == target {
            let gen = app.next_messages_generation();
            spawn_messages(client, tx, open, gen);
        }
    }

    // Reorder/update the chat list for events that change it (debounced).
    let affects_list = matches!(
        event,
        "message" | "message.revoked" | "message.edited" | "message.deleted"
    );
    if affects_list && app.should_refresh_chats(CHAT_REFRESH_DEBOUNCE) {
        spawn_chats(
            client,
            tx,
            cfg,
            app.show_archived,
            app.next_chats_generation(),
        );
        spawn_groups(client, tx);
    }
    if let Some(jid) = chat_id {
        spawn_chat_preview(client, tx, jid);
    }
}

fn estimated_max_scroll(app: &App) -> u16 {
    app.messages.len().saturating_mul(3) as u16
}

fn handle_message_scroll_up(app: &mut App, client: &ApiClient, tx: &Tx, key: KeyCode) {
    if app.current_jid.is_none() {
        return;
    }
    if key == KeyCode::Home {
        app.scroll_messages_to_top();
    } else {
        app.scroll_messages_page_up();
    }
    try_load_older_messages(app, client, tx);
}

fn try_load_older_messages(app: &mut App, client: &ApiClient, tx: &Tx) {
    let max_est = estimated_max_scroll(app);
    if app.msg_scroll_from_bottom < max_est.saturating_sub(4) {
        return;
    }
    if !app.has_older_messages() || app.loading_older_messages {
        return;
    }
    if let Some(jid) = app.current_jid.clone() {
        app.loading_older_messages = true;
        let offset = app.messages_fetched;
        let gen = app.active_messages_generation;
        spawn_older_messages(client, tx, jid, offset, gen);
    }
}

fn handle_message_scroll_down(app: &mut App, key: KeyCode) {
    if app.current_jid.is_none() {
        return;
    }
    match key {
        KeyCode::End => app.scroll_messages_to_bottom(),
        _ => app.scroll_messages_page_down(),
    }
}

fn spawn_devices(client: &ApiClient, tx: &Tx) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        match client.list_devices().await {
            Ok(d) => {
                let _ = tx.send(AppEvent::Devices(d));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(format!("devices: {e}")));
            }
        }
    });
}

fn spawn_contacts(client: &ApiClient, tx: &Tx) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        match client.list_contacts().await {
            Ok(c) => {
                let _ = tx.send(AppEvent::Contacts(c));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(format!("contacts: {e}")));
            }
        }
    });
}

fn spawn_groups(client: &ApiClient, tx: &Tx) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        match client.list_groups().await {
            Ok(g) => {
                let _ = tx.send(AppEvent::Groups(g));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(format!("groups: {e}")));
            }
        }
    });
}

/// Max concurrent inline thumbnail fetches per messages load.
const THUMBNAIL_BATCH: usize = 8;

fn maybe_spawn_thumbnails(app: &mut App, client: &ApiClient, tx: &Tx, cfg: &Config) {
    let Some(jid) = app.current_jid.clone() else {
        return;
    };
    for msg in app.messages_needing_thumbnails(THUMBNAIL_BATCH) {
        app.mark_thumbnail_pending(&msg.id);
        spawn_message_thumbnail(client, tx, cfg, jid.clone(), msg);
    }
}

fn spawn_message_thumbnail(
    client: &ApiClient,
    tx: &Tx,
    cfg: &Config,
    chat_jid: String,
    message: whatsatui::api::Message,
) {
    let client = client.clone();
    let tx = tx.clone();
    let db_path = cfg.chatstorage_db.clone();
    let message_id = message.id.clone();
    tokio::spawn(async move {
        let result = async {
            let row = media_row_for_message(&db_path, &message_id, &chat_jid, &message)?;
            let http = media_http_client()?;
            let bytes = media::download_decrypted(&http, &row).await?;
            let img = termimg::decode_image(&bytes)?;
            Ok::<_, anyhow::Error>(termimg::thumbnail_image(&img))
        }
        .await;

        match result {
            Ok(thumb) => {
                let _ = tx.send(AppEvent::ThumbnailReady {
                    message_id,
                    image: thumb,
                });
            }
            Err(_) => {
                let _ = tx.send(AppEvent::ThumbnailFailed { message_id });
            }
        }
        let _ = client;
    });
}

fn spawn_image_view(
    client: &ApiClient,
    tx: &Tx,
    cfg: &Config,
    chat_jid: String,
    message: whatsatui::api::Message,
) {
    let client = client.clone();
    let tx = tx.clone();
    let db_path = cfg.chatstorage_db.clone();
    let message_id = message.id.clone();
    tokio::spawn(async move {
        let result = async {
            let row = media_row_for_message(&db_path, &message_id, &chat_jid, &message)?;
            let http = media_http_client()?;
            let bytes = media::download_decrypted(&http, &row).await?;
            let img = termimg::decode_image(&bytes)?;
            let caption = image_caption(&message);
            Ok::<_, anyhow::Error>((img, caption))
        }
        .await;

        match result {
            Ok((image, caption)) => {
                let _ = tx.send(AppEvent::ImageReady {
                    message_id,
                    view: ImageView { image, caption },
                });
            }
            Err(e) => {
                let _ = tx.send(AppEvent::ImageFailed {
                    message_id,
                    error: format!("Image: {e}"),
                });
            }
        }
        let _ = client;
    });
}

fn media_row_for_message(
    db_path: &str,
    message_id: &str,
    chat_jid: &str,
    message: &whatsatui::api::Message,
) -> anyhow::Result<MediaRow> {
    if let Some(mut row) =
        media::load_media_row(std::path::Path::new(db_path), message_id, chat_jid)
    {
        if message.url.is_empty() {
            return Ok(row);
        }
        row.url = message.url.clone();
        return Ok(row);
    }
    if !message.url.is_empty() {
        return Err(anyhow::anyhow!(
            "media keys not found in chatstorage (set WHATSATUI_CHATSTORAGE_DB)"
        ));
    }
    Err(anyhow::anyhow!("message has no downloadable media"))
}

fn image_caption(message: &whatsatui::api::Message) -> String {
    let mut parts = Vec::new();
    if !message.filename.trim().is_empty() {
        parts.push(message.filename.trim().to_string());
    }
    if !message.content.trim().is_empty() {
        parts.push(message.content.trim().to_string());
    }
    if parts.is_empty() {
        "[image]".to_string()
    } else {
        parts.join(" — ")
    }
}

fn media_http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("whatsatui/0.1")
        .build()?)
}

fn spawn_chats(
    client: &ApiClient,
    tx: &Tx,
    cfg: &Config,
    show_archived: bool,
    generation: u64,
) {
    let client = client.clone();
    let tx = tx.clone();
    let whatsmeow_db = cfg.whatsmeow_db.clone();
    tokio::spawn(async move {
        let db_path = std::path::Path::new(&whatsmeow_db);
        let archived_jids = archive::load_archived_jids(db_path);

        let result = if show_archived {
            client.list_all_chats().await.map(|chats| {
                chats
                    .into_iter()
                    .filter(|c| archived_jids.contains(&c.jid))
                    .collect::<Vec<_>>()
            })
        } else {
            client.list_chats(100, 0, None).await.map(|page| {
                page
                    .chats
                    .into_iter()
                    .filter(|c| !archived_jids.contains(&c.jid))
                    .collect::<Vec<_>>()
            })
        };

        match result {
            Ok(chats) => {
                let _ = tx.send(AppEvent::Chats {
                    chats,
                    generation,
                    show_archived,
                });
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(format!("chats: {e}")));
            }
        }
    });
}

fn spawn_messages(client: &ApiClient, tx: &Tx, jid: String, generation: u64) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        match client.get_messages(&jid, MESSAGES_PAGE, 0).await {
            Ok(page) => {
                let _ = tx.send(AppEvent::Messages {
                    jid,
                    messages: page.messages,
                    generation,
                    total: page.total,
                });
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(format!("messages: {e}")));
            }
        }
    });
}

fn spawn_older_messages(
    client: &ApiClient,
    tx: &Tx,
    jid: String,
    offset: u32,
    generation: u64,
) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        match client.get_messages(&jid, MESSAGES_PAGE, offset).await {
            Ok(page) => {
                let _ = tx.send(AppEvent::OlderMessages {
                    jid,
                    messages: page.messages,
                    generation,
                    total: page.total,
                    offset,
                });
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Error(format!("older messages: {e}")));
            }
        }
    });
}

fn spawn_chat_previews(client: &ApiClient, tx: &Tx, jids: Vec<String>) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        for jid in jids {
            if let Ok(page) = client.get_messages(&jid, 1, 0).await {
                if let Some(m) = page.messages.first() {
                    let preview = m.preview_text();
                    let _ = tx.send(AppEvent::ChatPreview { jid, preview });
                }
            }
        }
    });
}

fn spawn_chat_preview(client: &ApiClient, tx: &Tx, jid: String) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        if let Ok(page) = client.get_messages(&jid, 1, 0).await {
            if let Some(m) = page.messages.first() {
                let preview = m.preview_text();
                let _ = tx.send(AppEvent::ChatPreview { jid, preview });
            }
        }
    });
}

/// Refetch after send with a short delay so the gateway has time to persist
/// the new message, and retry a few times if it is still missing.
fn spawn_messages_after_send(
    client: &ApiClient,
    tx: &Tx,
    jid: String,
    sent_text: String,
    generation: u64,
) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;

        for attempt in 0..4 {
            match client.get_messages(&jid, MESSAGES_PAGE, 0).await {
                Ok(page) => {
                    let confirmed = page
                        .messages
                        .iter()
                        .take(3)
                        .any(|m| m.is_from_me && m.content == sent_text);
                    let _ = tx.send(AppEvent::Messages {
                        jid: jid.clone(),
                        messages: page.messages,
                        generation,
                        total: page.total,
                    });
                    if confirmed || attempt == 3 {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("messages: {e}")));
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
    });
}

fn spawn_send(client: &ApiClient, tx: &Tx, jid: String, text: String) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        match client.send_message(&jid, &text).await {
            Ok(_) => {
                let _ = tx.send(AppEvent::Sent { jid, text });
            }
            Err(e) => {
                let _ = tx.send(AppEvent::SendFailed { text });
                let _ = tx.send(AppEvent::Error(format!("send: {e}")));
            }
        }
    });
}

/// Restore the terminal before printing a panic, so the user's shell isn't left
/// in raw mode / the alternate screen.
fn install_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        hook(info);
    }));
}
