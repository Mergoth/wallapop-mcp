# Wallapop MCP

A small Rust executable for searching and managing Wallapop through MCP or the CLI.
Five direct dependencies, native threads, pooled HTTP and verified TLS. No interpreter,
async runtime, browser automation framework or external process is needed at runtime.

## Build

Rust 1.85+ and a C toolchain (for the TLS dependency).

```sh
cargo build --release --locked
./target/release/wallapop-mcp --help
```

The executable is `target/release/wallapop-mcp` (`wallapop-mcp.exe` on Windows).
Release builds optimize for size and strip symbols. No published binaries yet.
The local macOS arm64 build is 1.39 MiB; see [measurements](docs/PERFORMANCE.md).

## MCP

```json
{
  "mcpServers": {
    "wallapop": {
      "command": "/absolute/path/to/wallapop-mcp",
      "args": []
    }
  }
}
```

The default command serves MCP over stdin/stdout. Diagnostics use stderr. Startup,
discovery and `session_status` never contact Wallapop. Public reads need no account,
subject to Wallapop's access controls.

Add `--session /absolute/path/to/.config/wallapop_cli` to reuse a saved session,
or set `WALLAPOP_ACCESS_TOKEN` and optionally `WALLAPOP_REFRESH_TOKEN`.
`WALLAPOP_SESSION_PATH` is equivalent to `--session`; files are loaded only from
an explicit path. Refresh supports both API tokens and web session cookies, with
atomic private session persistence.

**Writes are hidden and disabled unless `--allow-writes` is supplied.** This grants
capability; the agent still needs the user's intent for each consequential action.

| Surface | Features |
| --- | --- |
| 14 read tools | Session, search, deals, listing, seller, categories, own listings, inbox, thread, unread count, purchases, sales, offer limits and history |
| 10 optional write tools | Favorites, messages and seller replies, send/respond/counter offers, publish, edit, reserve, sold, delete |
| CLI | The same tools, ergonomic commands, JSON/CSV output, deal ranking, bounded watch with optional offers/messages and dry-run |
| Rust library | `Client::invoke`, with the same validation, write gate and errors |

## CLI

```sh
wallapop-mcp search 'nintendo switch' --city madrid --max-price 200 --ship-only
wallapop-mcp show 'https://es.wallapop.com/item/example-slug'
wallapop-mcp deals 'camera' --min-discount 0.2 --top 5
wallapop-mcp call list_categories '{}'
wallapop-mcp --session ./session.json login --cookies-file ./cookies.txt
wallapop-mcp --session ./session.json --allow-writes message --conversation THREAD --text 'Hola'
```

Prices are decimal **strings** in tool JSON, such as `"19.95"`, and ordinary decimal
arguments in CLI flags. All mutating CLI commands also require `--allow-writes`.
See [usage](docs/USAGE.md), [tool reference](docs/MCP.md), [Rust API](docs/API.md)
and [migration](docs/MIGRATION.md).

## Design and limits

- Separate modules for protocol, transport, sessions, search, response models,
  messaging, publication and CLI. No duplicated API implementation.
- 45-second total tool budget, four workers by default (maximum 16), one upstream
  request start per second. Ping and cancellation remain responsive during I/O.
- Reads retry transient failures, respect `Retry-After`, share identical cached
  requests and reuse connections. Mutations use fresh connections and are never
  retried after an uncertain result. A definitive 401 may refresh and retry once.
- 64 KiB input frames, 32 KiB tool arguments, 4 MiB upstream bodies, 128 KiB tool
  payloads, 64 cache entries and 8 MiB cache budget. No disk response cache.
- At most five search pages, 50 listings, ten JPEGs, 8 MiB per photo and 32 MiB per
  publication. Every photo is validated before creating the listing.
- Edits preserve existing state. Failed publication steps retain the created item
  ID. Uncertain offers retain their generated ID; uncertain messages identify the
  conversation. Check state before retrying.
- Automatic contact reuses bounded `contacted.json` history beside an explicit
  session, or `--contacted-file PATH`, recording attempts before sending.

## Docker / NAS deployment

Build and run as a persistent HTTP/SSE MCP server (e.g. on a Synology NAS):

```sh
docker compose up -d
```

The container exposes the MCP over SSE on port 8080. Connect agents with:

```json
{
  "mcpServers": {
    "wallapop": {
      "url": "http://<host>:8080/sse"
    }
  }
}
```

Mount a `data/` directory beside `docker-compose.yml` and place `session.json`
inside it. The environment variable `WALLAPOP_SESSION_PATH` is pre-set to
`/data/session.json` inside the container.

**Refreshing the session cookie** — Wallapop uses a web-only 2FA login, so
credentials must be imported from a browser session. When the cookie expires:

1. Go to `es.wallapop.com` in Chrome (logged in), open DevTools Console, run
   `copy(document.cookie)`
2. Run `./scripts/refresh-nas-cookie.sh` and paste when prompted

The script updates the local session, pushes it to the NAS, and restarts the
container in one step.

## Development

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo run --release --locked --example footprint -- target/release/wallapop-mcp
```

Tests use synthetic fixtures, local HTTP servers and the real executable's stdio.
They never contact Wallapop or read a developer's session. See
[architecture](docs/ARCHITECTURE.md), [performance](docs/PERFORMANCE.md) and
[contributing](CONTRIBUTING.md).

Experimental and unofficial: offline tests establish known API contracts, not
current live account compatibility. No checkout, payments or remote HTTP server.
The former browser-only helpers are replaced by session/cookie import; see the
migration guide for compatibility details.

MIT. Based on the endpoint research and behavior of
[figurophobia/wallapop-cli](https://github.com/figurophobia/wallapop-cli), with the
seller reply improvement reviewed in
[hugoocoto's python-only-mcp branch](https://github.com/hugoocoto/wallapop-cli/tree/python-only-mcp).
Original history and [credits](CREDITS.md) are preserved.
