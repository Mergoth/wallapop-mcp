FROM rust:1.85-slim-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN RUST_MIN_STACK=16777216 cargo build --release --locked

FROM node:20-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
RUN npm install -g supergateway
COPY --from=builder /build/target/release/wallapop-mcp /usr/local/bin/wallapop-mcp
ENV WALLAPOP_SESSION_PATH=/data/session.json
VOLUME /data
EXPOSE 8080
# MCP_AUTH_TOKEN is required — container refuses to start without it
ENV MCP_AUTH_TOKEN=""
CMD sh -c 'exec supergateway --port 8080 --bearer "$MCP_AUTH_TOKEN" --stdio wallapop-mcp'
