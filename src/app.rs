use std::collections::{HashMap, HashSet};
use std::time::Instant;

use image::{DynamicImage, GenericImageView};

use crate::api::{Chat, Contact, Device, Group, Message};
use crate::cache;

/// Which pane currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Chats,
    Compose,
    Search,
    ImageViewer,
}

/// Decoded image shown in the in-terminal viewer overlay.
pub struct ImageView {
    pub image: DynamicImage,
    pub caption: String,
}

impl std::fmt::Debug for ImageView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageView")
            .field("image", &self.image.dimensions())
            .field("caption", &self.caption)
            .finish()
    }
}

/// Events delivered to the main loop from input and background network tasks.
#[derive(Debug)]
pub enum AppEvent {
    Input(ratatui::crossterm::event::Event),
    Tick,
    Devices(Vec<Device>),
    Chats {
        chats: Vec<Chat>,
        generation: u64,
        show_archived: bool,
    },
    Messages {
        jid: String,
        messages: Vec<Message>,
        generation: u64,
        total: u32,
    },
    OlderMessages {
        jid: String,
        messages: Vec<Message>,
        generation: u64,
        total: u32,
        offset: u32,
    },
    ChatPreview {
        jid: String,
        preview: String,
    },
    Sent { jid: String, text: String },
    SendFailed { text: String },
    Contacts(Vec<Contact>),
    Groups(Vec<Group>),
    /// A real-time event pushed by the gateway via the local webhook receiver.
    Webhook {
        event: String,
        chat_id: Option<String>,
        is_from_me: bool,
    },
    ImageReady {
        message_id: String,
        view: ImageView,
    },
    ImageFailed {
        message_id: String,
        error: String,
    },
    ThumbnailReady {
        message_id: String,
        image: DynamicImage,
    },
    ThumbnailFailed {
        message_id: String,
    },
    Error(String),
}

/// Spinner frames shown while a fetch is in flight (all single-width glyphs).
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Default page size for message fetches.
pub const MESSAGES_PAGE: u32 = 50;

pub struct App {
    pub chats: Vec<Chat>,
    pub selected: usize,
    pub messages: Vec<Message>,
    pub current_jid: Option<String>,
    pub device: Option<Device>,
    pub focus: Focus,
    pub compose: String,
    pub chat_filter: String,
    pub show_archived: bool,
    pub status: String,
    pub loading_chats: bool,
    pub chats_generation: u64,
    pub active_chats_generation: u64,
    pub loading_messages: bool,
    pub loading_older_messages: bool,
    pub messages_generation: u64,
    pub active_messages_generation: u64,
    /// How many messages have been fetched from the API (offset for the next page).
    pub messages_fetched: u32,
    pub messages_total: Option<u32>,
    /// Lines scrolled up from the bottom of the conversation (0 = pinned to latest).
    pub msg_scroll_from_bottom: u16,
    pub spinner: usize,
    pub should_quit: bool,
    /// Resolved contact names, keyed by jid.
    pub contacts: HashMap<String, String>,
    /// Resolved group subjects, keyed by group jid.
    pub groups: HashMap<String, String>,
    /// Last-message preview per chat jid (from lazy fetches).
    pub chat_previews: HashMap<String, String>,
    /// Last time a live event triggered a chat-list refetch (for debouncing).
    pub last_chat_refresh: Option<Instant>,
    /// Index into `messages` for [/] image navigation and `v` viewer.
    pub media_pick: Option<usize>,
    pub loading_image: bool,
    pub image_view: Option<ImageView>,
    /// Message id currently being fetched/rendered for the viewer.
    pub image_message_id: Option<String>,
    /// Small inline previews keyed by message id.
    pub message_thumbnails: HashMap<String, DynamicImage>,
    pub thumbnail_pending: HashSet<String>,
}

