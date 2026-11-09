"""End-to-end tests: drive the real app the way claude.ai does, with Google's token endpoint faked."""

from pathlib import Path
from urllib.parse import parse_qs, urlparse

import pytest
from starlette.testclient import TestClient

from conftest import (
    BASE,
    CALLBACK,
    GOOGLE_ID,
    MCP_HEADERS,
    OWNER,
    RESOURCE,
    STATIC,
    FakeGoogle,
    authorize,
    env,
    login_txn,
    mcp_session,
    pkce,
    register,
    rpc_result,
    sign_in,
    to_google,
)
from wallapop_gateway.config import ConfigError, load_settings

# ---- discovery -----------------------------------------------------------------------------


def test_unauthenticated_mcp_is_401_with_resource_metadata(client: TestClient) -> None:
    r = client.post("/mcp", json={}, headers=MCP_HEADERS)
    assert r.status_code == 401
    assert f"{BASE}/.well-known/oauth-protected-resource/mcp" in r.headers["www-authenticate"]


def test_protected_resource_metadata_matches_the_entered_url(client: TestClient) -> None:
    meta = client.get("/.well-known/oauth-protected-resource/mcp").json()
    assert meta["resource"] == RESOURCE
    assert meta["authorization_servers"] == [BASE]


def test_metadata_advertises_public_clients(client: TestClient) -> None:
    meta = client.get("/.well-known/oauth-authorization-server").json()
    assert meta["issuer"] == BASE
    assert meta["token_endpoint_auth_methods_supported"][0] == "none"
    assert "S256" in meta["code_challenge_methods_supported"]


# ---- sign-in -------------------------------------------------------------------------------


def test_full_flow_grants_a_working_token(client: TestClient, google: FakeGoogle) -> None:
    _, tokens = sign_in(client, google)
    headers = mcp_session(client, tokens["access_token"])
    r = client.post(
        "/mcp", json={"jsonrpc": "2.0", "id": 2, "method": "tools/list"}, headers=headers
    )
    assert [t["name"] for t in rpc_result(r)["result"]["tools"]] == ["echo"]


def test_google_is_asked_with_pkce_nonce_and_the_owner_hint(
    client: TestClient, google: FakeGoogle
) -> None:
    _, challenge = pkce()
    txn = login_txn(client, register(client), challenge)
    query = to_google(client, google, txn)
    assert query["client_id"] == GOOGLE_ID
    assert query["redirect_uri"] == f"{BASE}/oauth/google/callback"
    assert query["scope"] == "openid email"
    assert query["code_challenge_method"] == "S256"
    assert query["login_hint"] == OWNER
    assert query["state"] == txn
    client.get("/oauth/google/callback", params={"state": txn, "code": "google-code"})
    sent = google.requests[0]
    assert sent["code"] == ["google-code"]
    assert sent["client_secret"] == ["google-secret"]
    assert sent["code_verifier"][0]


def test_consent_page_escapes_the_client_name_and_shows_the_redirect_host(
    client: TestClient,
) -> None:
    _, challenge = pkce()
    txn = login_txn(client, register(client, name="<script>x</script>"), challenge)
    page = client.get("/login", params={"txn": txn}).text
    assert "<script>x</script>" not in page
    assert "&lt;script&gt;" in page
    assert "claude.ai" in page


def test_consent_continues_with_a_link_not_a_form(client: TestClient) -> None:
    """CSP form-action would govern every redirect after a form submit, out to the client's
    own redirectors; a link navigation is not subject to it."""
    _, challenge = pkce()
    txn = login_txn(client, register(client), challenge)
    page = client.get("/login", params={"txn": txn}).text
    assert "<form" not in page
    assert f'href="/login?txn={txn}&amp;go=1"' in page


def test_consent_page_trusts_gemini_redirects(client: TestClient) -> None:
    gemini = "https://oauth-redirect.googleusercontent.com/r/user_bound_custom-mcp-1-example"
    client_id = register(client, name="Gemini", redirect=gemini)
    _, challenge = pkce()
    r = client.get(
        "/authorize",
        params={
            "response_type": "code",
            "client_id": client_id,
            "redirect_uri": gemini,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
            "state": "s",
        },
    )
    assert r.status_code == 302, r.text
    page = client.get(r.headers["location"].removeprefix(BASE))
    assert "not a known AI assistant" not in page.text


