use std::env;

/// Runtime configuration, sourced from environment variables with defaults that
/// match the local `go-whatsapp-web-multidevice` deployment in this repo.
#[derive(Clone, Debug)]
pub struct Config {
    pub base_url: String,
    pub user: String,
    pub pass: String,
    pub device_id: Option<String>,
    pub webhook_addr: String,
    pub webhook_path: String,
    pub webhook_secret: String,
    /// Path to the gateway's whatsmeow sqlite DB (for real archived-chat state).
    pub whatsmeow_db: String,
    /// Path to the gateway's chatstorage sqlite DB (media keys for image download).
    pub chatstorage_db: String,
}

impl Config {
    pub fn from_env() -> Self {
        let base_url = env::var("WHATSATUI_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:56310".to_string())
            .trim_end_matches('/')
            .to_string();
        let user = env::var("WHATSATUI_USER").unwrap_or_else(|_| "admin".to_string());
        let pass = env::var("WHATSATUI_PASS").unwrap_or_else(|_| "changeme".to_string());
        let device_id = env::var("WHATSATUI_DEVICE_ID")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let webhook_addr =
            env::var("WHATSATUI_WEBHOOK_ADDR").unwrap_or_else(|_| "127.0.0.1:56311".to_string());
        let webhook_path = {
            let p = env::var("WHATSATUI_WEBHOOK_PATH").unwrap_or_else(|_| "/webhook".to_string());
            if p.starts_with('/') {
                p
            } else {
                format!("/{p}")
            }
        };
        let webhook_secret =
            env::var("WHATSATUI_WEBHOOK_SECRET").unwrap_or_else(|_| "secret".to_string());
        let whatsmeow_db = env::var("WHATSATUI_WHATSMEOW_DB")
            .unwrap_or_else(|_| "storages/whatsapp.db".to_string());
        let chatstorage_db = env::var("WHATSATUI_CHATSTORAGE_DB")
            .unwrap_or_else(|_| "storages/chatstorage.db".to_string());

        Self {
            base_url,
            user,
            pass,
            device_id,
            webhook_addr,
            webhook_path,
            webhook_secret,
            whatsmeow_db,
            chatstorage_db,
        }
    }
}
