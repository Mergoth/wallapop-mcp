#!/usr/bin/env bash
# Refresh the Wallapop session cookie on the NAS deployment.
#
# Usage: ./scripts/refresh-nas-cookie.sh
#
# In Chrome, go to es.wallapop.com, open DevTools Console, run:
#   copy(document.cookie)
# Then run this script and paste when prompted.

set -euo pipefail

NAS_HOST="192.168.1.99"
NAS_PORT="2300"
NAS_USER="vlad"
NAS_SESSION='~/wallapop-mcp/data/session.json'
LOCAL_SESSION="${HOME}/.config/wallapop_cli/session.json"
BINARY="$(dirname "$0")/../target/release/wallapop-mcp"

echo "Paste cookies from Chrome DevTools Console (copy(document.cookie)), then press Enter:"
IFS= read -r COOKIES

if [[ -z "$COOKIES" ]]; then
  echo "No cookies provided, aborting." >&2
  exit 1
fi

# Update local session first
"$BINARY" --session "$LOCAL_SESSION" login --cookies "$COOKIES"
echo "Local session updated."

# Push updated session to NAS
ssh -p "$NAS_PORT" "${NAS_USER}@${NAS_HOST}" "cat > $NAS_SESSION" < "$LOCAL_SESSION"
echo "NAS session updated at ${NAS_HOST}:${NAS_SESSION}"

# Restart container so it picks up the new session immediately
ssh -p "$NAS_PORT" "${NAS_USER}@${NAS_HOST}" \
  "cd ~/wallapop-mcp && /usr/local/bin/docker compose restart"
echo "Container restarted. MCP is live at https://wallapop.mergoth.synology.me/mcp"
