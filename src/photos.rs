//! Photo input resolution for publish_item.
//!
//! Accepts OpenAI file objects, data: URLs, upload: ticket refs, and https://
//! URLs from allowlisted hosts. All paths normalise bytes to JPEG q90.
use crate::{Error, Result};
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use image::imageops;
use serde_json::Value;
use std::io::Cursor;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::Path;

pub const MAX_PHOTO_BYTES: usize = 15 * 1024 * 1024;
const MAX_PIXELS: u64 = 40_000_000;

/// Host suffixes allowed in https:// fetch (env PHOTO_URL_HOSTS adds to this).
static DEFAULT_ALLOWED_HOSTS: &[&str] =
    &["oaiusercontent.com", "openai.com", "chatgpt.com"];

pub struct PhotoConfig {
    pub upload_dir: std::path::PathBuf,
    /// Extra host suffixes from PHOTO_URL_HOSTS env var.
    pub extra_hosts: Vec<String>,
}

pub struct Photo {
    pub bytes: Vec<u8>,
}

/// Resolve photo_files items → (JPEG bytes, consumed ticket ids).
/// Any failure returns `Err` and no network/filesystem writes are made.
pub fn resolve(inputs: &[Value], cfg: &PhotoConfig) -> Result<(Vec<Photo>, Vec<String>)> {
    let mut photos = Vec::with_capacity(inputs.len());
    let mut ticket_ids = Vec::new();
    let mut errors: Vec<Value> = Vec::new();

    for (i, input) in inputs.iter().enumerate() {
        match resolve_one(input, cfg) {
            Ok((bytes, tid)) => {
                if let Some(id) = tid {
                    ticket_ids.push(id);
                }
                photos.push(Photo { bytes });
            }
            Err(e) => {
                errors.push(serde_json::json!({"index": i, "error": e.code}));
            }
        }
    }
    if !errors.is_empty() {
        return Err(Error::new("photo_resolution_failed").partial(serde_json::json!(errors)));
    }
    Ok((photos, ticket_ids))
}

fn resolve_one(input: &Value, cfg: &PhotoConfig) -> Result<(Vec<u8>, Option<String>)> {
    // OpenAI file object { download_url, file_id, ... }
    if let Some(obj) = input.as_object() {
        let url = obj
            .get("download_url")
            .and_then(Value::as_str)
            .ok_or(Error::new("openai_file_missing_download_url"))?;
        return Ok((fetch_url(url, &cfg.extra_hosts).and_then(|b| normalize(&b))?, None));
    }

    let s = input
        .as_str()
        .ok_or(Error::new("photo_input_must_be_string_or_object"))?;

    if s.starts_with("file_") {
        return Err(Error::new("chatgpt_file_id_alone_cannot_be_downloaded_pass_file_object"));
    }
    if let Some(id) = s.strip_prefix("upload:") {
        let (raw, ticket_id) = read_upload_ticket(id, &cfg.upload_dir)?;
        return Ok((normalize(&raw)?, Some(ticket_id)));
    }
    if s.starts_with("data:") {
        return Ok((normalize(&decode_data_url(s)?)?, None));
    }
    if s.starts_with("https://") || s.starts_with("http://") {
        return Ok((fetch_url(s, &cfg.extra_hosts).and_then(|b| normalize(&b))?, None));
    }
    Err(Error::new("photo_input_unrecognized_format"))
}

// ── data: URL ────────────────────────────────────────────────────────────────

fn decode_data_url(s: &str) -> Result<Vec<u8>> {
    use base64::{Engine, engine::general_purpose::STANDARD};

    let rest = s.strip_prefix("data:").unwrap_or(s);
    let (header, encoded) = rest
        .split_once(',')
        .ok_or(Error::new("data_url_missing_comma"))?;

    if !matches!(
        header,
        "image/jpeg;base64" | "image/png;base64" | "image/webp;base64"
    ) {
        return Err(Error::new("data_url_unsupported_media_type"));
    }
    // Pre-check: base64 adds ~4/3 overhead.
    if encoded.len() > (MAX_PHOTO_BYTES + 2) / 3 * 4 {
        return Err(Error::new("photo_too_large"));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| Error::new("data_url_invalid_base64"))?;
    if bytes.len() > MAX_PHOTO_BYTES {
        return Err(Error::new("photo_too_large"));
    }
    Ok(bytes)
}

