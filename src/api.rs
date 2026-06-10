use anyhow::{anyhow, Result};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;

use crate::config::Config;

/// A single device registered with the WhatsApp gateway.
#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    #[allow(dead_code)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub jid: String,
    #[serde(default)]
    pub state: String,
}

/// A contact as returned by `GET /user/my/contacts`.
#[derive(Debug, Clone, Deserialize)]
pub struct Contact {
    pub jid: String,
    #[serde(default)]
    pub name: String,
}

/// A group as returned by `GET /user/my/groups` (PascalCase fields).
#[derive(Debug, Clone, Deserialize)]
pub struct Group {
    #[serde(rename = "JID")]
    pub jid: String,
    #[serde(rename = "Name", default)]
    pub name: String,
}

/// A chat conversation entry as returned by `GET /chats`.
#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub jid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub last_message_time: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

/// Prefix for client-side optimistic messages not yet confirmed by the API.
pub const PENDING_MSG_PREFIX: &str = "pending:";

/// A single message inside a chat as returned by `GET /chat/{jid}/messages`.
#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: String,
    #[serde(default)]
    pub sender_jid: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub is_from_me: bool,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub url: String,
}

impl Message {
    pub fn is_viewable_image(&self) -> bool {
        matches!(self.media_type.as_str(), "image" | "sticker")
    }

    pub fn is_pending(&self) -> bool {
        self.id.starts_with(PENDING_MSG_PREFIX)
    }

    /// Text shown inside a bubble or as a chat-list preview.
    pub fn body_for_display(&self) -> String {
        let media = media_label(self);
        if !self.content.trim().is_empty() {
            if let Some(label) = media {
                return format!("{label} {}", self.content);
            }
            return self.content.clone();
        }
        media.unwrap_or_else(|| "(no content)".to_string())
    }

    /// One-line preview for the chat list (newlines collapsed).
    pub fn preview_text(&self) -> String {
        self.body_for_display()
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn outgoing_pending(content: String, sender_jid: &str) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: format!("{PENDING_MSG_PREFIX}{now}"),
            sender_jid: sender_jid.to_string(),
            content,
            timestamp: Some(now),
            is_from_me: true,
            media_type: String::new(),
            filename: String::new(),
            url: String::new(),
        }
    }
}

fn media_label(m: &Message) -> Option<String> {
    if m.media_type.is_empty() {
        return None;
    }
    match m.media_type.as_str() {
        "image" => Some("[image]".to_string()),
        "video" | "video_note" => Some("[video]".to_string()),
        "audio" | "ptt" => Some("[audio]".to_string()),
        "sticker" => Some("[sticker]".to_string()),
        "document" => {
            if m.filename.trim().is_empty() {
                Some("[document]".to_string())
            } else {
                Some(format!("[file: {}]", m.filename))
            }
        }
        other => Some(format!("[{other}]")),
    }
}

/// A page of messages plus pagination metadata from the gateway.
#[derive(Debug, Clone)]
pub struct MessagesPage {
    pub messages: Vec<Message>,
    pub total: u32,
    pub offset: u32,
}

#[derive(Debug, Deserialize)]
struct SendResult {
    #[serde(default)]
    message_id: String,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    results: T,
}

#[derive(Debug, Deserialize)]
struct Paginated<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

/// A page of chats plus pagination metadata from the gateway.
#[derive(Debug, Clone)]
pub struct ChatsPage {
    pub chats: Vec<Chat>,
    pub total: u32,
}

#[derive(Debug, Deserialize)]
struct ChatsResults {
    #[serde(default = "Vec::new")]
    data: Vec<Chat>,
    #[serde(default)]
    pagination: PaginationMeta,
}

#[derive(Debug, Default, Deserialize)]
struct PaginationMeta {
    #[serde(default)]
    total: u32,
    #[serde(default)]
    offset: u32,
}

#[derive(Debug, Deserialize)]
struct MessagesResults {
    #[serde(default = "Vec::new")]
    data: Vec<Message>,
    #[serde(default)]
    pagination: PaginationMeta,
}

/// Thin async HTTP client over the gateway REST API.
#[derive(Clone)]
pub struct ApiClient {
    http: Client,
    cfg: Config,
}

impl ApiClient {
    pub fn new(cfg: Config) -> Result<Self> {
        let http = Client::builder()
            .user_agent("whatsatui/0.1")
            .build()?;
        Ok(Self { http, cfg })
    }

