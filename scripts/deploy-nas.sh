#!/usr/bin/env bash
# Deploy the wallapop-mcp Docker image to the Synology NAS.
#
# Usage: ./scripts/deploy-nas.sh [--dry-run]
#
# Packs the source tree, ships it via tar+SSH (rsync is blocked on the NAS),
# then builds and restarts the container there.

set -euo pipefail

NAS_HOST="blacknas.local"
NAS_PORT="2300"
NAS_USER="vlad"
DOCKER="${DOCKER:-/usr/local/bin/docker}"
REMOTE_DIR="~/wallapop-mcp"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

GIT_SHA="$(git -C "${REPO_ROOT}" rev-parse HEAD)"

if [[ "${1:-}" == "--dry-run" ]]; then
    echo "[dry-run] Plan:"
    echo "  1. Run unit tests in server/"
    echo "  2. Ship to NAS: Cargo.toml Cargo.lock src server docker-compose.yml Dockerfile Makefile"
    echo "     (excludes: server/.venv server/.pytest_cache server/.ruff_cache __pycache__)"
    echo "  3. Build image on NAS with ${DOCKER} build --build-arg GIT_SHA=${GIT_SHA} -t wallapop-mcp:latest ."
    echo "  4. Restart container with ${DOCKER} compose up -d --force-recreate"
    echo "  5. Verify /etc/wallapop-mcp-sha matches ${GIT_SHA}"
    exit 0
fi

export PATH="$HOME/.cargo/bin:$PATH"

echo "==> Running server tests before deploy..."
(cd "${REPO_ROOT}/server" && uv run --frozen pytest -q)
echo "==> Server tests passed."

echo "==> Running Rust unit tests before deploy..."
cargo test --quiet
echo "==> Rust unit tests passed."

if [ -n "${WALLAPOP_SESSION_PATH:-}" ]; then
    echo "==> Running e2e tests (WALLAPOP_SESSION_PATH is set)..."
    cargo test --test e2e --quiet -- --include-ignored
    echo "==> E2e tests passed."
else
    echo "==> Skipping e2e tests (set WALLAPOP_SESSION_PATH to enable)."
fi

echo "==> Sending source to NAS..."
COPYFILE_DISABLE=1 tar -C "${REPO_ROOT}" -czf - \
    --exclude 'server/.venv' \
    --exclude 'server/.pytest_cache' \
    --exclude 'server/.ruff_cache' \
    --exclude '__pycache__' \
    Cargo.toml Cargo.lock \
    src \
    server \
    docker-compose.yml \
    Dockerfile \
    Makefile |
    ssh -o ServerAliveInterval=5 -p "${NAS_PORT}" "${NAS_USER}@${NAS_HOST}" \
        "cd ${REMOTE_DIR} && tar xzf -"

echo "==> Building image on NAS (Rust compiles there — Apple Silicon can't cross-compile)..."
ssh -o ServerAliveInterval=5 -p "${NAS_PORT}" "${NAS_USER}@${NAS_HOST}" \
    "cd ${REMOTE_DIR} && ${DOCKER} build --build-arg GIT_SHA=${GIT_SHA} --no-cache -t wallapop-mcp:latest ."

echo "==> Restarting container..."
ssh -o ServerAliveInterval=5 -p "${NAS_PORT}" "${NAS_USER}@${NAS_HOST}" \
    "cd ${REMOTE_DIR} && ${DOCKER} compose up -d --force-recreate"

echo "==> Verifying deployment sha..."
REMOTE_SHA="$(ssh -o ServerAliveInterval=5 -p "${NAS_PORT}" "${NAS_USER}@${NAS_HOST}" "${DOCKER} exec wallapop-mcp cat /etc/wallapop-mcp-sha" | tr -d '\r\n')"
if [ "${REMOTE_SHA}" != "${GIT_SHA}" ]; then
    echo "SHA mismatch: expected ${GIT_SHA}, got ${REMOTE_SHA}" >&2
    exit 1
fi
echo "==> Verification passed: remote container is running ${REMOTE_SHA}"

echo "==> Done. wallapop-mcp is live."
