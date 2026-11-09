# CLI usage

The executable defaults to MCP. Use a subcommand for CLI output. Run `--help` for
all flags and `--allow-writes tools` for exact tool schemas.

```sh
wallapop-mcp search 'nintendo switch' --city madrid --order price_asc --max-price 200 --ship-only
wallapop-mcp search 'camera' --lat 41.38 --lon 2.17 --pages 2 --limit 40 --csv
wallapop-mcp deals 'camera' --min-discount 0.2 --top 5
wallapop-mcp watch 'camera' --interval 300 --once
wallapop-mcp show ITEM
wallapop-mcp seller USER
wallapop-mcp --session ./session.json replies --conversation THREAD
wallapop-mcp --session ./session.json purchases --finished
wallapop-mcp --session ./session.json items
wallapop-mcp --session ./session.json offer status --last 10
wallapop-mcp --session ./session.json offer status THREAD
```

Global options work before or after the command. Option values are literal text;
`--` ends option parsing. Repeated options and conflicting aliases fail.

The complete city/region presets are embedded in `src/cli/cities.json`. Search
orders accept relevance, newest, price_asc, price_desc and closest. JSON is the
default output; searches and deals also support CSV and quiet URL output.

## Authentication

Sessions are read only when explicitly configured, with environment tokens taking
precedence. A session directory merges `session.json` and legacy `credentials.json`.

```sh
wallapop-mcp --session ./session.json login --cookies-file ./cookies.txt
wallapop-mcp --session ./session.json login --token-stdin
wallapop-mcp --session ./session.json login you@example.com --password-stdin
wallapop-mcp --session ./session.json session
wallapop-mcp --session ./session.json logout
```

The two stdin login modes read a token or password privately from standard input;
finish the input stream after entering it. Cookie import accepts Netscape files,
a cookie header or a JSON session. Login/logout default to
`~/.config/wallapop_cli/session.json` when no path is supplied. Other commands
never implicitly load it. Logout clears both session and legacy credential files.
A cookie-only session refreshes on its first authenticated operation.

## Writes

Every write requires `--allow-writes`. `--dry-run` prints a write's planned tool
and arguments. Seller replies use an existing conversation and do not create a
conversation for the seller's own item.

```sh
wallapop-mcp --allow-writes --session ./session.json message ITEM --text 'Hola'
wallapop-mcp --allow-writes --session ./session.json message --conversation THREAD --text 'Sí, disponible'
wallapop-mcp --allow-writes --session ./session.json offer ITEM --price 19.95
wallapop-mcp --allow-writes --session ./session.json offer accept OFFER_ID
wallapop-mcp --allow-writes --session ./session.json offer counter OFFER_ID --price 22.50
wallapop-mcp --allow-writes --session ./session.json edit ITEM --price 25.50
wallapop-mcp --allow-writes --session ./session.json reserve ITEM --remove
wallapop-mcp --allow-writes --session ./session.json sold ITEM --undo
wallapop-mcp --allow-writes --session ./session.json delete ITEM --yes
wallapop-mcp --allow-writes --session ./session.json publish ./photos --summary 'Camera' --description 'Works well' --price 25.50 --category-leaf-id 10088 --condition good --color blue --shipping-allowed false
```

Publish includes all JPEGs in the directory in sorted filename order (up to ten),
not just the first photo. For shipping, add `--shipping-allowed true
--max-weight-kg 2` and optionally `--cost-configuration-id ID`.

Deals/watch support `--offer-pct 0.8`, `--msg 'Hola, me interesa {title}'` and
`--dry-run`. Combining `--offer-pct` and `--msg` attaches the rendered message
to the offer as one action. Watch polls at least every five seconds, deduplicates up to 10,000
IDs during its process lifetime and stops after `--once` when requested.
Automatic contact uses `contacted.json` beside an explicit session, or a path
supplied with `--contacted-file`. Existing history is reused. Attempts are saved
before sending, including attempts whose delivery later becomes uncertain.
Dry runs do not save attempts. The file is bounded to 10,000 IDs; preserve it
across restarts to avoid repeating offers or messages.
Automatic actions are opt-in and require write capability. Do not rerun a failed
mutation until its state is checked.

## Any tool

```sh
wallapop-mcp call search '{"keywords":"camera","max_price":"100.00"}'
wallapop-mcp --session ./session.json call get_offer_status '{"conversation_id":"THREAD"}'
wallapop-mcp --allow-writes call publish_item -
```

`call TOOL -` reads JSON from stdin. Unknown commands, options, fields and invalid
limits fail. Errors go to stderr and produce a nonzero exit status; partial
resource information is printed with the error when present.
