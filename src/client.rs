//! Shared API boundary. Domain operations live in the child modules.
mod messaging;
mod models;
mod publishing;
mod reads;
mod search;
mod session;
mod writes;

use crate::{
    Context, Error, Result, first, text,
    transport::{Request, Transport, query},
};
use serde_json::{Value, json};
pub(crate) use session::atomic_save;
pub use session::{claims, read_file, token};
use std::{path::PathBuf, sync::Mutex, time::Duration};

#[derive(Default)]
pub struct Config {
    pub api: String,
    pub web: String,
    pub pubnub: String,
    pub access_token: String,
    pub refresh_token: String,
    pub session: Option<PathBuf>,
    pub allow_writes: bool,
    pub interval: Option<Duration>,
}
struct Session {
    access: String,
    refresh: String,
    data: Value,
    path: Option<PathBuf>,
}
pub struct Client {
    api: String,
    web: String,
    pubnub: String,
    pub allow_writes: bool,
    transport: Transport,
    session: Mutex<Session>,
    refreshing: Mutex<()>,
}
struct Call {
    method: &'static str,
    path: String,
    body: Vec<u8>,
    auth: bool,
    fresh: bool,
    headers: Vec<(String, String)>,
}
impl Client {
    pub fn new(c: Config) -> Result<Self> {
        let session = Session::load(&c)?;
        Ok(Self {
            api: base(c.api, "https://api.wallapop.com"),
            web: base(c.web, "https://es.wallapop.com"),
            pubnub: base(c.pubnub, "https://ps7.pndsn.com"),
            allow_writes: c.allow_writes,
            transport: Transport::new(c.interval.unwrap_or(Duration::from_secs(1))),
            session: Mutex::new(session),
            refreshing: Mutex::new(()),
        })
    }

    pub fn invoke(&self, ctx: &Context, name: &str, args: &Value) -> Result<Value> {
        let tool = crate::tools::definitions()
            .iter()
            .find(|t| t.name == name)
            .ok_or(Error::new("unknown_tool"))?;
        tool.validate(args)?;
        if tool.write && !self.allow_writes {
            return Err(Error::new("writes_disabled"));
        }
        ctx.remaining()?;
        if !tool.write {
            return self.read(ctx, name, args);
        }
        self.transport.invalidate();
        let result = self.write(ctx, name, args);
        self.transport.invalidate();
        result
    }

    fn get(&self, ctx: &Context, path: &str, auth: bool, fresh: bool) -> Result<Value> {
        self.execute(
            ctx,
            Call {
                method: "GET",
                path: path.into(),
                body: Vec::new(),
                auth,
                fresh,
                headers: Vec::new(),
            },
        )
    }

    fn send(&self, ctx: &Context, method: &'static str, path: &str, body: &Value) -> Result<Value> {
        self.execute(
            ctx,
            Call {
                method,
                path: path.into(),
                body: if body.is_null() {
                    Vec::new()
                } else {
                    body.to_string().into_bytes()
                },
                auth: true,
                fresh: true,
                headers: vec![("Content-Type".into(), "application/json".into())],
            },
        )
    }

