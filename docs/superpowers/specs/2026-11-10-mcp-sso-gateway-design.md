# MCP SSO gateway for the NAS — design

Date: 2026-11-10. Status: draft, awaiting approval.

Lines marked **(call)** are choices made while writing this, not things Vlad decided. Everything
else under "Decided" is his.

## Intent

Today each MCP on the Synology NAS carries its own sign-in: Wallapop has a Python gateway with
Google sign-in baked into its image (`gateway/` on branch `pr-6-rust`), second brain has its own
passphrase OAuth server. Every new MCP would repeat that work, and every client quirk (ChatGPT,
Gemini) would have to be fixed once per MCP.

Build one standalone gateway that owns sign-in for every MCP on the NAS. An MCP behind it holds no
auth code, and adding one needs no gateway change and no gateway restart.

## Decided

- **Extract, don't adopt.** The gateway is extracted from `gateway/` on `pr-6-rust` into its own
  repo. Ready-made options (mcp-front, R0Wi/mcp-gateway, Pomerium, agentgateway + Keycloak) were
  looked at from search results only; none claims ChatGPT and Gemini-app compatibility, which is
  where the existing code needed its fixes (`docs/chatgpt-oauth-attempts.md`, attempts 1–9).
- **Users:** Google sign-in, restricted to a pre-configured list of Gmail addresses. Two to start
  (Vlad, possibly his wife).
- **Clients:** Claude (claude.ai, Cowork, Claude Code), ChatGPT, Gemini (app and CLI), others later.
- **Backends:** second brain and Wallapop first, more to come.
- **Self-registration:** an MCP announces itself to the running gateway. No gateway restart.
- **Later feature:** the gateway passes the signed-in user's email to the MCP.
- **The MCP's URL identifies it everywhere.** A client is given only the MCP's URL, so every
  call it makes — discovery, client registration, sign-in, token, use — carries the `/<name>`
  part of that URL.
- **One rollout.** Gateway, both MCPs and the removal of their old sign-in ship together.
- **Built with AI Factory** (`~/work/ai-factory`): Claude writes briefs, specs and verdicts, `agy`
  writes the code. See the delivery plan next to this file.
- **Gateway ↔ MCP transport:** left to me. HTTP **(call)**, see below.

## Goals

1. One public hostname, one Google sign-in implementation, any number of MCPs.
2. Claude, ChatGPT and the Gemini app each complete the connect flow in one click, as they do
   against the current Wallapop gateway.
3. A new MCP container comes up, registers, and is reachable at `/<name>/mcp` with no gateway
   edit or restart.
4. A stdio-only MCP joins without changing its code.
5. Second brain and Wallapop both run behind it and their old auth code is removed, in the same
   delivery.

## Non-goals (v1)

- Per-user access lists. Every allowed email reaches every registered MCP **(call — this is how I
  read "MCPs should receive the user email": the MCP decides what a user may do, the gateway only
  decides who gets in)**.
- Forwarding the user email to MCPs. v1 only records it on every token so the later feature needs
  no re-sign-in (see "Identity forwarding").
- A static bearer token for header-capable clients (`MCP_AUTH_TOKEN` today). Dropped **(call)**:
  every target client does OAuth, and a bearer valid for all MCPs is a wide key.
- Migrating existing tokens. Each connector is re-added once under its new URL.
- An admin UI, multiple identity providers, aggregating several MCPs into one endpoint.

## Anti-goals

- The gateway must never need the Docker socket. (Label-based discovery was rejected: read access
  to the socket exposes every container's environment, including the Google client secret and
  Wallapop cookies, to an internet-facing process.)
- No MCP port is reachable from outside the Docker network.
- No regression of attempts 1–9 in the OAuth log. That log moves to the new repo and stays the
  rule: read before any auth change, add a row after each deploy + test.

## Shape

```
client ──HTTPS──▶ DSM reverse proxy ──▶ gateway :8080 (public app)
                   mcp.mergoth.synology.me      │  /<name>/authorize /<name>/token /<name>/register
                                                │  /<name>/login  /oauth/google/callback
                                                │  /.well-known/*/<name>…
                                                │  /<name>/mcp ── bearer check ──▶ http://<backend>/mcp
                                                │
                                   gateway :8081 (internal app, Docker network only)
                                                ▲  POST /backends   DELETE /backends/<name>
                                                │
                      second-brain container ───┤   (HTTP MCP, registers at start)
                      wallapop container ───────┘   (stdio MCP wrapped by the bridge)
```

