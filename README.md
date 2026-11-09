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

The container runs [`gateway/`](gateway), a small Python service that serves MCP over
Streamable HTTP at `/mcp` and forwards every tool call to one long-lived `wallapop-mcp`
stdio process. The gateway is its own OAuth 2.1 authorization server with exactly one
owner: claude.ai (web, Desktop, mobile) registers itself, and you approve it by signing
in with Google. Only the Google account in `MCP_OWNER_EMAIL`, with a verified email, gets
a token. Everyone else is refused after the Google step.

Public endpoint: `https://wallapop.mergoth.synology.me/mcp`

```
claude.ai → router :443 → DSM reverse proxy (TLS) → 127.0.0.1:8080 → gateway → wallapop-mcp
```

**One-time setup**

1. Google Cloud Console → APIs & Services → Credentials → Create OAuth client ID →
   *Web application*. Authorized redirect URI:
   `https://wallapop.mergoth.synology.me/oauth/google/callback`. The OAuth consent screen
   can stay in *Testing* with your account as the only test user.
2. DSM → Control Panel → Login Portal → Advanced → Reverse Proxy → Create: source
   `HTTPS wallapop.mergoth.synology.me:443`, destination `HTTP localhost:8080`. Under
   Custom Header add *WebSocket*; under Advanced Setting set the proxy read timeout to 86400.
   The existing `*.mergoth.synology.me` certificate covers it.
3. Run `./scripts/configure-nas-oauth.sh`: it prompts for the values, writes `.env` on
   the NAS and starts the container. Or write `.env` beside `docker-compose.yml` (mode 600)
   by hand:

   ```sh
   MCP_RESOURCE_URL=https://wallapop.mergoth.synology.me/mcp
   GOOGLE_CLIENT_ID=....apps.googleusercontent.com
   GOOGLE_CLIENT_SECRET=...
   MCP_OWNER_EMAIL=you@gmail.com
   # Optional: static bearer for Claude Code. Leave empty for OAuth only.
   MCP_AUTH_TOKEN=
   ```

4. `mkdir -p data state && chmod 700 state`, put `session.json` in `data/`, then
   `docker compose up -d --build`.

| Variable | Notes |
| --- | --- |
| `MCP_RESOURCE_URL` | The MCP URL exactly as clients enter it, `/mcp` included. claude.ai requires an exact match. |
| `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` | The Google OAuth web client from step 1. |
| `MCP_OWNER_EMAIL` | The one Google account allowed in, compared case-insensitively. |
| `MCP_AUTH_TOKEN` | Optional static bearer. Never expires, so it is the weakest credential; leave empty when unused. |
| `WALLAPOP_MCP_ARGS` | Extra arguments for the binary, e.g. `--allow-writes`. Writes stay off by default. |

OAuth clients and token digests (never tokens themselves) live in `state/oauth-state.json`.
Access tokens last 1 hour; refresh tokens last 90 days, rotate on every use, and a replayed
refresh token revokes its whole chain. To sign every client out, delete that file and restart.

**Security note:** keep the port bound to `127.0.0.1` and never forward 8080 on the router.
The reverse proxy is the only way in.

**Connecting** — claude.ai: Settings → Connectors → Add custom connector → URL
`https://wallapop.mergoth.synology.me/mcp`, then *Connect* and sign in with Google. The
connector then also works in Claude Desktop and the mobile apps. Claude Code can use the
same OAuth flow, or the static bearer:

```sh
claude mcp add --transport http wallapop https://wallapop.mergoth.synology.me/mcp \
  --header "Authorization: Bearer <MCP_AUTH_TOKEN>"
```

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
(cd gateway && uv run pytest -q && uv run ruff check .)
```

Tests use synthetic fixtures, local HTTP servers and the real executable's stdio.
They never contact Wallapop or read a developer's session. See
[architecture](docs/ARCHITECTURE.md), [performance](docs/PERFORMANCE.md) and
[contributing](CONTRIBUTING.md).

Experimental and unofficial: offline tests establish known API contracts, not
current live account compatibility. No checkout or payments. The binary has no HTTP
server of its own; the optional `gateway/` adds one for remote use.
The former browser-only helpers are replaced by session/cookie import; see the
migration guide for compatibility details.

MIT. Based on the endpoint research and behavior of
[figurophobia/wallapop-cli](https://github.com/figurophobia/wallapop-cli), with the
seller reply improvement reviewed in
[hugoocoto's python-only-mcp branch](https://github.com/hugoocoto/wallapop-cli/tree/python-only-mcp).
Original history and [credits](CREDITS.md) are preserved.