    /// Attach basic auth and the optional device-scoping header.
    fn authed(&self, rb: RequestBuilder) -> RequestBuilder {
        let rb = rb.basic_auth(&self.cfg.user, Some(&self.cfg.pass));
        match &self.cfg.device_id {
            Some(id) => rb.header("X-Device-Id", id),
            None => rb,
        }
    }

    pub async fn list_devices(&self) -> Result<Vec<Device>> {
        let url = format!("{}/devices", self.cfg.base_url);
        let resp = self
            .authed(self.http.get(&url))
            .send()
            .await?
            .error_for_status()?;
        let env: Envelope<Vec<Device>> = resp.json().await?;
        Ok(env.results)
    }

    pub async fn list_contacts(&self) -> Result<Vec<Contact>> {
        let url = format!("{}/user/my/contacts", self.cfg.base_url);
        let resp = self
            .authed(self.http.get(&url))
            .send()
            .await?
            .error_for_status()?;
        let env: Envelope<Paginated<Contact>> = resp.json().await?;
        Ok(env.results.data)
    }

    pub async fn list_groups(&self) -> Result<Vec<Group>> {
        let url = format!("{}/user/my/groups", self.cfg.base_url);
        let resp = self
            .authed(self.http.get(&url))
            .send()
            .await?
            .error_for_status()?;
        let env: Envelope<Paginated<Group>> = resp.json().await?;
        Ok(env.results.data)
    }

    /// List chats. Pass `offset` for pagination. The `archived` query param is
    /// optional; whatsatui prefers client-side filtering via `archive.rs`.
    pub async fn list_chats(
        &self,
        limit: u32,
        offset: u32,
        archived: Option<bool>,
    ) -> Result<ChatsPage> {
        let url = format!("{}/chats", self.cfg.base_url);
        let mut query = vec![
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        if let Some(archived) = archived {
            query.push(("archived", archived.to_string()));
        }
        let resp = self
            .authed(self.http.get(&url))
            .query(&query)
            .send()
            .await?
            .error_for_status()?;
        let env: Envelope<ChatsResults> = resp.json().await?;
        Ok(ChatsPage {
            chats: env.results.data,
            total: env.results.pagination.total,
        })
    }

    /// Fetch every chat (paginated) for archived-view filtering.
    pub async fn list_all_chats(&self) -> Result<Vec<Chat>> {
        let mut all = Vec::new();
        let mut offset = 0u32;
        loop {
            let page = self.list_chats(100, offset, None).await?;
            let n = page.chats.len() as u32;
            all.extend(page.chats);
            if n < 100 || offset + n >= page.total {
                break;
            }
            offset += n;
        }
        Ok(all)
    }

    pub async fn get_messages(&self, jid: &str, limit: u32, offset: u32) -> Result<MessagesPage> {
        let url = format!("{}/chat/{}/messages", self.cfg.base_url, jid);
        let resp = self
            .authed(self.http.get(&url))
            .query(&[
                ("limit", limit.to_string()),
                ("offset", offset.to_string()),
            ])
            .send()
            .await?
            .error_for_status()?;
        let env: Envelope<MessagesResults> = resp.json().await?;
        Ok(MessagesPage {
            messages: env.results.data,
            total: env.results.pagination.total,
            offset: env.results.pagination.offset,
        })
    }

    pub async fn send_message(&self, jid: &str, text: &str) -> Result<String> {
        let url = format!("{}/send/message", self.cfg.base_url);
        let body = serde_json::json!({ "phone": jid, "message": text });
        let resp = self.authed(self.http.post(&url)).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            return Err(anyhow!("send failed ({status}): {detail}"));
        }
        let env: Envelope<SendResult> = resp.json().await?;
        Ok(env.results.message_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(content: &str, media_type: &str, filename: &str) -> Message {
        Message {
            id: "1".to_string(),
            sender_jid: String::new(),
            content: content.to_string(),
            timestamp: None,
            is_from_me: false,
            media_type: media_type.to_string(),
            filename: filename.to_string(),
            url: String::new(),
        }
    }

    #[test]
    fn media_label_with_caption() {
        let m = msg("look at this", "image", "pic.jpg");
        assert_eq!(m.body_for_display(), "[image] look at this");
    }

    #[test]
    fn media_label_without_caption() {
        let m = msg("", "sticker", "s.webp");
        assert_eq!(m.body_for_display(), "[sticker]");
    }

    #[test]
    fn video_note_label() {
        let m = msg("", "video_note", "vn.mp4");
        assert_eq!(m.body_for_display(), "[video]");
    }
}
