# Local Rust performance

Measured on 2026-10-27 with macOS 27.0.1 arm64 and Rust 1.92.0, using the
size-optimized release profile in `Cargo.toml`. No Wallapop account, credentials
or external requests were used.

| Measurement | Observed result |
| --- | --- |
| Stripped executable | 1,452,704 bytes (1.39 MiB) |
| MCP process round trip, 30 warm launches | Median 2.275 ms; p95 4.003 ms |
| Direct runtime dependencies | 5 |
| Rust source in `src/`, including comments, blanks and unit tests | 3,656 lines |

The round trip includes process creation, initialization, tool discovery, a local
`session_status` call and clean shutdown. The Rust harness validates the actual
JSON responses on every launch. These figures describe this machine and workload;
they do not measure live searches, resident memory or allocation totals. Earlier
Go measurements used a different workload and are retained in Git history.

Reproduce with:

```sh
cargo build --release --locked
cargo run --release --locked --example footprint -- target/release/wallapop-mcp
```

On Windows, pass `target/release/wallapop-mcp.exe`. `make benchmark` runs the
equivalent commands on systems with Make.

The default upstream pacing is one request start per second, a 45-second tool
budget and four active calls. Public cached reads expire after 30 seconds,
account reads after five seconds and categories after an hour. Identical cacheable
reads share an upstream request; fresh reads run independently.

The cache holds at most 64 entries and 8 MiB, including request keys. Upstream
bodies are limited to 4 MiB and tool payloads to 128 KiB. Publication holds at most
32 MiB of input photos plus one multipart request buffer and normal HTTP overhead.
Multipart buffers reserve their capacity once; writes never construct cache keys
from image bytes. These are component limits, not a bound on total process memory.

Contact history is an optional bounded JSON file, at most 2 MiB and 10,000 IDs.
It does not cache upstream responses. The implementation uses native threads,
pooled blocking HTTP and TLS, without an async runtime or browser framework.