impl App {
    pub fn new() -> Self {
        Self {
            chats: Vec::new(),
            selected: 0,
            messages: Vec::new(),
            current_jid: None,
            device: None,
            focus: Focus::Chats,
            compose: String::new(),
            chat_filter: String::new(),
            show_archived: false,
            status: String::new(),
            loading_chats: false,
            chats_generation: 0,
            active_chats_generation: 0,
            loading_messages: false,
            loading_older_messages: false,
            messages_generation: 0,
            active_messages_generation: 0,
            messages_fetched: 0,
            messages_total: None,
            msg_scroll_from_bottom: 0,
            spinner: 0,
            should_quit: false,
            contacts: HashMap::new(),
            groups: cache::load_group_names(),
            chat_previews: HashMap::new(),
            last_chat_refresh: None,
            media_pick: None,
            loading_image: false,
            image_view: None,
            image_message_id: None,
            message_thumbnails: HashMap::new(),
            thumbnail_pending: HashSet::new(),
        }
    }

    pub fn set_contacts(&mut self, contacts: Vec<Contact>) {
        self.contacts = contacts
            .into_iter()
            .filter(|c| !c.name.trim().is_empty())
            .map(|c| (c.jid, c.name))
            .collect();
    }

    pub fn set_groups(&mut self, groups: Vec<Group>) {
        self.groups = groups
            .into_iter()
            .filter(|g| !g.name.trim().is_empty())
            .map(|g| (g.jid, g.name))
            .collect();
        cache::save_group_names(&self.groups);
    }

    pub fn set_chat_preview(&mut self, jid: String, preview: String) {
        self.chat_previews.insert(jid, preview);
    }

    /// Resolve a display name for a chat. Groups: subject from `/user/my/groups`
    /// (the chat list often has placeholder `Group <id>` names). Individuals:
    /// contact name -> chat name -> bare jid.
    pub fn chat_display_name(&self, chat: &Chat) -> String {
        if is_group_jid(&chat.jid) {
            if let Some(name) = self.groups.get(&chat.jid) {
                return name.clone();
            }
            if !chat.name.trim().is_empty() && !is_placeholder_group_name(&chat.name, &chat.jid) {
                return chat.name.clone();
            }
            return bare_jid(&chat.jid);
        }
        if let Some(name) = self.contact_name(&chat.jid) {
            return name;
        }
        if !chat.name.trim().is_empty() {
            return chat.name.clone();
        }
        bare_jid(&chat.jid)
    }

    /// Resolve a display name for an arbitrary jid (used for message senders).
    pub fn jid_display_name(&self, jid: &str) -> String {
        self.contact_name(jid).unwrap_or_else(|| bare_jid(jid))
    }

    fn contact_name(&self, jid: &str) -> Option<String> {
        self.contacts.get(jid).cloned()
    }

    pub fn chat_visible(&self, chat: &Chat) -> bool {
        if self.chat_filter.is_empty() {
            return true;
        }
        let q = self.chat_filter.to_lowercase();
        self.chat_display_name(chat).to_lowercase().contains(&q)
            || chat.jid.to_lowercase().contains(&q)
    }

