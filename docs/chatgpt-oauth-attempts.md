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

## Untried, if #7 isn't enough

- RFC 9207: add `iss` to every authorization response and advertise `authorization_response_iss_parameter_supported: true`. That changes the redirect URI to `https://chatgpt.com/connector_platform_oauth_redirect`.
