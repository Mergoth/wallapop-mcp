FROM rust:1.85-slim-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM python:3.13-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# UID/GID match the NAS user owning ./data and ./state, so the session refresh can write.
ARG UID=1026
ARG GID=100
RUN (getent group "${GID}" >/dev/null || groupadd --gid "${GID}" mcp) \
    && useradd --create-home --uid "${UID}" --gid "${GID}" mcp \
    && install -d -o "${UID}" -g "${GID}" /home/mcp/gateway

COPY --from=ghcr.io/astral-sh/uv:0.8 /uv /usr/local/bin/uv
COPY --from=builder /build/target/release/wallapop-mcp /usr/local/bin/wallapop-mcp

USER mcp
WORKDIR /home/mcp/gateway
ENV UV_COMPILE_BYTECODE=1 UV_LINK_MODE=copy
COPY --chown=${UID}:${GID} gateway/pyproject.toml gateway/uv.lock ./
RUN uv sync --frozen --no-dev --no-install-project --no-editable
COPY --chown=${UID}:${GID} gateway/src src/
RUN uv sync --frozen --no-dev --no-editable

# The venv interpreter directly: `uv run` needs a writable cache, and the rootfs is read-only.
ENV PATH="/home/mcp/gateway/.venv/bin:${PATH}" \
    PYTHONUNBUFFERED=1 \
    MCP_HOST=0.0.0.0 \
    MCP_PORT=8080 \
    MCP_STATE_DIR=/state \
    WALLAPOP_SESSION_PATH=/data/session.json

VOLUME ["/data", "/state"]
EXPOSE 8080
ENTRYPOINT ["python", "-m", "wallapop_gateway"]
