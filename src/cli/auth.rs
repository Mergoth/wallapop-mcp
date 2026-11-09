use crate::{Error, Result};
use serde_json::{Value, json};
use std::io::Read;

pub fn cookies(s: &str) -> Result<Value> {
    if s.trim_start().starts_with('{') {
        return serde_json::from_str(s).map_err(|_| Error::new("invalid_session_json"));
    }
    let mut out = json!({"cookies":{}});
    for line in s.lines() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line).trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() >= 7 {
            if matches!(
                fields[0].trim_start_matches('.'),
                "wallapop.com" | "es.wallapop.com"
            ) {
                out["cookies"][fields[5]] = fields[6].into();
            }
        } else {
            for part in line.split(';') {
                if let Some((k, v)) = part.trim().split_once('=') {
                    out["cookies"][k.trim()] = v.trim().into();
                }
            }
        }
    }
    Ok(out)
}

pub(super) fn stdin_text() -> Result<String> {
    let mut s = String::new();
    std::io::stdin()
        .take(65537)
        .read_to_string(&mut s)
        .map_err(|_| Error::new("stdin_failed"))?;
    if s.len() > 65536 {
        return Err(Error::new("stdin_too_large"));
    }
    Ok(s)
}
