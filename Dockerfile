FROM rust:1.85-slim-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM python:3.13-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# UID/GID match the NAS user owning ./data, so the session refresh can write.
ARG UID=1026
ARG GID=100
RUN (getent group "${GID}" >/dev/null || groupadd --gid "${GID}" mcp) \
    && useradd --create-home --uid "${UID}" --gid "${GID}" mcp \
    && install -d -o "${UID}" -g "${GID}" /home/mcp/server

ARG GIT_SHA=""
RUN echo "${GIT_SHA}" > /etc/wallapop-mcp-sha

COPY --from=ghcr.io/astral-sh/uv:0.8 /uv /usr/local/bin/uv
COPY --from=builder /build/target/release/wallapop-mcp /usr/local/bin/wallapop-mcp

USER mcp
WORKDIR /home/mcp/server
ENV UV_COMPILE_BYTECODE=1 UV_LINK_MODE=copy
COPY --chown=${UID}:${GID} server/pyproject.toml server/uv.lock ./
COPY --chown=${UID}:${GID} server/vendor ./vendor
RUN uv sync --frozen --no-dev --no-install-project --no-editable
COPY --chown=${UID}:${GID} server/src src/
RUN uv sync --frozen --no-dev --no-editable

# The venv interpreter directly: `uv run` needs a writable cache, and the rootfs is read-only.
ENV PATH="/home/mcp/server/.venv/bin:${PATH}" \
    PYTHONUNBUFFERED=1 \
    WALLAPOP_HOST=0.0.0.0 \
    WALLAPOP_PORT=8000 \
    WALLAPOP_SESSION_PATH=/data/session.json

VOLUME ["/data"]
EXPOSE 8000
ENTRYPOINT ["python", "-m", "wallapop_server"]