- Repo: `~/work/mcp-gateway`, package `mcp_gateway`, Python ≥ 3.12, `mcp==2.0.0`, `uvicorn`,
  `uv`, `pytest`, `ruff` — same stack as the code it comes from **(call on the name)**.
- Hostname: `mcp.mergoth.synology.me` **(call)**. One DSM reverse-proxy entry → `localhost:8080`,
  with no access-control profile: the browser sign-in steps need public reach, and LAN hairpin
  traffic arrives with the public IP as source.
- Each MCP is its own connector in each client, with its own issuer:
  resource `https://mcp.mergoth.synology.me/<name>/mcp`, issuer
  `https://mcp.mergoth.synology.me/<name>`. All of its OAuth endpoints live under `/<name>/`.
  The one exception is the Google callback, `/oauth/google/callback`, because Google needs a
  single fixed redirect URI; the sign-in transaction it carries names the backend.

## Public app

### Authorization server

Carried over from `oauth.py` / `metadata.py` / `app.py` with these changes:

- **Allowed emails** replace the single owner: `GATEWAY_ALLOWED_EMAILS`, comma-separated,
  lowercased, at least one. The Google `login_hint` is dropped so the account chooser shows.
- **Subject is the email.** Codes, access and refresh records store `subject=<email>` instead of
  `"owner"`. At token check, a subject no longer in the allowed list is rejected, so removing an
  address (config change + restart) cuts that person off immediately.
- **The backend comes from the URL path.** `/<name>/register`, `/<name>/authorize`,
  `/<name>/login` and `/<name>/token` all act for backend `<name>`; an unknown name is 404.
  Registered clients, pending sign-ins, codes and tokens each record their backend and are
  refused under any other name, so a token minted through `/wallapop/…` fails at `/brain/mcp`.
  A `resource` parameter is optional; when sent it must equal that backend's resource URL
  (`invalid_target` otherwise). A client that omits it still gets a correctly bound token.
- The 50-client cap applies per backend.
- **One scope**, `mcp`, advertised, default and required.
- **Consent page** is generic: "Allow *client* to use *backend title*?", page title "MCP gateway".
- Kept unchanged: DCR open with the 50-client cap and eviction rule; PKCE; token digests only on
  disk; refresh rotation with family reuse detection; consent as a **link** with the
  SameSite=Strict txn cookie and CSP `form-action 'none'` (attempt 9); `LOGIN_TTL` 30 min; the
  completed-txn "Sign-in complete" page; `none` advertised as a token auth method; the `/token`
  middleware accepting a client secret as form or Basic; the `/token` shape logger; the trusted
  redirect-host list on the consent page.

### Discovery

All generated per registered backend; unknown name → 404. Nothing is served at the bare
`/.well-known/…` root, because without a name there is no backend to describe.

- Protected resource: `/.well-known/oauth-protected-resource/<name>/mcp` →
  `resource: <base>/<name>/mcp`, `authorization_servers: ["<base>/<name>"]`,
  `scopes_supported: ["mcp"]`.
- Authorization server: `/.well-known/oauth-authorization-server/<name>` → issuer
  `<base>/<name>` and endpoints `<base>/<name>/authorize|token|register`.
- Aliases to the same two documents, generalised from `_McpDiscoveryAlias` (attempts 1–2), for
  clients that build the URL differently: `/<name>/.well-known/oauth-authorization-server`,
  `/<name>/.well-known/openid-configuration`, `/.well-known/openid-configuration/<name>`,
  `/.well-known/oauth-authorization-server/<name>/mcp`, `/<name>/mcp/.well-known/<either>`.

### MCP route

`/<name>/mcp`, all methods:

1. Unknown `<name>` → 404.
2. Missing/invalid/expired bearer, or audience ≠ this backend → 401 with
   `WWW-Authenticate: Bearer resource_metadata="<base>/.well-known/oauth-protected-resource/<name>/mcp"`.
3. Otherwise reverse-proxy the HTTP request to the backend URL and stream the response back
   (SSE included, no buffering, no read timeout on the response body).

The proxy is at the HTTP level **(call)**: the gateway does not parse MCP. Tools, prompts,
resources, notifications and `Mcp-Session-Id` pass through untouched, and the gateway holds no
per-session state. This replaces today's MCP-level re-serving in `proxy.py`.

