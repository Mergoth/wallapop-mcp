# MCP SSO gateway — delivery plan (AI Factory)

Date: 2026-11-10. Design: `docs/superpowers/specs/2026-11-10-mcp-sso-gateway-design.md`.

Built with AI Factory (`~/work/ai-factory`, v1.1.0): Claude Code writes the brief, specs,
handoffs and verdicts; `agy` writes all code. This plan fixes the repos, the order of factory
runs and what each must prove. The per-slice `specs/<slug>.md` and `handoffs/…` files are written
by `/factory-spec` inside each repo, against the code as it stands then — not here.

One delivery: every slice is built and checked first, then everything is deployed in one cutover.

## Repos

| Repo | Factory | Work |
|---|---|---|
| `~/work/mcp-gateway` (new) | install | slices G1–G3 |
| `~/work/second-brain-mcp` | already installed | slice B1 |
| `~/work/wallapop-mcp`, branch `pr-6-rust` | install (or ad-hoc mode, files untracked) | slice W1 |

Each factory run happens in a Claude Code session opened in that repo, because the `/factory-*`
skills are linked per repo.

## Step 0 — seed the gateway repo (Claude, no `agy`)

1. Create `~/work/mcp-gateway` from the existing code with history:
   `git subtree split --prefix=gateway pr-6-rust` in `wallapop-mcp`, pulled into a fresh repo.
   The result has `src/wallapop_gateway`, `tests/`, `pyproject.toml`, `uv.lock` at its root and
   the 38 tests passing, so the factory does not start from a greenfield repo.
2. Copy in `docs/chatgpt-oauth-attempts.md` (the attempts log) and the design doc.
3. Install the factory: `git submodule add https://github.com/Mergoth/ai-factory factory-engine`,
   `bash factory-engine/install.sh`, set `TEST_CMD="uv run --frozen pytest -q"` in `factory.env`.
4. Fill `factory/context.md` (stack, layout, NAS facts, deploy gotchas) and write the ADRs below.
   ADRs are binding on every later round: `/factory-check` fails a slice that breaks one even
   with green tests.
5. Write `factory/briefs/mcp-sso-gateway.md` from the design doc (problem, done, rejected
   alternatives, risks, assumptions). `/factory-think` is skipped: its job — settling the why
   and the alternatives — was done in the design conversation.
6. Commit. Confirm `TEST_CMD` is green and `agy models` answers.

ADRs to write in step 0:

- **0001** The backend is identified by the URL path on every endpoint; issuer is `<base>/<name>`.
- **0002** The gateway proxies HTTP and never parses MCP.
- **0003** Registration is a push to an internal-only listener with a shared secret; the gateway
  never touches the Docker socket.
- **0004** Tokens are stored as digests, bound to one backend and to the signed-in email.
- **0005** The consent step is a link, never a form (attempts 7–9).
- **0006** Read `docs/chatgpt-oauth-attempts.md` before any auth change; add a row after each
  deploy + test.

## Slices

Each is one `/factory-spec` → `/factory-build` → `/factory-check` run, committed when the check
says Success. G1–G3 run in order in `mcp-gateway`. B1 and W1 need only G3's `register` / `bridge`
commands to exist and are independent of each other.

### G1 `per-backend-auth` — tricky

Rename the package to `mcp_gateway`. Turn the single-resource, single-owner authorization server
into the per-backend one in the design ("Authorization server", "Discovery"): endpoints under
`/<name>/`, issuer `<base>/<name>`, allowed-email list, email as token subject, backend binding
on clients, codes and tokens, generic consent page, static bearer removed, `GATEWAY_*` settings.
Backends come from an in-memory table for now. Upload route and its GC are removed from the app.

Done when: the ported tests pass under `/<name>/` paths; a full connect flow passes with and
without `resource`; a client, code or token from backend A is refused at backend B; a third
email is refused and a removed email's token stops working; every discovery alias in the design
answers for two backends and 404s for an unknown one. First task in the handoff: establish
whether the SDK handlers mount per backend (design risk R4) and report which path was taken.

### G2 `proxy-and-registration` — normal

