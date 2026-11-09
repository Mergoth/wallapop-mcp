#![allow(dead_code)]
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use wallapop_mcp::{
    Context,
    client::{Client, Config},
};

pub const ITEM: &str = "abc123def456";
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}
impl Request {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap()
    }
}
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}
impl Reply {
    pub fn json(status: u16, v: Value) -> Self {
        Self::raw(status, v.to_string())
    }
    pub fn raw(status: u16, s: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: s.into(),
            headers: Vec::new(),
        }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
}
pub struct Server {
    pub url: String,
    pub requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    pub fn new(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (done, log) = (stop.clone(), requests.clone());
        let handler = Arc::new(handler);
        let worker = thread::spawn(move || {
            let mut children = Vec::new();
            while !done.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let handler = handler.clone();
                        let log = log.clone();
                        children.push(thread::spawn(move || connection(stream, log, &*handler)));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            for child in children {
                child.join().unwrap();
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    pub fn client(&self, writes: bool) -> Client {
        self.configured(writes, "account", "", None)
    }
    pub fn configured(
        &self,
        writes: bool,
        access: &str,
        refresh: &str,
        session: Option<PathBuf>,
    ) -> Client {
        Client::new(Config {
            api: self.url.clone(),
            web: self.url.clone(),
            pubnub: self.url.clone(),
            allow_writes: writes,
            access_token: access.into(),
            refresh_token: refresh.into(),
            session,
            interval: Some(Duration::ZERO),
        })
        .unwrap()
    }
    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}
fn connection(
    mut stream: TcpStream,
    log: Arc<Mutex<Vec<Request>>>,
    handler: &dyn Fn(&Request) -> Reply,
) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap().into();
    let target = parts.next().unwrap().into();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        let (k, v) = line.split_once(':').unwrap();
        headers.insert(k.to_ascii_lowercase(), v.trim().to_string());
    }
    let length = headers
        .get("content-length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let request = Request {
        method,
        target,
        headers,
        body,
    };
    log.lock().unwrap().push(request.clone());
    let reply = handler(&request);
    let mut head = format!(
        "HTTP/1.1 {} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (k, v) in reply.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
}
pub fn ctx() -> Context {
    Context::new(Duration::from_secs(5))
}
pub fn item() -> Value {
    json!({"id":ITEM,"title":{"original":"Camera"},"description":{"original":"Full description"},"slug":"camera-123",
    "price":{"cash":{"amount":19.95,"currency":"EUR"}},"brand":"Brand","model":"Model","color":"blue","condition":"good","category_leaf_id":"10088",
    "shipping":{"user_allows_shipping":false},"location":{"latitude":41.2,"longitude":2.2,"city":"Barcelona"},
    "images":[{"id":"photo1","urls":{"medium":"https://images.wallapop.com/one"}},{"id":"photo2","media":{"big":"https://images.wallapop.com/two"}}]})
}
pub fn thread() -> Value {
    json!({"hash":"conversation","channel":"channel","with_user":{"hash":"recipient","name":"Ada"},"messages":{"messages":[]}})
}
pub fn signed_token() -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    format!(
        "e30.{}.signature",
        URL_SAFE_NO_PAD.encode(br#"{"hashed_sub":"sender","exp":2000000000}"#)
    )
}
pub struct Temp(pub PathBuf);
impl Temp {
    pub fn new() -> Self {
        let p =
            std::env::temp_dir().join(format!("wallapop-test-{}", wallapop_mcp::uuid().unwrap()));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    pub fn file(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let p = self.0.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