    pub fn visible_chat_indices(&self) -> Vec<usize> {
        self.chats
            .iter()
            .enumerate()
            .filter(|(_, c)| self.chat_visible(c))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn clamp_selection(&mut self) {
        let n = self.visible_chat_indices().len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    /// Whether enough time has passed since the last live chat-list refetch.
    pub fn should_refresh_chats(&mut self, min_gap: std::time::Duration) -> bool {
        let now = Instant::now();
        let ready = self
            .last_chat_refresh
            .map(|t| now.duration_since(t) >= min_gap)
            .unwrap_or(true);
        if ready {
            self.last_chat_refresh = Some(now);
        }
        ready
    }

    pub fn next_messages_generation(&mut self) -> u64 {
        self.messages_generation += 1;
        self.messages_generation
    }

    pub fn next_chats_generation(&mut self) -> u64 {
        self.chats_generation += 1;
        self.chats_generation
    }

    pub fn is_loading(&self) -> bool {
        self.loading_chats
            || self.loading_messages
            || self.loading_older_messages
            || self.loading_image
    }

    pub fn has_older_messages(&self) -> bool {
        match self.messages_total {
            Some(total) => self.messages_fetched < total,
            None => false,
        }
    }

    pub fn spinner_frame(&self) -> &'static str {
        SPINNER[self.spinner % SPINNER.len()]
    }

    pub fn set_devices(&mut self, devices: Vec<Device>) {
        self.device = devices.into_iter().next();
    }

    pub fn set_chats(
        &mut self,
        chats: Vec<Chat>,
        generation: u64,
        show_archived: bool,
    ) {
        if generation < self.active_chats_generation {
            return;
        }
        // Drop responses from a fetch started under a different archived mode.
        if show_archived != self.show_archived {
            return;
        }
        self.active_chats_generation = generation;

        let selected_jid = self.selected_jid();
        self.chats = chats;
        self.loading_chats = false;
        if let Some(jid) = selected_jid {
            let visible = self.visible_chat_indices();
            self.selected = visible
                .iter()
                .position(|&i| self.chats[i].jid == jid)
                .unwrap_or(0);
        }
        self.clamp_selection();
        self.status = if self.show_archived {
            format!("{} archived chats", self.chats.len())
        } else {
            String::new()
        };
    }

    pub fn set_messages(
        &mut self,
        jid: String,
        messages: Vec<Message>,
        generation: u64,
        total: u32,
    ) {
        if self.current_jid.as_deref() != Some(jid.as_str()) {
            return;
        }
        if generation < self.active_messages_generation {
            return;
        }
        self.active_messages_generation = generation;

        let mut messages = messages;
        messages.reverse();

        let pending: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| m.is_pending())
            .cloned()
            .collect();
        for p in pending {
            if !server_has_outgoing(&messages, &p.content) {
                messages.push(p);
            }
        }

        self.messages = messages;
        self.messages_fetched = self.messages.len() as u32;
        self.messages_total = Some(total);
        self.loading_messages = false;
        self.msg_scroll_from_bottom = 0;
        self.sync_media_pick();
        self.prune_message_thumbnails();
    }

    pub fn prepend_older_messages(
        &mut self,
        jid: String,
        older: Vec<Message>,
        generation: u64,
        total: u32,
        offset: u32,
    ) {
        if self.current_jid.as_deref() != Some(jid.as_str()) {
            return;
        }
        if generation < self.active_messages_generation {
            return;
        }
        self.messages_total = Some(total);

        let mut older = older;
        older.reverse();
        let added = older.len();
        for m in older {
            if !self.messages.iter().any(|x| x.id == m.id) {
                self.messages.insert(0, m);
            }
        }
        self.messages_fetched = offset.saturating_add(added as u32);
        self.loading_older_messages = false;
        self.msg_scroll_from_bottom = self
            .msg_scroll_from_bottom
            .saturating_add((added as u16).saturating_mul(2));
        self.prune_message_thumbnails();
    }

    pub fn scroll_messages_page_up(&mut self) {
        self.msg_scroll_from_bottom = self.msg_scroll_from_bottom.saturating_add(8);
    }

    pub fn scroll_messages_page_down(&mut self) {
        self.msg_scroll_from_bottom = self.msg_scroll_from_bottom.saturating_sub(8);
    }

    pub fn scroll_messages_to_top(&mut self) {
        self.msg_scroll_from_bottom = u16::MAX / 2;
    }

    pub fn scroll_messages_to_bottom(&mut self) {
        self.msg_scroll_from_bottom = 0;
    }

    pub fn append_outgoing(&mut self, text: String, sender_jid: &str) {
        self.messages.push(Message::outgoing_pending(text, sender_jid));
        self.msg_scroll_from_bottom = 0;
    }

    pub fn remove_pending_outgoing(&mut self, text: &str) {
        if let Some(idx) = self
            .messages
            .iter()
            .rposition(|m| m.is_pending() && m.content == text)
        {
            self.messages.remove(idx);
        }
    }

