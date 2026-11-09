CARGO ?= cargo

.PHONY: build test check benchmark
build:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --locked

check:
	$(CARGO) fmt --check
	$(CARGO) clippy --locked --all-targets -- -D warnings
	$(CARGO) test --locked

benchmark: build
	$(CARGO) run --release --locked --example footprint -- target/release/wallapop-mcp