    fn execute(&self, ctx: &Context, c: Call) -> Result<Value> {
        let url = format!(
            "{}{}{}",
            self.api,
            if c.path.starts_with("/bff/") || c.path.starts_with("/sdui/") {
                ""
            } else {
                "/api/v3"
            },
            c.path
        );
        let read = c.method == "GET";
        let ttl = if c.fresh {
            Duration::ZERO
        } else if c.path.starts_with("/categories") {
            Duration::from_secs(3600)
        } else if c.auth {
            Duration::from_secs(5)
        } else {
            Duration::from_secs(30)
        };
        for attempt in 0..2 {
            let (access, device) = {
                let s = self.session.lock().unwrap();
                (
                    s.access.clone(),
                    text(&first(&s.data["cookies"], &["device_id"])),
                )
            };
            if c.auth && access.is_empty() && attempt == 0 {
                self.refresh(ctx, "")?;
                continue;
            }
            let mut headers: Vec<(String, String)> = vec![
                ("Accept".into(), "application/json, text/plain, */*".into()),
                ("Origin".into(), self.web.clone()),
                ("Referer".into(), format!("{}/", self.web)),
                ("X-DeviceOS".into(), "0".into()),
                ("X-Product".into(), "wallapop".into()),
                ("X-AppVersion".into(), "826560".into()),
                ("Accept-Language".into(), "es-ES,es;q=0.9,en;q=0.8".into()),
            ];
            if c.auth {
                headers.push(("Authorization".into(), format!("Bearer {access}")));
            }
            if !device.is_empty() {
                headers.push(("X-DeviceID".into(), device));
            }
            for (k, v) in &c.headers {
                headers.retain(|(key, _)| !key.eq_ignore_ascii_case(k));
                headers.push((k.clone(), v.clone()));
            }
            let response = self.transport.request(
                ctx,
                Request {
                    method: c.method,
                    url: &url,
                    headers: &headers,
                    body: &c.body,
                    read,
                    ttl,
                },
            );
            match response {
                Err(e) if c.auth && e.status == Some(401) && attempt == 0 => {
                    self.refresh(ctx, &access)?
                }
                Err(e) => return Err(e),
                Ok(r) => {
                    if r.body.iter().all(u8::is_ascii_whitespace) {
                        return Ok(json!({"ok":true,"status":r.status}));
                    }
                    // picture2 returns a plain photo ID on successful uploads.
                    if c.path.ends_with("/picture2") {
                        if let Ok(id) = std::str::from_utf8(&r.body) {
                            if crate::identifier(id.trim()).is_ok() {
                                return Ok(json!({"id":id.trim()}));
                            }
                        }
                    }
                    let result: Result<Value> = serde_json::from_slice::<Value>(&r.body)
                        .ok()
                        .filter(|v| !v.is_null())
                        .ok_or(Error::new("invalid_response"));
                    return result.map_err(|mut e| {
                        e.unknown = !read;
                        e
                    });
                }
            }
        }
        Err(Error::new("authentication_required"))
    }

    /// Makes an authenticated request and returns (HTTP status, Location header value).
    /// Used for endpoints that communicate their result via Location rather than body:
    /// save_search (POST → 201 + Location) and check_saved_search (HEAD → 200/404).
    pub(crate) fn authed_with_location(
        &self,
        ctx: &Context,
        method: &'static str,
        path: &str,
    ) -> Result<(u16, Option<String>)> {
        for attempt in 0..2 {
            let (access, device) = {
                let s = self.session.lock().unwrap();
                (
                    s.access.clone(),
                    text(&first(&s.data["cookies"], &["device_id"])),
                )
            };
            if access.is_empty() && attempt == 0 {
                self.refresh(ctx, "")?;
                continue;
            }
            let url = format!(
                "{}/api/v3{}",
                self.api,
                path
            );
            let mut headers: Vec<(String, String)> = vec![
                ("Accept".into(), "application/json, text/plain, */*".into()),
                ("Origin".into(), self.web.clone()),
                ("Referer".into(), format!("{}/", self.web)),
                ("X-DeviceOS".into(), "0".into()),
                ("X-Product".into(), "wallapop".into()),
                ("X-AppVersion".into(), "826560".into()),
                ("Accept-Language".into(), "es-ES,es;q=0.9,en;q=0.8".into()),
            ];
            if !access.is_empty() {
                headers.push(("Authorization".into(), format!("Bearer {access}")));
            }
            if !device.is_empty() {
                headers.push(("X-DeviceID".into(), device));
            }
            let result = self.transport.request(
                ctx,
                Request {
                    method,
                    url: &url,
                    headers: &headers,
                    body: &[],
                    read: false,
                    ttl: std::time::Duration::ZERO,
                },
            );
            match result {
                Err(e) if e.status == Some(401) && attempt == 0 => {
                    self.refresh(ctx, &access)?;
                }
                Err(e) if e.status == Some(404) => return Ok((404, None)),
                Err(e) => return Err(e),
                Ok(r) => return Ok((r.status, r.location)),
            }
        }
        Err(Error::new("authentication_required"))
    }

    fn with_query(&self, path: &str, params: &[(&str, String)]) -> String {
        query(
            path,
            &params
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<Vec<_>>(),
        )
    }
}

fn base(value: String, default: &str) -> String {
    if value.is_empty() {
        default.into()
    } else {
        value.trim_end_matches('/').into()
    }
}
