#!/usr/bin/env bash
# Write the gateway's .env on the NAS and (re)start the container.
#
# Usage: ./scripts/configure-nas-oauth.sh
#
# Prompts for the Google OAuth client and the owner account, so the client secret goes
# straight from this terminal to the NAS and is never stored locally.

set -euo pipefail

NAS_HOST="192.168.1.99"
NAS_PORT="2300"
NAS_USER="vlad"
RESOURCE_URL="https://wallapop.mergoth.synology.me/mcp"

read -r -p "Google OAuth client ID: " CLIENT_ID
read -r -s -p "Google OAuth client secret (hidden): " CLIENT_SECRET
echo
read -r -p "Owner Google account (email): " OWNER_EMAIL
read -r -p "Also issue a static bearer token for Claude Code? [y/N] " WANT_STATIC

if [[ -z "$CLIENT_ID" || -z "$CLIENT_SECRET" || "$OWNER_EMAIL" != *@* ]]; then
  echo "Client ID, secret and a valid email are required." >&2
  exit 1
fi

STATIC_TOKEN=""
if [[ "$WANT_STATIC" =~ ^[Yy] ]]; then
  STATIC_TOKEN="$(openssl rand -hex 32)"
fi

printf 'MCP_RESOURCE_URL=%s\nGOOGLE_CLIENT_ID=%s\nGOOGLE_CLIENT_SECRET=%s\nMCP_OWNER_EMAIL=%s\nMCP_AUTH_TOKEN=%s\n' \
  "$RESOURCE_URL" "$CLIENT_ID" "$CLIENT_SECRET" "$OWNER_EMAIL" "$STATIC_TOKEN" |
  ssh -p "$NAS_PORT" "${NAS_USER}@${NAS_HOST}" \
    'cd ~/wallapop-mcp && umask 077 && cat > .env && mkdir -p data state && chmod 700 state &&
     /usr/local/bin/docker compose up -d --force-recreate'

echo "Gateway restarted. Add the connector in claude.ai with ${RESOURCE_URL}"
if [[ -n "$STATIC_TOKEN" ]]; then
  echo
  echo "Static bearer (shown once; it is also in ~/wallapop-mcp/.env on the NAS):"
  echo "  claude mcp add --transport http wallapop ${RESOURCE_URL} --header \"Authorization: Bearer ${STATIC_TOKEN}\""
fi
