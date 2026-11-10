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

The container runs [`server/`](server), a Python service that serves MCP over
Streamable HTTP at `/mcp` behind the shared NAS gateway (`mcp-gateway`). The gateway
forwards tool calls to one long-lived `wallapop-mcp` stdio process. Single sign-in and
client authentication (OAuth) are handled by the NAS gateway, so this container publishes
no public ports and requires no per-service Google or reverse proxy configuration.
Deploy the shared gateway first, then deploy `wallapop-mcp`.

Public endpoint: `https://mcp.mergoth.synology.me/wallapop/mcp`

```
claude.ai / ChatGPT / Gemini → router :443 → DSM reverse proxy (TLS) → gateway (:8080) → wallapop-mcp (:8000)
```

**Setup**

1. Write `.env` beside `docker-compose.yml` (mode 600):

   ```sh
   GATEWAY_REGISTRATION_TOKEN=...
   WALLAPOP_UPSTREAM_TOKEN=...
   GATEWAY_INTERNAL_URL=http://mcp-gateway:8081
   WALLAPOP_MCP_ARGS=--allow-writes
   UPLOAD_SECRET=...
   UPLOAD_DIR=/data/uploads
   PUBLIC_BASE_URL=https://mcp.mergoth.synology.me/wallapop
   UPLOAD_TTL_SECONDS=600
   PHOTO_URL_HOSTS=
   ```

2. Put `session.json` in `data/`, ensure `data/` exists, and deploy:
   `docker compose up -d --build`

| Variable | Notes |
| --- | --- |
| `GATEWAY_REGISTRATION_TOKEN` | Shared secret to self-register backend with the gateway listener. |
| `WALLAPOP_UPSTREAM_TOKEN` | Bearer token required for gateway-to-backend calls (at least 32 characters). |
| `GATEWAY_INTERNAL_URL` | Gateway internal registration endpoint (default `http://mcp-gateway:8081`). |
| `WALLAPOP_MCP_ARGS` | Extra arguments for the binary, e.g. `--allow-writes`. Writes stay off by default. |
| `UPLOAD_SECRET` | Secret used to sign and verify photo upload tickets (required). |
| `UPLOAD_DIR` | Directory where uploaded photos are stored (default `/data/uploads`). |
| `PUBLIC_BASE_URL` | Public base URL used for generating upload ticket links (default `https://mcp.mergoth.synology.me/wallapop`). |
| `UPLOAD_TTL_SECONDS` | Ticket validity lifetime in seconds (default `600`). |
| `PHOTO_URL_HOSTS` | Extra allowlisted hosts for photo URLs (comma-separated, optional). |

**Photo uploads**

When uploads are enabled, tickets created by Wallapop MCP produce upload URLs in the shape:
`https://mcp.mergoth.synology.me/wallapop/upload/<ticket_id>?exp=<exp>&sig=<sig>`
The gateway routes `/upload/` paths directly without requiring gateway authentication, authorized by the ticket HMAC signature.

**Connecting** — Add custom connector with URL `https://mcp.mergoth.synology.me/wallapop/mcp`.
Client authentication is handled by the NAS gateway.

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
(cd server && uv run pytest -q && uv run ruff check .)
```

Tests use synthetic fixtures, local HTTP servers and the real executable's stdio.
They never contact Wallapop or read a developer's session. See
[architecture](docs/ARCHITECTURE.md), [performance](docs/PERFORMANCE.md) and
[contributing](CONTRIBUTING.md).

Experimental and unofficial: offline tests establish known API contracts, not
current live account compatibility. No checkout or payments. The binary has no HTTP
server of its own; the optional `server/` adds one for remote use behind the gateway.
The former browser-only helpers are replaced by session/cookie import; see the
migration guide for compatibility details.

MIT. Based on the endpoint research and behavior of
[figurophobia/wallapop-cli](https://github.com/figurophobia/wallapop-cli), with the
seller reply improvement reviewed in
[hugoocoto's python-only-mcp branch](https://github.com/hugoocoto/wallapop-cli/tree/python-only-mcp).
Original history and [credits](CREDITS.md) are preserved.
