use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Instant,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or("target/release/wallapop-mcp".into()),
    );
    let mut samples = Vec::new();
    for _ in 0..30 {
        let start = Instant::now();
        let mut child = Command::new(&path)
            .env_remove("WALLAPOP_SESSION_PATH")
            .env_remove("WALLAPOP_ACCESS_TOKEN")
            .env_remove("WALLAPOP_REFRESH_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"benchmark\",\"version\":\"1\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"session_status\"}}\n"
        ).as_bytes())?;
        let output = child.wait_with_output()?;
        assert!(output.status.success() && output.stderr.is_empty());
        let values = String::from_utf8(output.stdout)?
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(values.len(), 3);
        assert!(values.iter().all(|v| v.get("error").is_none()));
        assert!(
            values
                .iter()
                .any(|v| v["result"]["structuredContent"]["session_verified"] == false)
        );
        samples.push(start.elapsed().as_micros());
    }
    samples.sort_unstable();
    println!(
        "{}",
        serde_json::json!({"binary_bytes":std::fs::metadata(path)?.len(),"runs":samples.len(),"mcp_roundtrip_median_us":samples[15],"mcp_roundtrip_p95_us":samples[28]})
    );
    Ok(())
}
