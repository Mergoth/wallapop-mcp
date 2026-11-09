# MCP tools

Start `wallapop-mcp` without a subcommand. The server speaks newline-delimited
JSON-RPC 2.0 over stdio. It negotiates MCP 2024-11-05, 2025-03-26 or 2025-06-18;
an unsupported version receives 2025-06-18. Initialize, then send
`notifications/initialized` before tool requests. Batches are not supported.

`tools/list` returns 14 read tools, or 24 total with `--allow-writes`. Every schema
rejects unknown fields. All read tools except `session_status` support `fresh`.

| Read tool | Required arguments |
| --- | --- |
| `session_status` | None; local only |
| `search` | `keywords` |
| `find_deals` | `keywords`; optional `min_discount` and `top` |
| `get_item` | `item_id`: internal hash, slug or canonical item URL |
| `get_seller` | `user_id`; optional `include_reviews` |
| `list_categories` | None |
| `list_own_items` | None; optional `page`, `limit` |
| `list_inbox` | None; optional `page_size`, `max_messages` |
| `get_conversation` | `conversation_id` |
| `unread_count` | None |
| `list_purchases` | Optional `tab`: ongoing or finished |
| `list_sales` | Optional `tab`: ongoing or finished |
| `get_offer_constraints` | `item_id` |
| `get_offer_status` | Optional `conversation_id`; otherwise a summary of up to 30 inbox threads, limited by `last` |

| Write tool | Required arguments |
| --- | --- |
| `set_favorite` | `item_id`, `favorited` |
| `send_message` | `message` and exactly one of `item_id`, `conversation_id` |
| `send_offer` | `item_id`, `amount`; optional `message` |
| `respond_offer` | `offer_id`, `action`: accept or decline |
| `counter_offer` | `offer_id`, `amount`; optional currency EUR |
| `publish_item` | `title`, `description`, `price`, `category_leaf_id`, `condition`, `photo_paths`, `color`, `shipping_allowed` |
| `edit_item` | `item_id` plus the fields to change |
| `reserve_item` | `item_id`, `reserved` |
| `mark_sold` | `item_id`, `sold` |
| `delete_item` | `item_id` |

Search supports category/subcategory, condition, min/max price, shipping, sort,
coordinates, up to five pages and up to 50 results. Latitude and longitude must be
supplied together. The category tree resolves nested categories to search roots.
Repeated pagination cursors are errors. Search returns compact listings; get
individual details when needed. Deals exclude unavailable and zero-priced items
and compare against the median of the retrieved sample, not a market estimate.

All monetary inputs are decimal strings with at most two fractional digits.
Publish requires 1–10 regular JPEG files, max 8 MiB each and 32 MiB combined.
Shipping requires explicit weight and a supplied or existing account cost
configuration. No location, shipping consent, picture ID or price is guessed.

Successful calls have text content and an object `structuredContent`. Tool errors
have `isError: true` and a structured error code, HTTP status and
`outcome_unknown`. Partial errors preserve resource IDs in `partial_result`.
Publication can also return `partial: true` with known failed photo positions;
inspect the existing listing before retrying. Negotiation events are never
inferred from ordinary chat messages. Conversation history is marked incomplete.

Input is limited to 64 KiB per frame and arguments to 32 KiB. An oversized frame
returns an error and closes the stream. Tool payloads are limited to 128 KiB.
At worker capacity, retry a read after another call completes. Cancellation
notifications suppress the cancelled response; see the architecture's I/O limits.
