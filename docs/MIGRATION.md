# Migrating to Rust

The reference for this rewrite is `bc14906a39cd29459378489dd6209df4f4caf4c0`,
including the API fixes in `29f59a9` for publication color, complete edit payloads
and seller replies. Previous Python and Go sources remain in Git history.

Build with `cargo build --release --locked`. Replace the old Go binary or
`python3 wallapop.py` invocation with `target/release/wallapop-mcp`. MCP is the
default command. The same executable provides the CLI and a Rust library, sharing
one API implementation.

## Capability mapping

| Previous capability | Rust equivalent |
| --- | --- |
| Go MCP tools | Same 22 names, plus `find_deals` and `list_own_items` |
| Search, item details, seller reviews, categories | `search`, `show`, `seller`, `categories` |
| Inbox, conversation and unread counts | `replies`, `replies --conversation ID`, `unread` |
| Purchases and sales | `purchases`, `selling`; both accept `--finished` |
| Offer history | `offer status [CONVERSATION_ID]`; `--last 10` limits the inbox summary |
| Send/respond/counter offers | `offer ITEM --price 19.95`, `offer accept/decline/counter ID` |
| Percentage offers and deal automation | `--pct`, `--offer-pct`, templates and `--dry-run` |
| Buyer messages and seller replies | `message ITEM --text TEXT`, `message --conversation ID --text TEXT` |
| Favorites, editing, reservation, sold state, deletion | `favorite`, `edit`, `reserve`, `sold`, `delete --yes` |
| API publication | `publish PHOTOS_DIR`; up to ten sorted JPEGs, with partial photo progress |
| Email/password login | `login EMAIL --password-stdin` |
| Browser session/cookie login | `login --cookies-file FILE`; Netscape, cookie header or JSON formats |
| Direct Python/Go library calls | Rust `Client::invoke`; see [API.md](API.md) |

All mutations require `--allow-writes`, including CLI automation. Tool JSON
prices remain exact decimal strings; CLI prices remain decimal flags. Global
options work before or after the command; option values are literal text.
Duplicate options, conflicting aliases and extra arguments fail. `--` ends option
parsing.

Offer history without an ID summarizes at most 30 inbox threads and remains
incomplete. Supply a conversation ID for its available full thread; automatic
item-to-thread resolution is not performed. An automatic offer with `--msg`
attaches the rendered message to that offer as one action.

## Sessions

Existing session directories are accepted explicitly:

```sh
wallapop-mcp --session ~/.config/wallapop_cli session
wallapop-mcp --session ~/.config/wallapop_cli replies
```

`session.json` takes precedence over legacy `credentials.json`. Token aliases
in the root, `data` and `cookies` are understood. Environment tokens override
file tokens. Refreshed credentials are saved atomically; Unix files use mode 0600.
Importing a new account or logging in replaces the previous credentials. Logout
clears both session and legacy credential files.

Only login/logout select `~/.config/wallapop_cli/session.json` by default. Other
commands require `--session`, `WALLAPOP_SESSION_PATH` or environment tokens.
`credential_present` recognizes cookie-only and refresh-only sessions.
`logged_in` means an access token is present; `session_verified` remains false
because local inspection performs no account verification.

## Workflow changes

The executable does not launch a browser, extract Firefox profile databases or
run Playwright wizards/fallbacks. Import an existing session or browser-exported
cookies instead. API publication accepts JPEGs; PNG conversion is not included.
Publication requires category, description, condition, color and explicit shipping
consent, and validates every photo before creating the listing.

JSON is the default CLI output. CSV and quiet URLs are available for listing
collections. Watch keeps a bounded set of seen IDs within the current process.
Automatic contact also reuses `contacted.json` beside an explicitly configured
session, or `--contacted-file PATH`. The legacy JSON array is accepted. Attempts
are saved before sending so uncertain deliveries stay deduplicated across restarts;
dry runs leave history unchanged. Both histories are bounded to 10,000 IDs.

## Verification

`cargo test --locked` covers API fixtures, real stdio/CLI processes, sessions and
local HTTP behavior: partial publication, exact prices, token refresh, seller
replies, cancellation and non-replayed uncertain writes. It does not establish
current live account compatibility. Tests use no real credentials or account
mutations. See [USAGE.md](USAGE.md), [MCP.md](MCP.md) and
[PERFORMANCE.md](PERFORMANCE.md) for current commands, limits and measurements.
