//! Local, offline check of the webhook receiver path. It starts the same
//! `webhook::spawn` server the TUI uses, then POSTs a locally crafted, correctly
//! signed sample `message` event and a tampered one. It confirms a valid
//! signature is accepted (200) and forwarded as an `AppEvent::Webhook`, and a
//! bad signature is rejected (401). No WhatsApp message is ever sent.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tokio::sync::mpsc;
use whatsatui::app::AppEvent;
use whatsatui::config::Config;
use whatsatui::webhook;

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Use a dedicated port so this never clashes with a running TUI.
    let mut cfg = Config::from_env();
    cfg.webhook_addr = "127.0.0.1:56399".to_string();
    cfg.webhook_path = "/webhook".to_string();
    cfg.webhook_secret = "secret".to_string();

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    webhook::spawn(&cfg, tx)?;

    // Give the server thread a moment to bind.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let url = format!("http://{}{}", cfg.webhook_addr, cfg.webhook_path);
    let http = reqwest::Client::new();

    let body = serde_json::to_vec(&serde_json::json!({
        "event": "message",
        "device_id": "probe",
        "payload": { "chat_id": "123@s.whatsapp.net", "is_from_me": false }
    }))?;

    // 1) Valid signature -> accepted.
    let good = http
        .post(&url)
        .header("X-Hub-Signature-256", sign(&cfg.webhook_secret, &body))
        .body(body.clone())
        .send()
        .await?;
    println!("valid signature -> HTTP {}", good.status().as_u16());
    assert_eq!(good.status().as_u16(), 200, "valid POST should be accepted");

    // 2) Tampered/wrong signature -> rejected.
    let bad = http
        .post(&url)
        .header("X-Hub-Signature-256", sign("wrong-secret", &body))
        .body(body.clone())
        .send()
        .await?;
    println!("bad signature   -> HTTP {}", bad.status().as_u16());
    assert_eq!(bad.status().as_u16(), 401, "bad signature should be rejected");

    // 3) The valid event should have been forwarded exactly once.
    let ev = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await?
        .expect("expected a forwarded event");
    match ev {
        AppEvent::Webhook { event, chat_id, is_from_me } => {
            println!(
                "forwarded: event={event} chat_id={chat_id:?} is_from_me={is_from_me}"
            );
            assert_eq!(event, "message");
            assert_eq!(chat_id.as_deref(), Some("123@s.whatsapp.net"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "bad signature must not forward an event");

    println!("OK - webhook receiver verified (valid accepted, bad rejected, no messages sent)");
    Ok(())
}