Header handling: strip the client's `Authorization` and any inbound `X-Gateway-*`; add the
backend's own bearer if it registered one; set `Host` to the backend's; pass the rest.

Backend unreachable → 502 with a JSON-RPC-shaped error body. The registration is kept, so an MCP
restart does not break its connectors.

### Public pass-through paths

A backend may register `public_paths` (list of path prefixes, default empty). `/<name>/<prefix>…`
is then forwarded **without** a bearer check. This exists for Wallapop's photo upload
(`PUT /upload/<ticket>`, authorised by its own HMAC ticket) and is opt-in per backend. Request
body cap 25 MB on these paths **(call)**.

## Internal app (registration)

Second listener, `:8081`, bound inside the container and **not published** to the host. Only
containers on the shared Docker network `mcp-net` can reach it. Every call needs
`Authorization: Bearer $GATEWAY_REGISTRATION_TOKEN` (≥ 32 chars, compared in constant time).

`POST /backends`

```json
{
  "name": "wallapop",
  "url": "http://wallapop-mcp:8000/mcp",
  "title": "Wallapop",
  "upstream_token": "…optional bearer the gateway sends to the backend…",
  "public_paths": ["/upload/"]
}
```

- `name`: `^[a-z][a-z0-9-]{1,30}$`, not `oauth` or `health` (the only top-level paths the
  gateway keeps for itself besides `.well-known`).
- `url`: the backend's full MCP endpoint, `http://` or `https://`. `public_paths` are resolved
  against its origin.
- Idempotent upsert: the same name replaces the previous entry. Response `200` with the public
  resource URL, so a backend can log where it is reachable.
- Takes effect on the next request. Persisted to `<state>/backends.json` (atomic write, dir mode
  700) so a gateway restart keeps the table.

`DELETE /backends/<name>` removes it. Its tokens stop validating; its clients and tokens are
kept on disk until they expire, so re-registering the same name restores its connectors.
`GET /backends` lists names, URLs and titles (never tokens). `GET /health` → 200.

No health-based eviction **(call)**: a backend that is down answers 502 and stays registered.
Backends re-register every 5 minutes, which also heals a gateway whose state was wiped.

## Joining as a backend

The same package ships two commands so no MCP has to write gateway code:

- `mcp-gateway register --name brain --url http://second-brain-mcp:8000/mcp [--title …]
  [--upstream-token-env VAR] [--public-path /x/]` — registers, then re-registers every 5 minutes;
  retries with backoff while the gateway is down. Runs as a compose sidecar or before the MCP in
  its entrypoint. Reads `GATEWAY_INTERNAL_URL` and `GATEWAY_REGISTRATION_TOKEN`.
- `mcp-gateway bridge --name wallapop --port 8000 -- wallapop-mcp --allow-writes` — starts the
  stdio command as one long-lived child, serves it as Streamable HTTP, and registers it. This is
  today's `proxy.py`; it forwards **tools only**, as now **(call)**. Also importable
  (`mcp_gateway.bridge.build_app(command, args, env, extra_routes=…)`) so an MCP can add routes
  of its own to the same server.

HTTP is the gateway↔MCP transport because self-registration needs each MCP to be its own running
container; stdio would mean the gateway spawning children from its own image.

## Identity forwarding (later, hook reserved now)

When built: the gateway adds `X-Gateway-User-Email: <subject>` on every proxied request. v1
already stores the email on each token and already strips inbound `X-Gateway-*`, so turning it on
is one header and needs no re-sign-in. The bridge would then pass it to the stdio child per call;
how is left to that feature.

## Configuration

| Variable | Meaning |
|---|---|
| `GATEWAY_PUBLIC_URL` | `https://mcp.mergoth.synology.me` — base of every issuer and resource URL. Required unless bound to loopback. |
| `GATEWAY_ALLOWED_EMAILS` | Comma-separated Gmail addresses allowed in. |
| `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` | Google web OAuth client. |
| `GATEWAY_REGISTRATION_TOKEN` | Shared secret for the internal app. |
| `GATEWAY_STATE_DIR` | Holds `oauth-state.json` and `backends.json`. |
| `GATEWAY_HOST` / `GATEWAY_PORT` / `GATEWAY_INTERNAL_PORT` | Defaults `127.0.0.1` / `8080` / `8081`. |

Startup refuses bad config with one clear line, as `load_settings` does today.

## Deployment

