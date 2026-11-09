use super::{Call, Client, Config, Session};
use crate::{Context, Error, Result, text, transport::Request};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::Duration,
};

impl Session {
    pub(super) fn load(c: &Config) -> Result<Self> {
        let mut path = c.session.clone();
        if let Some(p) = &path {
            if p.is_dir() {
                path = Some(p.join("session.json"));
            }
        }
        let mut data = json!({});
        if let Some(p) = &path {
            for file in [
                Some(p.clone()),
                (p.file_name().is_some_and(|n| n == "session.json"))
                    .then(|| p.with_file_name("credentials.json")),
            ]
            .into_iter()
            .flatten()
            {
                if file.exists() {
                    let v: Value = serde_json::from_slice(&read_file(&file, 65536)?)
                        .map_err(|_| Error::new("invalid_session_json"))?;
                    let m = v.as_object().ok_or(Error::new("invalid_session_json"))?;
                    for (k, v) in m {
                        if data.get(k).is_none() {
                            data[k] = v.clone();
                        }
                    }
                }
            }
        }
        normalize_session(&mut data)?;
        let access = if c.access_token.is_empty() {
            token(&data)
        } else {
            c.access_token.clone()
        };
        let refresh = if c.refresh_token.is_empty() {
            refresh_token(&data)
        } else {
            c.refresh_token.clone()
        };
        if !access.is_empty() {
            data["cookies"]["accessToken"] = access.clone().into();
        }
        Ok(Self {
            access,
            refresh,
            data,
            path,
        })
    }

    fn store_token(&mut self, access: String, refresh: String) -> Result<()> {
        self.access = access;
        if !refresh.is_empty() {
            self.refresh = refresh;
        }
        for k in ["access_token", "accessToken", "token"] {
            self.data[k] = self.access.clone().into();
        }
        for k in ["refresh_token", "refreshToken"] {
            self.data[k] = self.refresh.clone().into();
        }
        self.data["cookies"]["accessToken"] = self.access.clone().into();
        if let Some(d) = self.data.get_mut("data").and_then(Value::as_object_mut) {
            for k in [
                "access_token",
                "accessToken",
                "token",
                "refresh_token",
                "refreshToken",
            ] {
                d.remove(k);
            }
        }
        if let Some(p) = &self.path {
            atomic_save(p, &self.data)?;
        }
        Ok(())
    }

    fn clear_legacy(&self) -> Result<()> {
        if let Some(p) = &self.path {
            let legacy = p.with_file_name("credentials.json");
            if legacy != *p && legacy.exists() {
                atomic_save(&legacy, &json!({}))?;
            }
        }
        Ok(())
    }
}

impl Client {
    pub fn status(&self) -> Value {
        let s = self.session.lock().unwrap();
        let present = !s.access.is_empty()
            || !s.refresh.is_empty()
            || !text(&s.data["cookies"]["__Secure-next-auth.session-token"]).is_empty();
        json!({"logged_in":!s.access.is_empty(),"credential_present":present,"session_verified":false,
            "source":if !present {"none"} else if s.path.is_some() {"session"} else {"token"},
            "expiry":claims(&s.access)["exp"],"writes_enabled":self.allow_writes})
    }

    pub(super) fn refresh(&self, ctx: &Context, old: &str) -> Result<()> {
        let _guard = loop {
            if let Ok(g) = self.refreshing.try_lock() {
                break g;
            }
            ctx.sleep(Duration::from_millis(10))?;
        };
        let (access, refresh, data) = {
            let s = self.session.lock().unwrap();
            (s.access.clone(), s.refresh.clone(), s.data.clone())
        };
        if !access.is_empty() && access != old {
            return Ok(());
        }
        let mut refreshed = None;
        if !refresh.is_empty() {
            let body = json!({"refresh_token":refresh,"client_id":"wallapop"}).to_string();
            let headers = vec![("Content-Type".into(), "application/json".into())];
            if let Ok(r) = self.transport.request(
                ctx,
                Request {
                    method: "POST",
                    url: &format!("{}/api/v3/access/refresh", self.api),
                    headers: &headers,
                    body: body.as_bytes(),
                    read: false,
                    ttl: Duration::ZERO,
                },
            ) {
                refreshed = serde_json::from_slice::<Value>(&r.body)
                    .ok()
                    .filter(|v| !token(v).is_empty() && token(v) != old);
            }
        }
        if refreshed.is_none()
            && !text(&data["cookies"]["__Secure-next-auth.session-token"]).is_empty()
        {
            let cookies = [
                "__Secure-next-auth.session-token",
                "device_id",
                "device_fgp",
                "accessToken",
            ]
            .iter()
            .filter_map(|k| data["cookies"][k].as_str().map(|v| format!("{k}={v}")))
            .collect::<Vec<_>>()
            .join("; ");
            let headers = vec![
                ("Cookie".into(), cookies),
                ("Accept".into(), "application/json".into()),
            ];
            if let Ok(r) = self.transport.request(
                ctx,
                Request {
                    method: "GET",
                    url: &format!("{}/api/auth/session", self.web),
                    headers: &headers,
                    body: &[],
                    read: false,
                    ttl: Duration::ZERO,
                },
            ) {
                refreshed = serde_json::from_slice::<Value>(&r.body)
                    .ok()
                    .filter(|v| !token(v).is_empty() && token(v) != old);
            }
        }
        let v = refreshed.ok_or_else(|| {
            ctx.remaining()
                .err()
                .unwrap_or(Error::new("authentication_required"))
        })?;
        self.store_token(token(&v), refresh_token(&v))
    }

