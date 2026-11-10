//! Upload ticket creation and GC for the photo upload flow.
use crate::{Error, Result};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use serde_json::{Value, json};
use std::path::Path;

type HmacSha256 = Hmac<Sha256>;

pub struct UploadConfig {
    pub upload_dir: std::path::PathBuf,
    pub public_base_url: String,
    pub secret: Vec<u8>,
    pub ttl_seconds: u64,
}

pub fn create_tickets(count: usize, cfg: &UploadConfig) -> Result<Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::new("system_time_error"))?
        .as_secs();
    let exp = now + cfg.ttl_seconds;
    let mut tickets = Vec::with_capacity(count);
    for _ in 0..count {
        let id = random_hex(32)?;
        let sig = sign_ticket(&id, exp, &cfg.secret)?;
        let meta_path = cfg.upload_dir.join(format!("{id}.meta"));
        let meta = json!({"id": id, "exp": exp, "sig": sig});
        std::fs::write(&meta_path, meta.to_string())
            .map_err(|_| Error::new("upload_ticket_write_failed"))?;
        let upload_url = format!(
            "{}/upload/{}?exp={}&sig={}",
            cfg.public_base_url.trim_end_matches('/'),
            id,
            exp,
            sig
        );
        tickets.push(json!({
            "ref": format!("upload:{id}"),
            "upload_url": upload_url,
            "expires_at": exp
        }));
    }
    Ok(Value::Array(tickets))
}

pub fn sign_ticket(id: &str, exp: u64, secret: &[u8]) -> Result<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| Error::new("hmac_key_error"))?;
    mac.update(format!("{id}.{exp}").as_bytes());
    let result = mac.finalize().into_bytes();
    Ok(URL_SAFE_NO_PAD.encode(result))
}

fn random_hex(len: usize) -> Result<String> {
    use std::fmt::Write;
    let mut bytes = vec![0u8; len];
    getrandom::getrandom(&mut bytes).map_err(|_| Error::new("random_unavailable"))?;
    Ok(bytes.iter().fold(String::with_capacity(len * 2), |mut s, b| {
        write!(s, "{b:02x}").unwrap();
        s
    }))
}

/// Delete upload files older than `max_age_secs`. Runs at startup and on a 10-min loop.
pub fn gc_uploads(upload_dir: &Path, max_age_secs: u64) {
    let Ok(entries) = std::fs::read_dir(upload_dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(max_age_secs))
        .unwrap_or(std::time::UNIX_EPOCH);
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "meta" | "bin" | "consumed") {
            continue;
        }
        if let Ok(metadata) = entry.metadata() {
            if let Ok(modified) = metadata.modified() {
                if modified < cutoff {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}
