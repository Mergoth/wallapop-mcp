# Contributing

Use Rust 1.85 or newer. Prefer small modules with one responsibility, explicit
control flow, clear names and existing dependencies. Fewer lines are useful when
they remove duplication; compressed expressions and clever abstractions are not.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

Add behavioral regressions using synthetic fixtures or a local HTTP server.
Tests must not contact Wallapop, read real credentials or mutate an account.
The integration suite exercises the executable's actual MCP stdin/stdout contract.
Keep assertions on observable behavior rather than implementation structure.

API work belongs in `src/client/`, transport policy in `src/transport.rs`, MCP
framing and lifecycle in `src/protocol.rs`, tool contracts in `src/tools.rs` and
CLI ergonomics in `src/cli/`. Both frontends must use `Client::invoke`.

Reserve stdout for protocol/command output. Bound network and file reads, preserve
resource IDs after partial failures, never replay uncertain mutations, and keep
credentials and upstream bodies out of errors. Document live checks separately
from offline test results. Preserve the upstream history, license and credits.
