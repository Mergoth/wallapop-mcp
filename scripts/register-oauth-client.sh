#!/usr/bin/env bash
# Pre-register a confidential OAuth client, for connectors that ask for a Client ID and secret
# instead of registering themselves (Gemini app custom connectors, Gemini Enterprise).
#
# Usage: ./scripts/register-oauth-client.sh "<name>" "<redirect URI copied from the connector>"
#   Gemini app:        https://oauth-redirect.googleusercontent.com/r/user_bound_custom-mcp-...
#   Gemini Enterprise: https://vertexaisearch.cloud.google.com/oauth-redirect
# Override the server with MCP_BASE_URL (default: https://wallapop.mergoth.synology.me).

set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 <name> <redirect-uri>" >&2
    exit 2
fi

BASE_URL="${MCP_BASE_URL:-https://wallapop.mergoth.synology.me}"

payload="$(python3 -c 'import json,sys; print(json.dumps({
    "client_name": sys.argv[1],
    "redirect_uris": [sys.argv[2]],
    "token_endpoint_auth_method": "client_secret_post",
    "grant_types": ["authorization_code", "refresh_token"],
    "response_types": ["code"],
    "scope": "wallapop",
}))' "$1" "$2")"

curl -fsS -X POST "${BASE_URL}/register" -H 'Content-Type: application/json' -d "${payload}" |
    python3 -c 'import json,sys; d=json.load(sys.stdin); print("Client ID:     " + d["client_id"]); print("Client secret: " + d["client_secret"])'
