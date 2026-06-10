//! Read-only connectivity probe: verifies the API client and serde models
//! against a live gateway. It only performs GET requests and never sends a
//! message. Message contents are intentionally NOT printed.

use std::path::Path;

use whatsatui::api::ApiClient;
use whatsatui::archive;
use whatsatui::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env();
    println!("base_url: {}", cfg.base_url);
    println!("whatsmeow_db: {}", cfg.whatsmeow_db);

    let client = ApiClient::new(cfg.clone())?;

    let devices = client.list_devices().await?;
    println!("devices parsed: {}", devices.len());
    if let Some(d) = devices.first() {
        println!("  first device state: {}", d.state);
    }

    let contacts = client.list_contacts().await?;
    let named = contacts.iter().filter(|c| !c.name.trim().is_empty()).count();
    println!("contacts parsed: {} (named: {})", contacts.len(), named);

    let groups = client.list_groups().await?;
    let named_groups = groups.iter().filter(|g| !g.name.trim().is_empty()).count();
    println!("groups parsed: {} (named: {})", groups.len(), named_groups);

    let archived_jids = archive::load_archived_jids(Path::new(&cfg.whatsmeow_db));
    println!(
        "archived jids (whatsmeow): {} (db exists: {})",
        archived_jids.len(),
        Path::new(&cfg.whatsmeow_db).exists()
    );

    let page = client.list_chats(10, 0, None).await?;
    let active: Vec<_> = page
        .chats
        .iter()
        .filter(|c| !archived_jids.contains(&c.jid))
        .collect();
    println!(
        "chats parsed (first page, active after filter): {} / {}",
        active.len(),
        page.chats.len()
    );

    if let Some(first) = active.first() {
        let page = client.get_messages(&first.jid, 5, 0).await?;
        println!(
            "messages parsed in first chat: {} (total: {})",
            page.messages.len(),
            page.total
        );
        let from_me = page.messages.iter().filter(|m| m.is_from_me).count();
        println!("  (of which outgoing: {})", from_me);
        if let Some(m) = page.messages.first() {
            println!("  latest preview: {}", m.preview_text());
        }
    }

    if let Ok(jid) = std::env::var("WHATSATUI_PROBE_JID") {
        let page = client.get_messages(&jid, 5, 0).await?;
        println!("messages parsed in target chat: {}", page.messages.len());
        let with_media = page
            .messages
            .iter()
            .filter(|m| !m.media_type.is_empty())
            .count();
        println!("  (of which media: {})", with_media);
    }

    println!("OK - read-only probe complete, no messages sent");
    Ok(())
}