`/<name>/mcp` becomes an HTTP reverse proxy with the bearer check; the stdio re-serving in
`proxy.py` leaves the public app. Internal app on its own port: `POST/GET/DELETE /backends`,
`/health`, shared-secret auth, validation, `backends.json` persistence. `public_paths`.

Done when: against a fake HTTP backend — registration auth/validation/upsert/persist/delete; 404,
401 (with the right `resource_metadata`) and 502; header stripping and `upstream_token`
injection; an SSE response streams through without buffering; a `public_paths` prefix passes
without a bearer and every other path under `/<name>/` does not; a backend registered at runtime
is served without restarting the app.

### G3 `backend-commands-and-deploy` — normal

`mcp-gateway register` and `mcp-gateway bridge` (CLI and `bridge.build_app(…, extra_routes=…)`),
re-registration every 5 minutes with backoff. `Dockerfile`, `compose.yaml` (hardened, loopback
publish, `mcp-net`), `.env.example`, `scripts/deploy-nas.sh` (tar over `ssh -p 2300`, verifies a
marker in the running container), README covering "add an MCP".

Done when: bridge tests pass with `fake_upstream.py`; a test starts gateway + bridge and calls a
tool end to end through `/<name>/mcp`; `register` retries until the gateway appears. Environment
limit to state in the handoff: the image build and NAS deploy cannot be verified on the Mac —
the builder declares them uncovered rather than claiming they work.

### B1 `behind-gateway` in `second-brain-mcp` — normal

Compose joins `mcp-net`, stops publishing a host port, adds the `register` sidecar passing
`MCP_AUTH_TOKEN` as `upstream_token`. Delete the owner-passphrase OAuth (`oauth.py`, `/login`,
`MCP_OWNER_PASSWORD`, related metadata patching and tests). Supersede ADR-0004/0005 there with a
new ADR: auth is the gateway's. Check design risk R2 (Host header accepted from the gateway).

Done when: tests green with the OAuth tests removed, not skipped; static-bearer HTTP mode still
starts and serves; no import of the deleted module remains.

### W1 `behind-gateway` in `wallapop-mcp` (`pr-6-rust`) — normal

New entrypoint module that builds the bridge app around the Rust binary and mounts the upload
route; `upload.py`, its GC and their tests move here. Dockerfile installs `mcp-gateway` instead of
the bundled gateway; compose joins `mcp-net`, drops the host port and the Google/owner settings,
sets `PUBLIC_BASE_URL=https://mcp.mergoth.synology.me/wallapop`, registers
`public_paths: ["/upload/"]`. Delete `gateway/` and the scripts that only served it
(`configure-nas-oauth.sh`, `register-oauth-client.sh`). The 15-minute Rust build on the NAS stays;
`deploy-nas.sh` keeps its marker check.

Done when: upload tests pass in their new home; the container entrypoint starts the bridge with
the fake upstream in tests; nothing references `wallapop_gateway`.

## Cutover (Claude drives, Vlad does the manual steps)

Manual, Vlad:
1. DSM: reverse-proxy entry `mcp.mergoth.synology.me` → `localhost:8080`, no access profile;
   certificate for the hostname.
2. Google Console, project `wallapop-mcp-gw`: add redirect URI
   `https://mcp.mergoth.synology.me/oauth/google/callback`; add the second Gmail as a test user.

Then, in order:
3. NAS: `docker network create mcp-net`; deploy the gateway; `GET /health` on both ports.
4. Deploy second brain and Wallapop; `GET /backends` on the gateway lists both.
5. Connect, one click each, and run one real tool call: Claude, ChatGPT, Gemini app × `/brain/mcp`,
   `/wallapop/mcp`. One Wallapop photo upload through `/wallapop/upload/…`. Each result goes in
   the attempts log. Watch the gateway log for 404s under `/.well-known/` and `/<name>/` (R1).
6. Sign in once with the second Gmail; confirm a third account is refused.
7. Only after 5–6 pass: stop the old containers, remove the two old DSM entries and the old
   connectors in each client.
8. `/factory-reflect` in each repo.

If step 5 fails for a client, the old containers and connectors are still live; fix through a
new factory round, log the attempt, retry.

## Not in this delivery

Forwarding the user email to MCPs, per-user access lists, any MCP beyond the two.
