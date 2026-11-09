# Rust API

```rust
use std::time::Duration;
use serde_json::json;
use wallapop_mcp::{Context, client::{Client, Config}};

let client = Client::new(Config::default())?;
let result = client.invoke(
    &Context::new(Duration::from_secs(45)),
    "search",
    &json!({"keywords":"camera","max_price":"200.00","limit":10}),
)?;
println!("{result}");
# Ok::<(), wallapop_mcp::Error>(())
```

`Client` can be shared through `Arc`. `Config` selects session/token sources,
write capability and pacing. Endpoint overrides are available to Rust callers for
local tests; the CLI always uses the fixed Wallapop service origins.

`Context` contains a total deadline and a cancellation flag. `invoke` uses the
same validation and write gate as MCP; the CLI does not bypass it.

`Error` exposes a stable `code`, optional HTTP `status`, `unknown` outcome flag
and optional `partial` result. Its display never includes upstream URLs, response
bodies, tokens or user input. `Error::value()` produces a JSON error object.

`Client::status()` is local and unverified. `login`, `import_session` and `logout`
manage authentication without exposing credentials in their results. Status
`expiry` contains unverified JWT expiration as Unix seconds when available.

Inspect `tools::definitions()` or run `wallapop-mcp --allow-writes tools` for the
complete argument schemas. The crate is not yet published to a registry; use a
path or Git dependency. The old Go and Python packages are retired.
