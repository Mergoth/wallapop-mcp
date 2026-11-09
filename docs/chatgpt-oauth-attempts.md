# ChatGPT connector OAuth — attempts log

Read this before every new attempt. Add a row after every deploy + test, with the evidence.

Setup: ChatGPT → `https://wallapop.mergoth.synology.me/mcp` → Python gateway (MCP SDK OAuth +
Google sign-in for the single owner) → Rust MCP over stdio. Claude.ai connects fine.

Flow (browser): `/authorize` → 302 `/login?txn` (consent page) → click = form POST `/login` →
302 Google → 302 `/oauth/google/callback` → 302 `https://chatgpt.com/connector/oauth/AGXFn6Knmk2c?code&state`
→ ChatGPT backend should call `POST /token`.

## Attempts

| # | Hypothesis | Change | Result | Verdict |
|---|------------|--------|--------|---------|
| 1 | ChatGPT discovers metadata via path-appended URL | `/mcp/.well-known/*` → `/.well-known/*` alias middleware | Discovery 200 | Kept (harmless) |
| 2 | ChatGPT needs OIDC discovery | `/.well-known/openid-configuration` aliased to RFC 8414 metadata | No change in outcome | Kept (docs allow either) |
| 3 | ChatGPT shows the sign-in link late, txn expires | `LOGIN_TTL` raised to 30 min | No change in outcome | Kept (harmless) |
| 4 | ChatGPT backend "re-verifies" with `POST /login` after callback | `_completed_logins`: completed txn returns 200 "Sign-in complete" instead of 400 | Still no `POST /token` | Wrong theory. The extra `POST /login` is the user's 2nd click (see #7) |
| 5 | Re-verify POST must 302 to the callback with the code | `POST /login` for completed txn → 302 to callback URL | Never actually deployed (NAS build still running, then killed) | Wrong theory; reverted |
| 6 | `POST /token` → 401 from bad client auth | Logged `/token` calls (grant, client_id known?, auth header, SDK error body) | After a real attempt: **no `POST /token` at all**. Registered ChatGPT clients are all correct (`none`, no secret, right redirect_uri) | The one old 401 wasn't ChatGPT's flow. Logger kept |
| 7 | Browser never reaches chatgpt.com | User's DevTools: chain stops after `callback?state=…` 302; page stays on consent. Root cause: consent page CSP `form-action 'self' https://accounts.google.com`. Browsers apply `form-action` to every redirect after a form POST, so the final hop to `chatgpt.com` is blocked. This also explains "had to click twice" (1st click silently blocked; 2nd re-POSTs a spent txn → 400) | Consent page CSP now adds the client's redirect_uri origin to `form-action` | _pending: deploy + retest_ | |

## Verified facts (don't re-check)

- Metadata matches OpenAI's checklist (https://developers.openai.com/plugins/build/auth): issuer equals `authorization_servers[0]`, S256, `none` auth method, `registration_endpoint`.
- ChatGPT uses DCR and registers a new client on every retry (`token_endpoint_auth_method=none`).
- Redirect URI is `https://chatgpt.com/connector/oauth/AGXFn6Knmk2c` (callback-ID form, because we don't advertise RFC 9207 `iss`).
- OpenAI docs have no "re-verify" step; ChatGPT only exchanges the code at `POST /token`.
- Deploy pitfall: `deploy-nas.sh` can "succeed" while the image is stale. After each deploy, grep the running container for the new code.

## Gemini

What each client expects, from its docs:
- **Gemini CLI**: DCR + PKCE, redirect `http://localhost:<random-port>/oauth/callback`; RFC 9207 `iss` required only if the server advertises it (we don't). Should work as is. Config: `{"mcpServers": {"wallapop": {"httpUrl": "https://wallapop.mergoth.synology.me/mcp"}}}`.
- **Gemini app** (gemini.google.com → Settings → Connectors → + Custom): takes a Client ID + secret. The redirect URI is per user, `https://oauth-redirect.googleusercontent.com/r/user_bound_custom-mcp-<google-user-id>-<host>`; copy it from "Additional settings → Show more". Not documented: whether it attempts DCR, and which token auth method it uses.
- **Gemini Enterprise**: fixed redirect `https://vertexaisearch.cloud.google.com/oauth-redirect`, admin-entered client ID/secret.

| # | Hypothesis | Change | Result | Verdict |
|---|------------|--------|--------|---------|
| G1 | Gemini app/Enterprise need a pre-registered confidential client, and may send the secret as Basic or form | `scripts/register-oauth-client.sh <name> <redirect-uri>`; `/token` accepts the secret either way (SDK alone rejects Basic without form client_id); Google redirect hosts trusted on consent page | The Gemini app **does DCR by itself** (client_name "Google"), so the script isn't needed for it. Same symptom as ChatGPT #7/#8 | Kept; not the blocker |

## After #7 (2026-11-09): same symptom with Gemini app and ChatGPT

| # | Hypothesis | Check | Result | Verdict |
|---|------------|-------|--------|---------|
| 8a | Wrong Google account | Server log | `owner signed in` on every attempt; a non-owner gets a 403 page, never "Sign-in complete" | Ruled out |
| 8b | #7 CSP not live | Live `/login` headers for the Gemini client | `form-action 'self' https://accounts.google.com https://oauth-redirect.googleusercontent.com` | Fix is live |
| 8c | Chromium still blocks the chain even with the right CSP | Local replica (consent → form POST → "google" → callback → "client" on 3 sites), headless Chromium 1248, `scratchpad/csp-repro` | Old CSP: **blocked** (`form-action` console error). New CSP: **reaches client**. No CSP: reaches client. Link instead of form: reaches client | Server-side chain is correct now |
| 8d | Client redirect URI forwards on to another host that form-action blocks | `curl` both redirect URIs **without** code/state | Both 200 | **Wrong check**: with `?code=…` the Gemini URI answers 302 (see 8e) |
| 8e | Find where the chain stops | User DevTools capture | `…/user_bound_custom-mcp-…?code=… 302`, then nothing. The client's redirector forwards onward, and that hop is still in the form-POST redirect chain, so `form-action` blocks it | **Root cause**: no allowlist can name every client's onward hosts |
| 9 | Stop starting the chain with a form submit | Consent button is a link (`GET /login?txn=…&go=1`, same SameSite=Strict cookie check); CSP `form-action 'none'`; #7's per-client form-action removed | Replica with an onward 302: form + #7 CSP **blocked**, link **reaches the end**. 38 tests pass | **Fixed** (2026-11-09): user confirms one click connects |

## Untried, if #7 isn't enough

- RFC 9207: add `iss` to every authorization response and advertise `authorization_response_iss_parameter_supported: true`. That changes the redirect URI to `https://chatgpt.com/connector_platform_oauth_redirect`.