    pub fn selected_jid(&self) -> Option<String> {
        self.selected_chat().map(|c| c.jid.clone())
    }

    pub fn selected_chat(&self) -> Option<&Chat> {
        let idx = self.visible_chat_indices().get(self.selected).copied()?;
        self.chats.get(idx)
    }

    pub fn select_next(&mut self) {
        let n = self.visible_chat_indices().len();
        if n > 0 {
            self.selected = (self.selected + 1).min(n - 1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn open_jid(&mut self, jid: String) {
        self.current_jid = Some(jid);
        self.messages.clear();
        self.messages_fetched = 0;
        self.messages_total = None;
        self.msg_scroll_from_bottom = 0;
        self.loading_messages = true;
        self.media_pick = None;
        self.close_image_viewer();
    }

    pub fn viewable_media_indices(&self) -> Vec<usize> {
        self.messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.is_viewable_image())
            .map(|(i, _)| i)
            .collect()
    }

    fn sync_media_pick(&mut self) {
        let indices = self.viewable_media_indices();
        self.media_pick = indices.last().copied();
    }

    pub fn step_media_pick(&mut self, delta: isize) {
        let indices = self.viewable_media_indices();
        if indices.is_empty() {
            self.media_pick = None;
            return;
        }
        let current = self
            .media_pick
            .and_then(|p| indices.iter().position(|&i| i == p))
            .unwrap_or(0);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            (current + delta as usize).min(indices.len() - 1)
        };
        self.media_pick = Some(indices[next]);
    }

    pub fn picked_media_message(&self) -> Option<&Message> {
        let idx = self.media_pick?;
        self.messages.get(idx).filter(|m| m.is_viewable_image())
    }

    pub fn begin_image_view(&mut self, message_id: String) {
        self.loading_image = true;
        self.image_message_id = Some(message_id);
        self.image_view = None;
        self.focus = Focus::ImageViewer;
        self.status = "Loading image…".to_string();
    }

    pub fn set_image_view(&mut self, message_id: String, view: ImageView) {
        if self.image_message_id.as_deref() != Some(message_id.as_str()) {
            return;
        }
        self.image_view = Some(view);
        self.loading_image = false;
        self.status.clear();
    }

    pub fn image_view_failed(&mut self, message_id: String, error: String) {
        if self.image_message_id.as_deref() != Some(message_id.as_str()) {
            return;
        }
        self.loading_image = false;
        self.image_view = None;
        self.status = error;
    }

    pub fn close_image_viewer(&mut self) {
        self.focus = Focus::Chats;
        self.loading_image = false;
        self.image_view = None;
        self.image_message_id = None;
    }

    pub fn message_thumbnail(&self, message_id: &str) -> Option<&DynamicImage> {
        self.message_thumbnails.get(message_id)
    }

    pub fn thumbnail_loading(&self, message_id: &str) -> bool {
        self.thumbnail_pending.contains(message_id)
    }

    /// Messages that still need an inline thumbnail fetch (most recent first).
    pub fn messages_needing_thumbnails(&self, limit: usize) -> Vec<Message> {
        self.messages
            .iter()
            .rev()
            .filter(|m| {
                m.is_viewable_image()
                    && !self.message_thumbnails.contains_key(&m.id)
                    && !self.thumbnail_pending.contains(&m.id)
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn mark_thumbnail_pending(&mut self, message_id: &str) {
        self.thumbnail_pending.insert(message_id.to_string());
    }

    pub fn set_message_thumbnail(&mut self, message_id: String, image: DynamicImage) {
        self.thumbnail_pending.remove(&message_id);
        if self.messages.iter().any(|m| m.id == message_id) {
            self.message_thumbnails.insert(message_id, image);
        }
    }

    pub fn thumbnail_fetch_done(&mut self, message_id: &str) {
        self.thumbnail_pending.remove(message_id);
    }

    fn prune_message_thumbnails(&mut self) {
        let ids: HashSet<&str> = self.messages.iter().map(|m| m.id.as_str()).collect();
        self.message_thumbnails
            .retain(|id, _| ids.contains(id.as_str()));
        self.thumbnail_pending.retain(|id| ids.contains(id.as_str()));
    }

    pub fn on_sent(&mut self) {
        self.status = "Message sent".to_string();
    }

    pub fn toggle_archived(&mut self) {
        self.show_archived = !self.show_archived;
        self.selected = 0;
        self.current_jid = None;
        self.messages.clear();
        self.chat_filter.clear();
        if self.focus == Focus::Search {
            self.focus = Focus::Chats;
        }
        self.loading_chats = true;
        self.status = if self.show_archived {
            "Loading archived chats…".to_string()
        } else {
            "Loading active chats…".to_string()
        };
    }

    pub fn enter_search(&mut self) {
        self.focus = Focus::Search;
    }

    pub fn exit_search(&mut self) {
        self.chat_filter.clear();
        self.focus = Focus::Chats;
        self.clamp_selection();
    }
}

/// Strip the `@server` suffix from a jid, leaving the bare user part.
pub fn bare_jid(jid: &str) -> String {
    jid.split('@').next().unwrap_or(jid).to_string()
}

fn is_group_jid(jid: &str) -> bool {
    jid.ends_with("@g.us")
}

fn is_placeholder_group_name(name: &str, jid: &str) -> bool {
    name == format!("Group {}", bare_jid(jid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_placeholder_group_name() {
        assert!(is_placeholder_group_name(
            "Group 120363423684744408",
            "120363423684744408@g.us"
        ));
        assert!(!is_placeholder_group_name(
            "Phoenix IITD",
            "120363416969378279@g.us"
        ));
    }

    #[test]
    fn resolves_group_subject_from_groups_map() {
        let mut app = App::new();
        app.groups.insert(
            "120363416969378279@g.us".to_string(),
            "Phoenix IITD".to_string(),
        );
        let chat = Chat {
            jid: "120363416969378279@g.us".to_string(),
            name: "Group 120363416969378279".to_string(),
            last_message_time: None,
            archived: false,
        };
        assert_eq!(app.chat_display_name(&chat), "Phoenix IITD");
    }

    #[test]
    fn filters_chats_by_name() {
        let mut app = App::new();
        app.chats = vec![
            Chat {
                jid: "111@s.whatsapp.net".to_string(),
                name: "Alice".to_string(),
                last_message_time: None,
                archived: false,
            },
            Chat {
                jid: "222@s.whatsapp.net".to_string(),
                name: "Bob".to_string(),
                last_message_time: None,
                archived: false,
            },
        ];
        app.chat_filter = "ali".to_string();
        assert_eq!(app.visible_chat_indices(), vec![0]);
    }

    #[test]
    fn drops_stale_or_mismatched_chat_fetch() {
        let mut app = App::new();
        app.chats = vec![Chat {
            jid: "1@s.whatsapp.net".to_string(),
            name: "Alice".to_string(),
            last_message_time: None,
            archived: false,
        }];
        app.active_chats_generation = 2;

        // Older generation is ignored.
        app.set_chats(vec![], 1, false);
        assert_eq!(app.chats.len(), 1);

        // Wrong archived mode is ignored (e.g. archived fetch after toggle back).
        app.set_chats(vec![], 3, true);
        assert_eq!(app.chats.len(), 1);

        app.show_archived = false;
        app.set_chats(
            vec![Chat {
                jid: "2@s.whatsapp.net".to_string(),
                name: "Bob".to_string(),
                last_message_time: None,
                archived: false,
            }],
            3,
            false,
        );
        assert_eq!(app.chats.len(), 1);
        assert_eq!(app.chats[0].jid, "2@s.whatsapp.net");
    }
}

fn server_has_outgoing(messages: &[Message], content: &str) -> bool {
    messages
        .iter()
        .rev()
        .take(3)
        .any(|m| m.is_from_me && m.content == content)
}
