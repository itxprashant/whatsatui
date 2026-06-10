use std::path::Path;

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use anyhow::{anyhow, Context, Result};
use hmac::{Hmac, KeyInit, Mac};
use reqwest::Client;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

const WA_ORIGIN: &str = "https://web.whatsapp.com";
const MEDIA_HMAC_LEN: usize = 10;

/// Encryption metadata for a media message, read from chatstorage.
#[derive(Debug, Clone)]
pub struct MediaRow {
    pub url: String,
    pub media_key: Vec<u8>,
    pub file_sha256: Vec<u8>,
    pub file_enc_sha256: Vec<u8>,
    pub media_type: String,
}

impl MediaRow {
    pub fn hkdf_info(&self) -> &'static [u8] {
        match self.media_type.as_str() {
            "video" | "video_note" => b"WhatsApp Video Keys",
            "audio" | "ptt" => b"WhatsApp Audio Keys",
            "document" => b"WhatsApp Document Keys",
            // image and sticker both use image keys in whatsmeow
            _ => b"WhatsApp Image Keys",
        }
    }
}

/// Load downloadable media fields for a message from the gateway chatstorage DB.
pub fn load_media_row(db_path: &Path, message_id: &str, chat_jid: &str) -> Option<MediaRow> {
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    let mut stmt = conn
        .prepare(
            "SELECT url, media_key, file_sha256, file_enc_sha256, media_type
             FROM messages WHERE id = ?1 AND chat_jid = ?2 LIMIT 1",
        )
        .ok()?;
    let row = stmt
        .query_row([message_id, chat_jid], |row| {
            Ok(MediaRow {
                url: row.get(0)?,
                media_key: row.get(1)?,
                file_sha256: row.get(2)?,
                file_enc_sha256: row.get(3)?,
                media_type: row.get(4)?,
            })
        })
        .ok()?;
    if row.url.trim().is_empty() || row.media_key.len() != 32 {
        return None;
    }
    Some(row)
}

/// Download and decrypt WhatsApp-hosted media (same algorithm as whatsmeow).
pub async fn download_decrypted(http: &Client, row: &MediaRow) -> Result<Vec<u8>> {
    let resp = http
        .get(&row.url)
        .header("Origin", WA_ORIGIN)
        .header("Referer", format!("{WA_ORIGIN}/"))
        .send()
        .await
        .context("media GET")?
        .error_for_status()
        .context("media HTTP status")?;
    let data = resp.bytes().await.context("media body")?;

    if data.len() <= MEDIA_HMAC_LEN {
        return Err(anyhow!("encrypted media too short"));
    }
    if row.file_enc_sha256.len() == 32 {
        let digest = Sha256::digest(&data);
        if digest.as_slice() != row.file_enc_sha256.as_slice() {
            return Err(anyhow!("encrypted media checksum mismatch"));
        }
    }

    let (iv, cipher_key, mac_key) = expand_media_keys(&row.media_key, row.hkdf_info());
    let file_part = &data[..data.len() - MEDIA_HMAC_LEN];
    let mac = &data[data.len() - MEDIA_HMAC_LEN..];

    let mut mac_checker = HmacSha256::new_from_slice(&mac_key).context("mac key")?;
    mac_checker.update(&iv);
    mac_checker.update(file_part);
    let expected_mac = &mac_checker.finalize().into_bytes()[..MEDIA_HMAC_LEN];
    if expected_mac != mac {
        return Err(anyhow!("media HMAC mismatch"));
    }

    let mut buf = file_part.to_vec();
    let plain = Aes256CbcDec::new_from_slices(&cipher_key, &iv)
        .map_err(|e| anyhow!("AES key: {e}"))?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow!("AES decrypt: {e}"))?;

    if row.file_sha256.len() == 32 {
        let digest = Sha256::digest(plain);
        if digest.as_slice() != row.file_sha256.as_slice() {
            return Err(anyhow!("decrypted media checksum mismatch"));
        }
    }

    Ok(plain.to_vec())
}

/// Matches golang.org/x/crypto/hkdf with SHA-256 (whatsmeow hkdfutil.SHA256).
fn expand_media_keys(media_key: &[u8], info: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let expanded = hkdf_sha256(media_key, info, 112);
    (
        expanded[0..16].to_vec(),
        expanded[16..48].to_vec(),
        expanded[48..80].to_vec(),
    )
}

fn hkdf_sha256(ikm: &[u8], info: &[u8], out_len: usize) -> Vec<u8> {
    let zero_salt = [0u8; 32];
    let mut hasher = HmacSha256::new_from_slice(&zero_salt).expect("salt len");
    hasher.update(ikm);
    let prk = hasher.finalize().into_bytes();

    let mut okm = vec![0u8; out_len];
    let mut t = Vec::new();
    let mut offset = 0usize;
    let mut block = 1u8;
    while offset < out_len {
        let mut hasher = HmacSha256::new_from_slice(&prk).expect("prk len");
        if block > 1 {
            hasher.update(&t);
        }
        hasher.update(info);
        hasher.update(&[block]);
        t = hasher.finalize().into_bytes().to_vec();
        let take = (out_len - offset).min(t.len());
        okm[offset..offset + take].copy_from_slice(&t[..take]);
        offset += take;
        block += 1;
    }
    okm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hkdf_splits_into_iv_cipher_mac() {
        let key = [1u8; 32];
        let (iv, ck, mk) = expand_media_keys(&key, b"WhatsApp Image Keys");
        assert_eq!(iv.len(), 16);
        assert_eq!(ck.len(), 32);
        assert_eq!(mk.len(), 32);
    }
}
