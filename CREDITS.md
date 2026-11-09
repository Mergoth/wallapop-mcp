# Credits

This project is based on
[figurophobia/wallapop-cli](https://github.com/figurophobia/wallapop-cli), created
by [figurophobia](https://github.com/figurophobia). Its API research, behavioral
contracts, regression scenarios and original Git history are preserved.

The Rust rewrite replaces the Python CLI/library and Go MCP server with one
shared implementation. It preserves seller-safe conversation replies from
[hugoocoto's API fixes](https://github.com/jrrome/wallapop-mcp/commit/29f59a9).
The reference revision and compatibility changes are recorded in
[the migration guide](docs/MIGRATION.md).

Maintained at [jrrome/wallapop-mcp](https://github.com/jrrome/wallapop-mcp).
Attribution does not imply endorsement or collaborator status.
Dependencies retain their respective authors and licenses. See `Cargo.lock`.
