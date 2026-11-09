# Architecture

The executable has two frontends, MCP stdio and CLI, sharing `Client::invoke`.
That boundary validates the tool contract, enforces write capability and invalidates
read caches around mutations. Domain modules perform API operations; the transport
owns connection, retry, pacing and memory policies.

| Module | Responsibility |
| --- | --- |
| `src/protocol.rs` | JSON-RPC envelopes, lifecycle, discovery, bounded workers, cancellation, structured results |
| `src/tools.rs` | Tool definitions and the limited JSON Schema vocabulary they use |
| `src/client.rs` | Client configuration, validated dispatch, common API requests |
| `src/client/session.rs` | Explicit session loading, token refresh, login, private atomic persistence |
| `src/client/search.rs` | Category normalization, modern/legacy search, bounded pagination, item URL resolution |
| `src/client/models.rs` | Pure response normalization, conversation order, offer history, transactions and deal ranking |
| `src/client/reads.rs` | Read operation dispatch |
| `src/client/writes.rs` | Simple mutations and write dispatch |
| `src/client/messaging.rs` | Existing-thread replies, duplicate guard, PubNub acknowledgement |
| `src/client/publishing.rs` | Photo preflight, multipart publication, partial results, complete edit payloads |
| `src/transport.rs` | HTTP/TLS, read retries, pacing, LRU response cache and identical-read gates |
| `src/cli/` | Argument mapping, session import, deal automation and output |
| `src/cli/contacts.rs` | Bounded persistent contact attempts, saved before sending |
| `tests/` | API behavior, transport invariants, executable MCP/CLI integration |

There is no browser runtime, interpreter, asynchronous runtime, database or
framework dependency. Native worker threads allow ping and cancellation input to
remain responsive while an operation is doing blocking HTTP I/O. Capacity is
bounded; a full server returns `capacity_exceeded` instead of creating a queue.

Cancellation stops waits and subsequent requests. An HTTP exchange already in
progress is bounded by the remaining tool timeout; the blocking client does not
abort its socket immediately on cancellation. Cancelled MCP calls emit no result.

The response cache is per client/account, keyed by the full request including its
headers. Entries expire after 30 seconds for public reads, five seconds for
account reads and an hour for categories. `fresh` bypasses stored responses.
Identical cacheable reads wait on one gate; fresh reads run independently and
mutations never allocate cache keys or gates. Concurrent token refreshes share
a lock. Generation checks prevent an old read from repopulating an invalidated
cache after a mutation.

Mutation errors distinguish a definite rejection from an unknown outcome. The
transport does not retry writes; a definitive 401 may be repeated once after a
changed token. A partial publication keeps the created ID and failed photo
indices, while a failed offer keeps its generated ID. No checkout is implemented.

Schemas deliberately use a small vocabulary implemented in `tools.rs`: objects,
required properties, scalar types, enums, ranges, string/array limits and exact
price validation. This is not a general-purpose JSON Schema implementation.
