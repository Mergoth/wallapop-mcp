#!/usr/bin/env bash
# Build and vendor mcp-gateway wheel into server/vendor/.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SERVER_DIR="${REPO_ROOT}/server"
VENDOR_DIR="${SERVER_DIR}/vendor"
MCP_GATEWAY_SRC="${MCP_GATEWAY_SRC:-$HOME/work/mcp-gateway}"

echo "==> Building wheel from ${MCP_GATEWAY_SRC}..."
(cd "${MCP_GATEWAY_SRC}" && uv build --wheel)

mkdir -p "${VENDOR_DIR}"
rm -f "${VENDOR_DIR}"/mcp_gateway-*.whl

WHEEL="$(ls -t "${MCP_GATEWAY_SRC}"/dist/mcp_gateway-*.whl | head -1)"
WHEEL_NAME="$(basename "${WHEEL}")"
cp "${WHEEL}" "${VENDOR_DIR}/"

echo "==> Copied ${WHEEL_NAME} to ${VENDOR_DIR}/"
echo "Ensure server/pyproject.toml has:"
echo "[tool.uv.sources]"
echo "mcp-gateway = { path = \"vendor/${WHEEL_NAME}\" }"

echo "==> Running uv lock in server/..."
(cd "${SERVER_DIR}" && uv lock)
