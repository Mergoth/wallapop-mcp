mod common;
use common::*;
use serde_json::{Value, json};
use std::{
    io::{BufReader, Write},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use wallapop_mcp::{
    Error,
    client::{Client, Config},
    protocol::{MAX_LINE, MAX_OUTPUT, serve, tool_result},
};

fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})
}
fn ready() -> Value {
    json!({"jsonrpc":"2.0","method":"notifications/initialized"})
}
fn rpc(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
#[derive(Clone)]
struct Output(Arc<Mutex<Vec<u8>>>);
impl Write for Output {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn exchange(requests: Vec<Value>, writes: bool) -> Vec<Value> {
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let output = Output(Arc::default());
    let c = Client::new(Config {
        allow_writes: writes,
        ..Config::default()
    })
    .unwrap();
    serve(
        input.as_bytes(),
        output.clone(),
        Arc::new(c),
        Duration::from_secs(1),
        4,
    )
    .unwrap();
    let bytes = output.0.lock().unwrap();
    String::from_utf8_lossy(&bytes)
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
#[test]
fn lifecycle_discovery_negotiation_annotations_and_write_gate() {
    for writes in [false, true] {
        let mut init = initialize();
        init["params"]["protocolVersion"] = "future-version".into();
        let r = exchange(
            vec![
                init,
                ready(),
                rpc(1, "tools/list", json!({})),
                rpc(2, "tools/call", json!({"name":"session_status"})),
            ],
            writes,
        );
        assert_eq!(r[0]["result"]["protocolVersion"], "2025-06-18");
        let tools = r[1]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), if writes { 24 } else { 14 });
        assert_eq!(
            tools
                .iter()
                .find(|t| t["name"] == "session_status")
                .unwrap()["annotations"]["openWorldHint"],
            false
        );
        assert_eq!(
            r[2]["result"]["structuredContent"]["writes_enabled"],
            writes
        );
        assert!(r[2]["result"]["content"][0]["text"].is_string());
    }
}
#[test]
fn rejects_unknown_disabled_methods_and_invalid_input_without_upstream() {
    let r = exchange(
        vec![
            initialize(),
            ready(),
            rpc(
                1,
                "tools/call",
                json!({"name":"delete_item","arguments":{"item_id":ITEM}}),
            ),
            rpc(2, "unknown", json!({})),
            rpc(
                3,
                "tools/call",
                json!({"name":"search","arguments":{"keywords":"x","pages":6}}),
            ),
            rpc(
                4,
                "tools/call",
                json!({"name":"send_message","arguments":{"item_id":ITEM,"conversation_id":"thread","message":"x"}}),
            ),
        ],
        true,
    );
    assert_eq!(
        r.iter().find(|r| r["id"] == 2).unwrap()["error"]["code"],
        -32601
    );
    for id in [3, 4] {
        assert_eq!(
            r.iter().find(|r| r["id"] == id).unwrap()["result"]["isError"],
            true
        );
    }
    let r = exchange(
        vec![
            initialize(),
            ready(),
            rpc(
                1,
                "tools/call",
                json!({"name":"delete_item","arguments":{"item_id":ITEM}}),
            ),
        ],
        false,
    );
    assert_eq!(r[1]["error"]["code"], -32602);
}
#[test]
fn requires_handshake_and_valid_jsonrpc_envelopes() {
    let r = exchange(
        vec![
            rpc(1, "tools/list", json!({})),
            json!([]),
            json!({"jsonrpc":"1.0","method":"ping","id":2}),
            json!({"jsonrpc":"2.0","id":false,"method":"ping"}),
            initialize(),
            ready(),
            rpc(3, "ping", json!({})),
            initialize(),
        ],
        false,
    );
    assert_eq!(r[0]["error"]["code"], -32600);
    assert_eq!(r[1]["error"]["code"], -32600);
    assert_eq!(r[5]["result"], json!({}));
    assert_eq!(r[6]["error"]["code"], -32600);
}
#[test]
fn output_budget_preserves_partial_resource_and_unknown_outcome() {
    let e = Error::new("failed")
        .unknown()
        .partial(json!({"item_id":ITEM,"huge":"x".repeat(MAX_OUTPUT)}));
    let v = tool_result(Err(e), true);
    assert_eq!(v["isError"], true);
    assert_eq!(v["structuredContent"]["partial_result"]["item_id"], ITEM);
    assert_eq!(v["structuredContent"]["error"]["outcome_unknown"], true);
    assert_eq!(
        tool_result(Ok(json!([1, 2])), false)["structuredContent"],
        json!({"result":[1,2]})
    );
}
#[test]
fn oversized_frame_is_bounded_and_malformed_json_recovers() {
    let output = Output(Arc::default());
    let c = Arc::new(Client::new(Config::default()).unwrap());
    let bytes = format!("broken\n{}\n", "x".repeat(MAX_LINE + 1));
    serve(
        bytes.as_bytes(),
        output.clone(),
        c,
        Duration::from_secs(1),
        4,
    )
    .unwrap();
    let b = output.0.lock().unwrap();
    let r = String::from_utf8_lossy(&b)
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0]["error"]["code"], -32700);
    assert_eq!(r[1]["error"]["code"], -32600);
}
#[test]
fn executable_stdio_handshake_discovery_call_and_clean_stdout() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wallapop-mcp"))
        .env_remove("WALLAPOP_SESSION_PATH")
        .env_remove("WALLAPOP_ACCESS_TOKEN")
        .env_remove("WALLAPOP_REFRESH_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for v in [
        initialize(),
        ready(),
        rpc(1, "tools/list", json!({})),
        rpc(2, "tools/call", json!({"name":"session_status"})),
    ] {
        writeln!(input, "{v}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let r = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(r.len(), 3);
    assert_eq!(r[1]["result"]["tools"].as_array().unwrap().len(), 14);
    assert_eq!(r[2]["result"]["structuredContent"]["logged_in"], false);
}
#[test]
fn cancelled_read_does_not_block_ping_or_issue_followup_requests() {
    let s = Server::new(|r| {
        assert!(r.path().ends_with("/search/components"));
        std::thread::sleep(Duration::from_millis(100));
        Reply::json(200, json!({"search_id":"sid"}))
    });
    let client = Arc::new(s.client(false));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let stream = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (input, _) = listener.accept().unwrap();
    let output = Output(Arc::default());
    let captured = output.clone();
    let worker = std::thread::spawn(move || {
        serve(
            BufReader::new(input),
            captured,
            client,
            Duration::from_secs(1),
            1,
        )
        .unwrap()
    });
    let mut stream = stream;
    for v in [
        initialize(),
        ready(),
        rpc(
            1,
            "tools/call",
            json!({"name":"search","arguments":{"keywords":"a"}}),
        ),
    ] {
        writeln!(stream, "{v}").unwrap();
    }
    while s.count() == 0 {
        std::thread::sleep(Duration::from_millis(1));
    }
    writeln!(
        stream,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}})
    )
    .unwrap();
    writeln!(stream, "{}", rpc(2, "ping", json!({}))).unwrap();
    for _ in 0..100 {
        if String::from_utf8_lossy(&output.0.lock().unwrap()).contains("\"id\":2") {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(String::from_utf8_lossy(&output.0.lock().unwrap()).contains("\"id\":2"));
    drop(stream);
    worker.join().unwrap();
    assert_eq!(s.count(), 1);
    let b = output.0.lock().unwrap();
    assert!(!String::from_utf8_lossy(&b).contains("\"id\":1"));
}