    fn store_token(&self, access: String, refresh: String) -> Result<()> {
        let result = self.session.lock().unwrap().store_token(access, refresh);
        self.transport.invalidate();
        result
    }

    pub fn import_session(&self, mut data: Value) -> Result<()> {
        normalize_session(&mut data)?;
        let access = token(&data);
        let refresh = refresh_token(&data);
        if access.is_empty()
            && refresh.is_empty()
            && text(&data["cookies"]["__Secure-next-auth.session-token"]).is_empty()
        {
            return Err(Error::new("session_has_no_credentials"));
        }
        let mut s = self.session.lock().unwrap();
        s.data = data;
        s.refresh.clear();
        let result = s
            .store_token(access, refresh)
            .and_then(|_| s.clear_legacy());
        self.transport.invalidate();
        result
    }

    pub fn login(&self, ctx: &Context, email: &str, password: &str) -> Result<()> {
        let body = json!({"email":email,"password":password,"client_id":"wallapop"});
        let v = self.execute(
            ctx,
            Call {
                method: "POST",
                path: "/access/login".into(),
                body: body.to_string().into_bytes(),
                auth: false,
                fresh: true,
                headers: vec![("Content-Type".into(), "application/json".into())],
            },
        )?;
        let access = token(&v);
        if access.is_empty() {
            return Err(Error::new("login_failed"));
        }
        self.import_session(v)
    }

    pub fn logout(&self) -> Result<()> {
        let mut s = self.session.lock().unwrap();
        s.access.clear();
        s.refresh.clear();
        s.data = json!({});
        if let Some(p) = &s.path {
            atomic_save(p, &s.data)?;
        }
        let result = s.clear_legacy();
        self.transport.invalidate();
        result
    }
}

fn normalize_session(data: &mut Value) -> Result<()> {
    if !data.is_object()
        || data.get("cookies").is_some_and(|v| !v.is_object())
        || data.get("data").is_some_and(|v| !v.is_object())
    {
        return Err(Error::new("invalid_session_json"));
    }
    for container in [&*data, &data["data"]] {
        for key in [
            "access_token",
            "accessToken",
            "token",
            "refresh_token",
            "refreshToken",
        ] {
            if container.get(key).is_some_and(|v| !v.is_string()) {
                return Err(Error::new("invalid_session_json"));
            }
        }
    }
    if data.get("cookies").is_none() {
        data["cookies"] = json!({});
    }
    for k in [
        "__Secure-next-auth.session-token",
        "device_id",
        "device_fgp",
        "accessToken",
    ] {
        if data["cookies"].get(k).is_none() {
            if let Some(v) = data.get(k).cloned() {
                data["cookies"][k] = v;
            }
        }
    }
    if data["cookies"]
        .as_object()
        .unwrap()
        .values()
        .any(|v| !v.is_string())
    {
        return Err(Error::new("invalid_session_json"));
    }
    Ok(())
}

pub fn token(v: &Value) -> String {
    credential(v, &["access_token", "accessToken", "token"])
}

fn refresh_token(v: &Value) -> String {
    credential(v, &["refresh_token", "refreshToken"])
}

fn credential(v: &Value, aliases: &[&str]) -> String {
    for p in [v, &v["data"], &v["cookies"]] {
        for key in aliases {
            if let Some(s) = p[*key].as_str().filter(|s| !s.is_empty()) {
                return s.into();
            }
        }
    }
    String::new()
}

pub fn claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}

pub fn read_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    if !fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(Error::new("regular_file_required"));
    }
    let mut b = Vec::new();
    fs::File::open(path)
        .map_err(|_| Error::new("file_unreadable"))?
        .take((limit + 1) as u64)
        .read_to_end(&mut b)
        .map_err(|_| Error::new("file_unreadable"))?;
    if b.len() > limit {
        return Err(Error::new("file_too_large"));
    }
    Ok(b)
}

pub(crate) fn atomic_save(path: &Path, data: &Value) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|_| Error::new("session_persistence_failed"))?;
    let tmp = dir.join(format!(".session-{}", crate::uuid()?));
    let result = (|| -> std::io::Result<()> {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(data.to_string().as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    let _ = fs::remove_file(tmp);
    result.map_err(|_| Error::new("session_persistence_failed"))
}
