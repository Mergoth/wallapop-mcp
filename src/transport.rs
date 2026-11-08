use crate::{Context, Error, Result};
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant, SystemTime},
};
use ureq::tls::{TlsConfig, TlsProvider};

pub const MAX_BODY: usize = 4 << 20;
const MAX_CACHE: usize = 8 << 20;
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}
struct Entry {
    key: String,
    until: Instant,
    response: Response,
}
#[derive(Default)]
struct State {
    cache: VecDeque<Entry>,
    bytes: usize,
    generation: u64,
}
pub struct Transport {
    agent: ureq::Agent,
    writes: ureq::Agent,
    interval: Duration,
    next: Mutex<Instant>,
    state: Mutex<State>,
    flights: Mutex<HashMap<String, Weak<Mutex<()>>>>,
}
pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(String, String)],
    pub body: &'a [u8],
    pub read: bool,
    pub ttl: Duration,
}
impl Transport {
    pub fn new(interval: Duration) -> Self {
        let tls = || {
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .build()
        };
        let config = || {
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .max_redirects(0)
                .timeout_connect(Some(Duration::from_secs(10)))
                .timeout_per_read(Some(Duration::from_secs(30)))
                .tls_config(tls())
        };
        Self {
            agent: config().build().into(),
            writes: config().max_idle_connections(0).build().into(),
            interval,
            next: Mutex::new(Instant::now()),
            state: Mutex::default(),
            flights: Mutex::default(),
        }
    }
    pub fn invalidate(&self) {
        let mut s = self.state.lock().unwrap();
        s.cache.clear();
        s.bytes = 0;
        s.generation += 1;
    }
    pub fn request(&self, ctx: &Context, r: Request<'_>) -> Result<Response> {
        ctx.remaining()?;
        // Uploads and fresh reads never need cache keys or coalescing gates.
        // Formatting a photo body here would multiply its memory footprint.
        if !r.read || r.ttl.is_zero() {
            return self.perform(ctx, &r);
        }
        // The entire header set, including credentials, isolates read cache keys.
        let key = format!("{}\0{}\0{:?}\0{:?}", r.method, r.url, r.headers, r.body);
        let generation = self.state.lock().unwrap().generation;
        let flight_key = format!("{generation}\0{key}");
        let gate = {
            let mut flights = self.flights.lock().unwrap();
            flights.retain(|_, v| v.strong_count() > 0);
            let gate = flights
                .get(&flight_key)
                .and_then(Weak::upgrade)
                .unwrap_or_default();
            flights.insert(flight_key, Arc::downgrade(&gate));
            gate
        };
        let _guard = loop {
            if let Ok(lock) = gate.try_lock() {
                break lock;
            }
            ctx.sleep(Duration::from_millis(10))?;
        };
        {
            let mut s = self.state.lock().unwrap();
            if let Some(i) = s
                .cache
                .iter()
                .position(|e| e.key == key && e.until > Instant::now())
            {
                let entry = s.cache.remove(i).unwrap();
                let result = entry.response.clone();
                s.cache.push_back(entry);
                return Ok(result);
            }
        }
        let result = self.perform(ctx, &r)?;
        {
            let mut s = self.state.lock().unwrap();
            if s.generation == generation && result.body.len() <= MAX_CACHE {
                // Count the request key too, so cache memory stays bounded.
                let size = key.len() + result.body.len();
                while !s.cache.is_empty() && (s.cache.len() >= 64 || s.bytes + size > MAX_CACHE) {
                    let e = s.cache.pop_front().unwrap();
                    s.bytes -= e.key.len() + e.response.body.len();
                }
                if size <= MAX_CACHE {
                    s.bytes += size;
                    s.cache.push_back(Entry {
                        key,
                        until: Instant::now() + r.ttl,
                        response: result.clone(),
                    });
                }
            }
        }
        Ok(result)
    }
    fn pace(&self, ctx: &Context) -> Result<()> {
        let mut next = loop {
            if let Ok(g) = self.next.try_lock() {
                break g;
            }
            ctx.sleep(Duration::from_millis(10))?;
        };
        ctx.sleep(next.saturating_duration_since(Instant::now()))?;
        *next = Instant::now() + self.interval;
        Ok(())
    }
    fn perform(&self, ctx: &Context, r: &Request<'_>) -> Result<Response> {
        for attempt in 0..if r.read { 3 } else { 1 } {
            self.pace(ctx)?;
            let agent = if r.read { &self.agent } else { &self.writes };
            let mut req = ureq::http::Request::builder().method(r.method).uri(r.url);
            for (k, v) in r.headers {
                req = req.header(k, v);
            }
            // A fresh connection and no automatic redirects prevent replay of
            // mutations, including PubNub's write-shaped GET endpoint.
            if !r.read {
                req = req.header("Connection", "close");
            }
            let req = req
                .body(r.body)
                .map_err(|_| Error::new("invalid_request"))?;
            let req = agent
                .configure_request(req)
                .timeout_global(Some(ctx.remaining()?))
                .build();
            let response = agent.run(req);
            match response {
                Ok(mut response) => {
                    let status = response.status().as_u16();
                    let delay = retry_after(
                        response
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok()),
                    );
                    let mut body = Vec::new();
                    let read_error = response
                        .body_mut()
                        .as_reader()
                        .take((MAX_BODY + 1) as u64)
                        .read_to_end(&mut body);
                    if let Err(source) = read_error {
                        if r.read && attempt < 2 && transient_io(&source) {
                            ctx.sleep(Duration::from_millis(200 << attempt))?;
                            continue;
                        }
                        let mut e = ctx
                            .remaining()
                            .err()
                            .unwrap_or(Error::new("response_read_failed"));
                        e.unknown = !r.read;
                        return Err(e);
                    }
                    if body.len() > MAX_BODY {
                        let mut e = Error::new("response_too_large");
                        e.unknown = !r.read;
                        return Err(e);
                    }
                    if (200..300).contains(&status) {
                        return Ok(Response { status, body });
                    }
                    if r.read && attempt < 2 && matches!(status, 429 | 502 | 503 | 504) {
                        ctx.sleep(delay.max(Duration::from_millis(200 << attempt)))?;
                        continue;
                    }
                    return Err(Error {
                        status: Some(status),
                        unknown: !r.read && status >= 500,
                        ..Error::new("http_error")
                    });
                }
                Err(e)
                    if r.read
                        && attempt < 2
                        && matches!(&e, ureq::Error::Io(e) if transient_io(e)) =>
                {
                    ctx.sleep(Duration::from_millis(200 << attempt))?
                }
                Err(_) => {
                    let mut e = ctx.remaining().err().unwrap_or(Error::new("network_error"));
                    e.unknown = !r.read;
                    return Err(e);
                }
            }
        }
        Err(Error::new("retry_exhausted"))
    }
}
fn transient_io(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        e.kind(),
        TimedOut
            | WouldBlock
            | ConnectionReset
            | ConnectionRefused
            | ConnectionAborted
            | BrokenPipe
            | UnexpectedEof
            | Interrupted
    )
}
pub fn retry_after(value: Option<&str>) -> Duration {
    let Some(v) = value.map(str::trim) else {
        return Duration::ZERO;
    };
    if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) {
        // A huge Retry-After must exhaust the tool budget, never wrap to zero.
        return Duration::from_secs(v.parse().unwrap_or(86400).min(86400));
    }
    httpdate::parse_http_date(v)
        .ok()
        .and_then(|t| t.duration_since(SystemTime::now()).ok())
        .unwrap_or_default()
}
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
pub fn query(path: &str, params: &[(String, String)]) -> String {
    if params.is_empty() {
        return path.to_string();
    }
    format!(
        "{path}?{}",
        params
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    )
}
