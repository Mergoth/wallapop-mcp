#!/usr/bin/env bash
# Deploy the wallapop-mcp Docker image to the Synology NAS.
#
# Usage: ./scripts/deploy-nas.sh
#
# Packs the source tree, ships it via tar+SSH (rsync is blocked on the NAS),
# then builds and restarts the container there.

set -euo pipefail

NAS_HOST="192.168.1.99"
NAS_PORT="2300"
NAS_USER="vlad"
DOCKER="/usr/local/bin/docker"
REMOTE_DIR="~/wallapop-mcp"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

echo "==> Running tests before deploy..."
cargo test --quiet
echo "==> Tests passed."

echo "==> Sending source to NAS..."
tar -C "${REPO_ROOT}" -czf - \
    Cargo.toml Cargo.lock \
    src \
    gateway \
    docker-compose.yml \
    Dockerfile \
    Makefile |
    ssh -p "${NAS_PORT}" "${NAS_USER}@${NAS_HOST}" \
        "cd ${REMOTE_DIR} && tar xzf -"

echo "==> Building image on NAS (Rust compiles there — Apple Silicon can't cross-compile)..."
ssh -p "${NAS_PORT}" "${NAS_USER}@${NAS_HOST}" \
    "cd ${REMOTE_DIR} && ${DOCKER} build --no-cache -t wallapop-mcp:latest ."

echo "==> Restarting container..."
ssh -p "${NAS_PORT}" "${NAS_USER}@${NAS_HOST}" \
    "cd ${REMOTE_DIR} && ${DOCKER} compose up -d --force-recreate"

echo "==> Done. wallapop-mcp is live at https://wallapop.mergoth.synology.me/mcp"