def _confidential(client: TestClient, method: str) -> tuple[str, str]:
    r = client.post(
        "/register",
        json={
            "client_name": "Gemini",
            "redirect_uris": [CALLBACK],
            "token_endpoint_auth_method": method,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        },
    )
    assert r.status_code == 201, r.text
    return r.json()["client_id"], r.json()["client_secret"]


def _bad_code_exchange(client: TestClient, form: dict, auth: tuple[str, str] | None = None):
    data = {"grant_type": "authorization_code", "code": "nope", "redirect_uri": CALLBACK,
            "code_verifier": "v" * 43, **form}
    return client.post("/token", data=data, auth=auth)


@pytest.mark.parametrize("registered", ["client_secret_post", "client_secret_basic"])
@pytest.mark.parametrize("sent", ["post", "basic"])
def test_token_accepts_the_secret_in_form_or_basic(
    client: TestClient, registered: str, sent: str
) -> None:
    """A bad code past a passing client check is invalid_grant (400), not invalid_client (401)."""
    client_id, secret = _confidential(client, registered)
    if sent == "post":
        r = _bad_code_exchange(client, {"client_id": client_id, "client_secret": secret})
    else:
        r = _bad_code_exchange(client, {}, auth=(client_id, secret))
    assert r.status_code == 400, r.text
    assert r.json()["error"] == "invalid_grant"


def test_token_still_rejects_a_wrong_secret(client: TestClient) -> None:
    client_id, _ = _confidential(client, "client_secret_post")
    r = _bad_code_exchange(client, {}, auth=(client_id, "wrong"))
    assert r.status_code == 401


def test_consent_click_without_the_page_cookie_is_refused(client: TestClient) -> None:
    """A cross-site link carries no SameSite=Strict cookie, so it cannot skip consent."""
    _, challenge = pkce()
    txn = login_txn(client, register(client), challenge)
    go = {"txn": txn, "go": "1"}
    assert client.get("/login", params=go).status_code == 403
    assert client.post("/login", data={"txn": txn}).status_code == 403
    client.get("/login", params={"txn": txn})
    client.cookies.clear()
    client.cookies.set("wallapop_login", "another-txn")
    assert client.get("/login", params=go).status_code == 403


@pytest.mark.parametrize(
    "claims",
    [
        {"email": "intruder@example.com"},
        {"email_verified": False},
        {"email_verified": None},
    ],
)
def test_only_the_verified_owner_gets_a_code(
    client: TestClient, google: FakeGoogle, claims: dict
) -> None:
    google.claims = claims
    _, challenge = pkce()
    txn = login_txn(client, register(client), challenge)
    to_google(client, google, txn)
    r = client.get("/oauth/google/callback", params={"state": txn, "code": "google-code"})
    assert r.status_code == 403
    assert "location" not in r.headers


def test_owner_email_matches_case_insensitively(client: TestClient, google: FakeGoogle) -> None:
    google.claims = {"email": OWNER.upper()}
    sign_in(client, google)


@pytest.mark.parametrize(
    "claims",
    [
        {"aud": "someone-elses-client"},
        {"iss": "https://evil.example.com"},
        {"exp": 1},
        {"nonce": "replayed"},
    ],
)
def test_an_invalid_id_token_is_refused(
    client: TestClient, google: FakeGoogle, claims: dict
) -> None:
    google.claims = claims
    _, challenge = pkce()
    txn = login_txn(client, register(client), challenge)
    to_google(client, google, txn)
    r = client.get("/oauth/google/callback", params={"state": txn, "code": "google-code"})
    assert r.status_code == 502
    assert "location" not in r.headers


def test_google_token_error_is_refused(client: TestClient, google: FakeGoogle) -> None:
    google.status = 400
    _, challenge = pkce()
    txn = login_txn(client, register(client), challenge)
    to_google(client, google, txn)
    r = client.get("/oauth/google/callback", params={"state": txn, "code": "google-code"})
    assert r.status_code == 502


def test_callback_state_is_single_use(client: TestClient, google: FakeGoogle) -> None:
    _, challenge = pkce()
    txn = login_txn(client, register(client), challenge)
    to_google(client, google, txn)
    assert (
        client.get("/oauth/google/callback", params={"state": txn, "code": "c"}).status_code == 302
    )
    assert (
        client.get("/oauth/google/callback", params={"state": txn, "code": "c"}).status_code == 400
    )


