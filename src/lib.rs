pub mod cli;
pub mod client;
pub mod photos;
pub mod protocol;
pub mod tools;
pub mod transport;
pub mod upload;

use serde_json::{Value, json};
use std::{
    fmt::{self, Write as FmtWrite},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
pub struct Error {
    pub code: &'static str,
    pub status: Option<u16>,
    pub unknown: bool,
    pub partial: Option<Value>,
    /// Safe fields extracted from an upstream error body (no tokens, trimmed to 2 KB).
    pub detail: Option<Value>,
}

impl Error {
    pub fn new(code: &'static str) -> Self {
        Self {
            code,
            status: None,
            unknown: false,
            partial: None,
            detail: None,
        }
    }
    pub fn unknown(mut self) -> Self {
        self.unknown = true;
        self
    }
    pub fn partial(mut self, value: Value) -> Self {
        self.partial = Some(value);
        self
    }
    pub fn field(mut self, name: &'static str) -> Self {
        self.detail = Some(json!({"field": name}));
        self
    }
    pub fn value(&self) -> Value {
        let mut e =
            json!({"code":self.code,"status":self.status,"outcome_unknown":self.unknown});
        if let Some(d) = &self.detail {
            e["detail"] = d.clone();
        }
        let mut v = json!({"error": e});
        if let Some(p) = &self.partial {
            v["partial_result"] = p.clone();
        }
        v
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code)?;
        if let Some(status) = self.status {
            write!(f, " (HTTP {status})")?;
        }
        if self.unknown {
            write!(f, "; outcome unknown; check state before retrying")?;
        }
        Ok(())
    }
}
impl std::error::Error for Error {}

#[derive(Clone)]
pub struct Context {
    pub cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
impl Context {
    pub fn new(timeout: Duration) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + timeout,
        }
    }
    pub fn remaining(&self) -> Result<Duration> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(Error::new("cancelled"));
        }
        self.deadline
            .checked_duration_since(Instant::now())
            .ok_or(Error::new("timeout"))
    }
    pub fn sleep(&self, duration: Duration) -> Result<()> {
        let end = Instant::now() + duration.min(self.remaining()?);
        while Instant::now() < end {
            std::thread::sleep(
                end.saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(20)),
            );
            self.remaining()?;
        }
        Ok(())
    }
}

pub fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
pub fn first(v: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .find_map(|k| v.get(*k).filter(|v| !v.is_null()).cloned())
        .unwrap_or(Value::Null)
}
pub fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m.get(key).or_else(|| m.values().find_map(|v| find(v, key))),
        Value::Array(a) => a.iter().find_map(|v| find(v, key)),
        _ => None,
    }
}
pub fn pick(v: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|k| v.get(*k).map(|v| (k.to_string(), v.clone())))
            .collect(),
    )
}
pub fn identifier(s: &str) -> Result<&str> {
    if s.is_empty()
        || s.len() > 180
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Error::new("invalid_identifier"));
    }
    Ok(s)
}
/// Validate decimal currency without floating point, returning integer cents.
pub fn cents(s: &str) -> Result<u64> {
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    if whole.is_empty()
        || whole.len() > 8
        || (whole.len() > 1 && whole.starts_with('0'))
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (s.contains('.') && fraction.is_empty())
    {
        return Err(Error::new("invalid_price"));
    }
    Ok(whole
        .parse::<u64>()
        .map_err(|_| Error::new("invalid_price"))?
        * 100
        + fraction.parse::<u64>().unwrap_or(0) * if fraction.len() == 1 { 10 } else { 1 })
}
pub fn money(v: &Value) -> Result<Value> {
    let s = v
        .as_str()
        .ok_or(Error::new("price_must_be_decimal_string"))?;
    if cents(s)? == 0 {
        return Err(Error::new("price_must_be_positive"));
    }
    serde_json::from_str(s).map_err(|_| Error::new("invalid_price"))
}
pub fn uuid() -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).map_err(|_| Error::new("random_unavailable"))?;
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    let h = b.iter().fold(String::with_capacity(32), |mut s, b| {
        write!(s, "{b:02x}").unwrap();
        s
    });
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}