// ── SSRF-safe https:// fetch ─────────────────────────────────────────────────

fn extract_host(url: &str) -> Result<String> {
    let after = url
        .strip_prefix("https://")
        .ok_or(Error::new("photo_url_must_be_https"))?;
    // Authority ends at '/', '?', '#' or end-of-string.
    let auth_end = after
        .find(|c| matches!(c, '/' | '?' | '#'))
        .unwrap_or(after.len());
    let authority = &after[..auth_end];

    if authority.contains('@') {
        return Err(Error::new("photo_url_no_userinfo_allowed"));
    }

    // Strip IPv6 brackets, then split host from optional port.
    let (host, port_str) = if authority.starts_with('[') {
        let close = authority
            .rfind(']')
            .ok_or(Error::new("photo_url_invalid"))?;
        let rest = &authority[close + 1..];
        let port = rest.strip_prefix(':').unwrap_or("");
        (&authority[1..close], port)
    } else {
        // Split on the LAST ':' to handle hostnames without confusion.
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, p),
            None => (authority, ""),
        }
    };

    if !port_str.is_empty() && port_str != "443" {
        return Err(Error::new("photo_url_port_not_allowed"));
    }

    Ok(host.to_ascii_lowercase())
}

fn is_allowed_host(host: &str, extra: &[String]) -> bool {
    for &domain in DEFAULT_ALLOWED_HOSTS {
        if host == domain || host.ends_with(&format!(".{domain}")) {
            return true;
        }
    }
    for domain in extra {
        let d = domain.to_ascii_lowercase();
        if host == d || host.ends_with(&format!(".{d}")) {
            return true;
        }
    }
    is_openai_azure_blob(host)
}

