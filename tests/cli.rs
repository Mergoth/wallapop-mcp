mod common;
use common::*;
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};
fn command() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_wallapop-mcp"));
    for key in [
        "WALLAPOP_SESSION_PATH",
        "WALLAPOP_ACCESS_TOKEN",
        "WALLAPOP_REFRESH_TOKEN",
    ] {
        c.env_remove(key);
    }
    c
}
#[test]
fn cli_help_version_discovery_and_session() {
    for arg in ["--help", "--version", "tools", "session"] {
        let o = command().arg(arg).output().unwrap();
        assert!(
            o.status.success(),
            "{arg}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(!o.stdout.is_empty());
    }
    let o = command()
        .args(["call", "session_status", "{}"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["logged_in"], false);
}
#[test]
fn cli_rejects_bad_options_and_limits() {
    for args in [
        vec!["--concurrency", "0"],
        vec!["--timeout", "0"],
        vec!["--timeout", "NaN"],
        vec!["--min-interval", "-1"],
        vec!["search", "x", "--typo", "1"],
        vec!["delete", ITEM],
        vec!["call", "session_status", "[]"],
        vec!["call", "unknown", "{}"],
        vec!["search", "x", "--pages", "10"],
        vec!["show"],
        vec!["deals", "camera", "--offer-pct", "0.8"],
        vec!["deals", "camera", "--msg", "Hola"],
        vec!["deals", "camera", "--offer-pct", "NaN", "--dry-run"],
        vec!["offer", ITEM, "--pct", "2", "--dry-run"],
        vec!["show", ITEM, "extra"],
        vec!["delete", ITEM, "extra", "--yes", "--dry-run"],
        vec!["serve", "--typo", "value"],
        vec!["tools", "--typo", "value"],
        vec!["logout", "--typo", "value"],
        vec!["call", "session_status", "{}", "--typo", "value"],
        vec!["message", ITEM, "--text", "a", "--text", "b", "--dry-run"],
        vec!["message", ITEM, "--text", "a", "--once", "--dry-run"],
        vec![
            "offer",
            ITEM,
            "--price",
            "10",
            "--amount",
            "20",
            "--dry-run",
        ],
        vec!["offer", ITEM, "--price", "10", "--pct", "0.8", "--dry-run"],
        vec![
            "message",
            ITEM,
            "--text",
            "a",
            "--item-id",
            "other",
            "--dry-run",
        ],
        vec!["login", "--token-stdin", "--cookies", "accessToken=other"],
        vec!["login", "--cookies", "accessToken=other", "--typo", "value"],
    ] {
        let o = command().args(&args).output().unwrap();
        assert!(!o.status.success(), "{args:?}");
        assert!(o.stdout.is_empty());
    }
}
#[test]
fn cli_keeps_option_values_literal_and_supports_global_options_after_command() {
    for literal in ["--help", "-h", "--version", "--allow-writes", "--session"] {
        let o = command()
            .args([
                "message",
                ITEM,
                "--text",
                literal,
                "--dry-run",
                "--timeout",
                "1s",
            ])
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{literal}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["arguments"]["message"], literal);
    }
    let o = command()
        .args(["tools", "--allow-writes"])
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&o.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        24
    );
    let o = command()
        .args(["message", "--text", "Hola", "--dry-run", "--", "--help"])
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&o.stdout).unwrap()["arguments"]["item_id"],
        "--help"
    );
    let o = command()
        .args(["message", ITEM, "--text", "--allow-writes"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("writes_disabled"));
}
#[test]
fn cli_dry_run_maps_search_and_write_aliases_without_sending() {
    for (args, tool, key, value) in [
        (
            vec!["message", ITEM, "--text", "Hola", "--dry-run"],
            "send_message",
            "message",
            json!("Hola"),
        ),
        (
            vec!["reserve", ITEM, "--remove", "--dry-run"],
            "reserve_item",
            "reserved",
            json!(false),
        ),
        (
            vec![
                "offer",
                "counter",
                "offer-id",
                "--price",
                "19.95",
                "--dry-run",
            ],
            "counter_offer",
            "amount",
            json!("19.95"),
        ),
    ] {
        let o = command().args(args).output().unwrap();
        assert!(o.status.success());
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["tool"], tool);
        assert_eq!(v["arguments"][key], value);
    }
}
#[test]
fn cli_login_imports_cookies_and_logout_clears_legacy_session() {
    let dir = Temp::new();
    let file = dir.file(
        "cookies.txt",
        "# Netscape\n#HttpOnly_.wallapop.com\tTRUE\t/\tTRUE\t0\taccessToken\ttest-token\n",
    );
    let o = command()
        .arg("--session")
        .arg(&dir.0)
        .args(["login", "--cookies-file"])
        .arg(file)
        .output()
        .unwrap();
    assert!(o.status.success());
    assert!(!String::from_utf8_lossy(&o.stdout).contains("test-token"));
    let o = command()
        .arg("--session")
        .arg(&dir.0)
        .arg("session")
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&o.stdout).unwrap()["logged_in"],
        true
    );
    assert!(
        command()
            .arg("--session")
            .arg(&dir.0)
            .arg("logout")
            .status()
            .unwrap()
            .success()
    );
    let o = command()
        .arg("--session")
        .arg(&dir.0)
        .arg("session")
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&o.stdout).unwrap()["logged_in"],
        false
    );
}
#[test]
fn cli_private_token_stdin_and_call_stdin() {
    let dir = Temp::new();
    let mut child = command()
        .arg("--session")
        .arg(&dir.0)
        .args(["login", "--token-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"private-token\n")
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success());
    assert!(!String::from_utf8_lossy(&o.stdout).contains("private-token"));
    let mut child = command()
        .args(["call", "session_status", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{}").unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
}
#[test]
fn decimal_inputs_durations_and_cookie_formats() {
    use wallapop_mcp::{
        cents,
        cli::{cookies, duration},
        money,
    };
    for (s, n) in [
        ("0", 0),
        ("0.01", 1),
        ("19.95", 1995),
        ("12.5", 1250),
        ("99999999.99", 9999999999),
    ] {
        assert_eq!(cents(s).unwrap(), n);
    }
    for s in [
        "-1",
        "01",
        "1.",
        "1.999",
        "NaN",
        "inf",
        "1e2",
        "1,50",
        "100000000",
        ".01",
        " 1",
        "1 ",
        "+1",
        "",
    ] {
        assert!(cents(s).is_err(), "{s}");
    }
    assert!(money(&json!("0")).is_err());
    assert!(money(&json!(19.95)).is_err());
    assert_eq!(money(&json!("19.95")).unwrap().to_string(), "19.95");
    assert_eq!(duration("200ms").unwrap(), Duration::from_millis(200));
    assert_eq!(duration("1.5s").unwrap(), Duration::from_millis(1500));
    assert_eq!(
        cookies("accessToken=abc; device_id=def").unwrap()["cookies"]["device_id"],
        "def"
    );
    assert!(
        cookies(".evil.test\tTRUE\t/\tTRUE\t0\taccessToken\tevil").unwrap()["cookies"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}
