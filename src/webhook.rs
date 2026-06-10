use std::thread;

use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::config::Config;

type HmacSha256 = Hmac<Sha256>;

/// Envelope the gateway POSTs to the webhook: `{ event, device_id, payload }`.
#[derive(Debug, Deserialize)]
struct WebhookEnvelope {
    #[serde(default)]
    event: String,
    #[serde(default)]
    payload: WebhookPayload,
}

#[derive(Debug, Default, Deserialize)]
struct WebhookPayload {
    #[serde(default)]
    chat_id: String,
    #[serde(default)]
    is_from_me: bool,
}

/// Verify the gateway's `X-Hub-Signature-256` header against the raw body.
/// The header has the form `sha256=<hex>` where `<hex>` is the HMAC-SHA256 of
/// the raw body keyed by the shared secret. Comparison is constant-time.
pub fn verify_signature(secret: &str, body: &[u8], header: &str) -> bool {
    let hex_sig = header.strip_prefix("sha256=").unwrap_or(header);
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

/// Start the webhook receiver on a dedicated OS thread. Parsed, signature-valid
/// events are forwarded to the main loop as `AppEvent::Webhook`. Returns an
/// error if the listener cannot bind (caller decides how to surface it).
pub fn spawn(cfg: &Config, tx: UnboundedSender<AppEvent>) -> std::io::Result<()> {
    let server = tiny_http::Server::http(&cfg.webhook_addr)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let path = cfg.webhook_path.clone();
    let secret = cfg.webhook_secret.clone();

    thread::spawn(move || {
        for mut request in server.incoming_requests() {
            // Only accept POSTs to the configured path.
            let url_path = request.url().split('?').next().unwrap_or("").to_string();
            let is_post = matches!(request.method(), tiny_http::Method::Post);
            if !is_post || url_path != path {
                let _ = request.respond(tiny_http::Response::empty(404));
                continue;
            }

            let signature = request
                .headers()
                .iter()
                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("X-Hub-Signature-256"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default();

            let mut body = Vec::new();
            if request.as_reader().read_to_end(&mut body).is_err() {
                let _ = request.respond(tiny_http::Response::empty(400));
                continue;
            }

            if !verify_signature(&secret, &body, &signature) {
                let _ = request.respond(tiny_http::Response::empty(401));
                continue;
            }

            if let Ok(env) = serde_json::from_slice::<WebhookEnvelope>(&body) {
                let chat_id = if env.payload.chat_id.is_empty() {
                    None
                } else {
                    Some(env.payload.chat_id)
                };
                let _ = tx.send(AppEvent::Webhook {
                    event: env.event,
                    chat_id,
                    is_from_me: env.payload.is_from_me,
                });
            }

            let _ = request.respond(tiny_http::Response::empty(200));
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // HMAC-SHA256("hello world", "secret") precomputed:
    const BODY: &[u8] = b"hello world";
    const SECRET: &str = "secret";
    const VALID_HEX: &str = "734cc62f32841568f45715aeb9f4d7891324e6d948e4c6c60c0621cdac48623a";

    #[test]
    fn accepts_valid_signature_with_prefix() {
        let header = format!("sha256={VALID_HEX}");
        assert!(verify_signature(SECRET, BODY, &header));
    }

    #[test]
    fn accepts_valid_signature_without_prefix() {
        assert!(verify_signature(SECRET, BODY, VALID_HEX));
    }

    #[test]
    fn rejects_tampered_body() {
        let header = format!("sha256={VALID_HEX}");
        assert!(!verify_signature(SECRET, b"hello worlds", &header));
    }

    #[test]
    fn rejects_wrong_secret() {
        let header = format!("sha256={VALID_HEX}");
        assert!(!verify_signature("wrong", BODY, &header));
    }

    #[test]
    fn rejects_garbage_header() {
        assert!(!verify_signature(SECRET, BODY, "not-hex"));
    }
}