fn is_openai_azure_blob(host: &str) -> bool {
    // Pattern: oaisdmntpr[a-z0-9]{1,14}.blob.core.windows.net
    let Some(prefix) = host.strip_suffix(".blob.core.windows.net") else {
        return false;
    };
    let Some(rest) = prefix.strip_prefix("oaisdmntpr") else {
        return false;
    };
    !rest.is_empty() && rest.len() <= 14 && rest.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn is_non_public_ip(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(ip) => {
            let o = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || (o[0] == 100 && (o[1] >> 6) == 1) // CGNAT 100.64.0.0/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 2) // TEST-NET
                || (o[0] == 198 && o[1] == 51 && o[2] == 100) // TEST-NET-2
                || (o[0] == 203 && o[1] == 0 && o[2] == 113) // TEST-NET-3
                || o[0] >= 240 // Reserved 240.0.0.0/4
        }
        IpAddr::V6(ip) => {
            let segs = ip.segments();
            ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unspecified()
                || (segs[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (segs[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
        }
    }
}

fn fetch_url(url: &str, extra_hosts: &[String]) -> Result<Vec<u8>> {
    use std::io::Read;
    use ureq::tls::{TlsConfig, TlsProvider};

    let host = extract_host(url)?;

    if !is_allowed_host(&host, extra_hosts) {
        let safe: String = host
            .chars()
            .take(80)
            .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-')
            .collect();
        return Err(Error {
            partial: Some(serde_json::json!({"host": safe})),
            ..Error::new("photo_url_host_not_allowed")
        });
    }

    // Resolve DNS and reject private IPs (SSRF guard).
    let addrs: Vec<_> = format!("{host}:443")
        .to_socket_addrs()
        .map_err(|_| Error::new("photo_url_dns_failed"))?
        .collect();
    if addrs.is_empty() {
        return Err(Error::new("photo_url_dns_failed"));
    }
    for addr in &addrs {
        if is_non_public_ip(addr.ip()) {
            return Err(Error::new("photo_url_resolves_to_private_ip"));
        }
    }

    // Fetch with no redirects and 20 s timeout.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .into();

    let mut resp = agent
        .get(url)
        .call()
        .map_err(|_| Error::new("photo_url_fetch_failed"))?;

    if resp.status() != 200 {
        return Err(Error::new("photo_url_fetch_failed"));
    }

    let mut buf = Vec::new();
    resp.body_mut()
        .as_reader()
        .take((MAX_PHOTO_BYTES + 1) as u64)
        .read_to_end(&mut buf)
        .map_err(|_| Error::new("photo_url_fetch_failed"))?;
    if buf.len() > MAX_PHOTO_BYTES {
        return Err(Error::new("photo_too_large"));
    }
    Ok(buf)
}

// ── upload: ticket reader ────────────────────────────────────────────────────

fn read_upload_ticket(id: &str, upload_dir: &Path) -> Result<(Vec<u8>, String)> {
    // Strictly validate id to prevent path traversal.
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::new("upload_ticket_id_invalid"));
    }
    let meta_path = upload_dir.join(format!("{id}.meta"));
    let bin_path = upload_dir.join(format!("{id}.bin"));
    let consumed_path = upload_dir.join(format!("{id}.consumed"));

    if !meta_path.exists() {
        return Err(Error::new("upload_ticket_not_found"));
    }
    if !bin_path.exists() {
        return Err(Error::new("upload_ticket_not_uploaded_yet"));
    }
    if consumed_path.exists() {
        return Err(Error::new("upload_ticket_already_consumed"));
    }

    let bytes = std::fs::read(&bin_path).map_err(|_| Error::new("upload_ticket_read_failed"))?;
    if bytes.len() > MAX_PHOTO_BYTES {
        return Err(Error::new("photo_too_large"));
    }
    Ok((bytes, id.to_owned()))
}

/// Mark tickets consumed and remove .bin files after a successful publish.
pub fn consume_tickets(ids: &[String], upload_dir: &Path) {
    for id in ids {
        let bin = upload_dir.join(format!("{id}.bin"));
        let consumed = upload_dir.join(format!("{id}.consumed"));
        let _ = std::fs::write(&consumed, b"");
        let _ = std::fs::remove_file(&bin);
    }
}

// ── Image normalisation ──────────────────────────────────────────────────────

/// Decode → validate format/pixels → EXIF transpose → RGB → JPEG q90.
pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > MAX_PHOTO_BYTES {
        return Err(Error::new("photo_too_large"));
    }
    check_format(bytes)?;
    let img = image::load_from_memory(bytes).map_err(|_| Error::new("photo_decode_failed"))?;
    if img.width() as u64 * img.height() as u64 > MAX_PIXELS {
        return Err(Error::new("photo_exceeds_40_million_pixels"));
    }
    let img = exif_transpose(bytes, img);
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|_| Error::new("photo_encode_failed"))?;
    Ok(out)
}

fn check_format(bytes: &[u8]) -> Result<()> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(());
    }
    if bytes.len() >= 8 && bytes.starts_with(b"\x89PNG") {
        return Ok(());
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes[8..12] == *b"WEBP" {
        return Ok(());
    }
    Err(Error::new("photo_must_be_jpeg_png_or_webp"))
}

fn exif_transpose(bytes: &[u8], img: DynamicImage) -> DynamicImage {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return img; // Not JPEG; PNG/WebP carry no EXIF orientation.
    }
    let orientation: u32 = exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()
        .and_then(|e| {
            e.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| match &f.value {
                    exif::Value::Short(v) => v.first().map(|&n| n as u32),
                    _ => None,
                })
        })
        .unwrap_or(1);

    let rgb = img.to_rgb8();
    let rotated = match orientation {
        2 => imageops::flip_horizontal(&rgb),
        3 => imageops::rotate180(&rgb),
        4 => imageops::flip_vertical(&rgb),
        5 => imageops::flip_horizontal(&imageops::rotate270(&rgb)),
        6 => imageops::rotate90(&rgb),
        7 => imageops::flip_horizontal(&imageops::rotate90(&rgb)),
        8 => imageops::rotate270(&rgb),
        _ => return DynamicImage::ImageRgb8(rgb),
    };
    DynamicImage::ImageRgb8(rotated)
}