def test_callback_before_consent_is_refused(client: TestClient) -> None:
    _, challenge = pkce()
    txn = login_txn(client, register(client), challenge)
    r = client.get("/oauth/google/callback", params={"state": txn, "code": "google-code"})
    assert r.status_code == 400


def test_cancelled_google_sign_in_is_refused(client: TestClient, google: FakeGoogle) -> None:
    _, challenge = pkce()
    txn = login_txn(client, register(client), challenge)
    to_google(client, google, txn)
    r = client.get("/oauth/google/callback", params={"state": txn, "error": "access_denied"})
    assert r.status_code == 400
    assert google.requests == []


def test_authorize_rejects_a_foreign_resource(client: TestClient) -> None:
    _, challenge = pkce()
    r = authorize(client, register(client), challenge, resource="https://other.example.com/mcp")
    assert r.status_code == 302
    assert parse_qs(urlparse(r.headers["location"]).query)["error"] == ["invalid_target"]


# ---- tokens --------------------------------------------------------------------------------


def test_refresh_rotates_and_reuse_revokes_the_family(
    client: TestClient, google: FakeGoogle
) -> None:
    client_id, first = sign_in(client, google)

    def refresh(token: str):
        return client.post(
            "/token",
            data={"grant_type": "refresh_token", "refresh_token": token, "client_id": client_id},
        )

    r = refresh(first["refresh_token"])
    assert r.status_code == 200, r.text
    second = r.json()
    assert (
        client.post(
            "/mcp",
            json={},
            headers={**MCP_HEADERS, "Authorization": f"Bearer {first['access_token']}"},
        ).status_code
        == 401
    )

    assert refresh(first["refresh_token"]).status_code == 400
    assert refresh(second["refresh_token"]).status_code == 400


def test_tokens_survive_a_restart_as_digests_only(
    client: TestClient, google: FakeGoogle, state_dir: Path
) -> None:
    _, tokens = sign_in(client, google)
    stored = (state_dir / "oauth-state.json").read_text()
    assert tokens["access_token"] not in stored
    assert tokens["refresh_token"] not in stored

    from wallapop_gateway.app import build_app

    with TestClient(build_app(load_settings(env(state_dir))), base_url=BASE) as again:
        mcp_session(again, tokens["access_token"])


def test_static_bearer_calls_tools_through_the_proxy(client: TestClient) -> None:
    headers = mcp_session(client, STATIC)
    r = client.post(
        "/mcp",
        json={
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "echo", "arguments": {"text": "hola"}},
        },
        headers=headers,
    )
    content = rpc_result(r)["result"]["content"]
    assert content[0]["text"] == "echo:hola:/data/session.json"


def test_garbage_bearer_is_rejected(client: TestClient) -> None:
    r = client.post("/mcp", json={}, headers={**MCP_HEADERS, "Authorization": "Bearer nope"})
    assert r.status_code == 401


# ---- configuration -------------------------------------------------------------------------


@pytest.mark.parametrize(
    "missing", ["GOOGLE_CLIENT_ID", "GOOGLE_CLIENT_SECRET", "MCP_OWNER_EMAIL", "MCP_STATE_DIR"]
)
def test_missing_setting_refuses_to_start(state_dir: Path, missing: str) -> None:
    with pytest.raises(ConfigError, match=missing):
        load_settings(env(state_dir, **{missing: ""}))


def test_resource_url_is_required_off_loopback(state_dir: Path) -> None:
    with pytest.raises(ConfigError, match="MCP_RESOURCE_URL"):
        load_settings(env(state_dir, MCP_RESOURCE_URL="", MCP_HOST="0.0.0.0"))


def test_static_token_is_optional(state_dir: Path) -> None:
    assert load_settings(env(state_dir, MCP_AUTH_TOKEN="")).static_token is None


def test_only_wallapop_variables_reach_the_child(state_dir: Path) -> None:
    settings = load_settings(env(state_dir) | {"PATH": "/bin"})
    assert set(settings.upstream_env) == {
        "WALLAPOP_MCP_COMMAND",
        "WALLAPOP_MCP_ARGS",
        "WALLAPOP_SESSION_PATH",
    }