- NAS project `~/mcp-gateway/` with `compose.yaml`, `.env`, `state/` (700). Container hardening
  copied from the existing compose files (read-only root, `cap_drop: ALL`, `no-new-privileges`,
  non-root uid 1026 / gid 100, tmpfs `/tmp`). Publishes only `127.0.0.1:8080:8080`.
- External Docker network `mcp-net`, created once; the gateway and every MCP compose project
  join it. MCP containers stop publishing host ports.
- The image is pure Python, so it builds on the NAS in seconds (no 15-minute rustc run).
  Deploy over `ssh -p 2300` with tar, not rsync; verify by grepping the running container for a
  new symbol, never by the script's exit code.
- Google: add `https://mcp.mergoth.synology.me/oauth/google/callback` to the web client in GCP
  project `wallapop-mcp-gw` (Console only), and add the second Gmail address as a **test user**
  while the consent screen is in Testing. Both are manual steps for Vlad.
- DSM: one new reverse-proxy entry and certificate for the hostname. Manual step for Vlad.

## Rollout

One delivery, one cutover. The code is built in slices (see the delivery plan) but nothing is
deployed until all of it is ready:

- **Gateway** at the new hostname, with no backends baked in.
- **Second brain** runs in its existing static-bearer HTTP mode on `mcp-net` with a `register`
  sidecar that passes its `MCP_AUTH_TOKEN` as `upstream_token`. Its owner-passphrase OAuth code
  (`oauth.py`, the `/login` route, `MCP_OWNER_PASSWORD`) is deleted in the same change.
- **Wallapop**: the container entrypoint becomes a small module in the Wallapop repo that calls
  `bridge.build_app(…, extra_routes=[upload])`; `upload.py` and the upload GC move there, it
  registers `public_paths: ["/upload/"]`, and `PUBLIC_BASE_URL` becomes
  `https://mcp.mergoth.synology.me/wallapop`. `gateway/` is deleted from the Wallapop repo in the
  same change.
- **Cutover**: deploy all three, connect Claude, ChatGPT and the Gemini app to `/brain/mcp` and
  `/wallapop/mcp`, then remove the two old DSM reverse-proxy entries and the old connectors.

The old containers keep running until the new connectors are confirmed, so the fallback during
cutover is simply not removing them yet.

## Testing

- Port the existing 38 gateway tests; they define the auth behaviour that must not change.
- New unit/integration tests against a fake HTTP backend: registration (auth, validation, upsert,
  persistence, delete), routing (404 / 401 with the right `resource_metadata` / 502), a full
  connect flow under `/<name>/` with and without a `resource` parameter, backend binding across
  two backends (client, code and token from one refused at the other), allowed-email list (second address accepted, third refused,
  removed address's token refused), header stripping, SSE streaming through the proxy,
  `public_paths` on and off, discovery aliases for a two-backend setup.
- Bridge: reuse `fake_upstream.py`.
- Manual acceptance at cutover with real clients — Claude, ChatGPT, Gemini app, each against both
  MCPs, one click each — recorded in the attempts log.

## Risks to check early

- **R1 — issuer with a path.** Today's issuer is a bare origin. With `<base>/<name>` as issuer,
  each client has to find the metadata at a path-bearing well-known URL. The alias list above
  covers the forms known from the spec and from attempts 1–2, but which form ChatGPT and the
  Gemini app actually request is unverified. Log every 404 under `/.well-known/` and under
  `/<name>/` during cutover; a missing alias shows up there.
- **R2 — backend Host checks.** The MCP SDK's transport security can reject unexpected `Host`
  headers. Verify second brain and the bridge accept the internal hostname the gateway sends.
- **R3 — streaming through DSM.** Already works for the current gateway; re-verify once with a
  long-running tool call through the new hostname.
- **R4 — SDK shape.** The SDK's `streamable_http_app` assumes one resource and one issuer at the
  root. The gateway needs the SDK's authorize / token / register handlers mounted once per
  backend under `/<name>/`, each with its own issuer and a provider scoped to that backend, plus
  its own bearer check for `/<name>/mcp`. Whether SDK 2.0.0's handlers can be mounted that way
  is the first thing the build must establish; if not, those three handlers are reimplemented
  on the existing provider and the 38 ported tests are the safety net.

## Left to the builder

Module layout, the HTTP client used for proxying, exact log lines, script names for NAS deploy, and the wording of user-facing pages beyond what is
quoted here.
