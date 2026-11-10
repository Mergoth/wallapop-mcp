"""Single-owner OAuth 2.1 authorization server whose owner signs in with Google.

claude.ai connectors authenticate only with OAuth, so the gateway runs its own authorization
server. The MCP SDK implements the protocol (discovery, dynamic client registration, PKCE,
code and token exchange); this module holds the owner check and token storage. The owner check
is a Google OpenID Connect sign-in accepted for exactly one verified email address.

Token storage, rotation and reuse detection follow second-brain-mcp (its ADR-0004/0005).
"""

import base64
import hashlib
import hmac
import html
import json
import logging
import os
import secrets
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.parse import urlencode, urlparse

import httpx2
from mcp.server.auth.provider import (
    AccessToken,
    AuthorizationCode,
    AuthorizationParams,
    AuthorizeError,
    OAuthAuthorizationServerProvider,
    RefreshToken,
    TokenError,
    construct_redirect_uri,
)
from mcp.shared.auth import OAuthClientInformationFull, OAuthToken
from starlette.requests import Request
from starlette.responses import HTMLResponse, RedirectResponse, Response

from wallapop_gateway.config import ConfigError
from wallapop_gateway.metadata import SCOPE

log = logging.getLogger(__name__)

LOGIN_PATH = "/login"
CALLBACK_PATH = "/oauth/google/callback"
STATIC_CLIENT_ID = "static-bearer"
OWNER = "owner"

GOOGLE_AUTH_URL = "https://accounts.google.com/o/oauth2/v2/auth"
GOOGLE_TOKEN_URL = "https://oauth2.googleapis.com/token"
GOOGLE_ISSUERS = ("https://accounts.google.com", "accounts.google.com")

ACCESS_TTL = 3600
REFRESH_TTL = 90 * 86400
CODE_TTL = 300
LOGIN_TTL = 1800  # 30 min — ChatGPT may delay showing the sign-in link to the user
MAX_CLIENTS = 50
MAX_PENDING = 100
TXN_COOKIE = "wallapop_login"

_SECURITY_HEADERS = {
    "Cache-Control": "no-store",
    "Content-Security-Policy": (
        "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; "
        "form-action 'none'"
    ),
    "Referrer-Policy": "no-referrer",
    "X-Frame-Options": "DENY",
}


def _digest(token: str) -> str:
    """Tokens are stored only as SHA-256 digests; the state file never holds a usable token."""
    return hashlib.sha256(token.encode()).hexdigest()


def _normalize(url: str) -> str:
    return url.rstrip("/")


def _s256(verifier: str) -> str:
    return base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip("=")


def _jwt_claims(token: str) -> dict[str, Any]:
    parts = token.split(".")
    if len(parts) != 3:
        raise ValueError("id_token is not a JWT")
    payload = parts[1] + "=" * (-len(parts[1]) % 4)
    claims = json.loads(base64.urlsafe_b64decode(payload))
    if not isinstance(claims, dict):
        raise TypeError("id_token payload is not an object")
    return claims


@dataclass
class _PendingLogin:
    client_id: str
    params: AuthorizationParams
    expires_at: float
    google_verifier: str | None = None
    nonce: str | None = None


class GoogleOwnerOAuthProvider(
    OAuthAuthorizationServerProvider[AuthorizationCode, RefreshToken, AccessToken]
):
    """Authorization server with exactly one resource owner, authenticated by Google."""

    def __init__(
        self,
        *,
        google_client_id: str,
        google_client_secret: str,
        owner_email: str,
        static_token: str | None,
        resource_url: str,
        issuer_url: str,
        state_dir: Path,
        http_client: httpx2.AsyncClient | None = None,
    ) -> None:
        if not state_dir.is_dir():
            raise ConfigError(f"MCP_STATE_DIR={str(state_dir)!r} is not a directory")
        self._google_id = google_client_id
        self._google_secret = google_client_secret
        self._owner_email = owner_email.strip().lower()
        self._static = static_token.encode() if static_token else None
        self._resource = _normalize(resource_url)
        self._issuer = _normalize(issuer_url)
        self._secure_cookie = self._issuer.startswith("https://")
        self._state_path = state_dir / "oauth-state.json"
        self._http = http_client or httpx2.AsyncClient(timeout=15)

        # Short-lived, in memory only: a restart just means signing in again.
        self._pending: dict[str, _PendingLogin] = {}
        self._codes: dict[str, AuthorizationCode] = {}
        # Recently completed txns, so a reloaded or resubmitted sign-in page says "done", not "expired".
        self._completed_logins: dict[str, dict] = {}

        # Durable: registered clients and token digests.
        self._clients: dict[str, OAuthClientInformationFull] = {}
        self._access: dict[str, dict[str, Any]] = {}
        self._refresh: dict[str, dict[str, Any]] = {}
        # Spent refresh digests, kept until they would have expired so a replay is recognised
        # as reuse rather than as an unknown token.
        self._consumed: dict[str, dict[str, Any]] = {}
        self._load()

    @property
    def callback_url(self) -> str:
        return f"{self._issuer}{CALLBACK_PATH}"

    # ---- persistence -------------------------------------------------------------------

    def _load(self) -> None:
        if not self._state_path.exists():
            return
        try:
            state = json.loads(self._state_path.read_text())
            self._clients = {
                cid: OAuthClientInformationFull.model_validate(info)
                for cid, info in state.get("clients", {}).items()
            }
        except (OSError, ValueError) as exc:
            raise ConfigError(f"cannot read OAuth state {self._state_path}: {exc}") from exc
        self._access = state.get("access", {})
        self._refresh = state.get("refresh", {})
        self._consumed = state.get("consumed", {})

    def _save(self) -> None:
        now = time.time()
        self._access = {d: r for d, r in self._access.items() if r["expires_at"] > now}
        self._refresh = {d: r for d, r in self._refresh.items() if r["expires_at"] > now}
        self._consumed = {d: r for d, r in self._consumed.items() if r["expires_at"] > now}
        state = {
            "clients": {
                cid: info.model_dump(mode="json", exclude_none=True)
                for cid, info in self._clients.items()
            },
            "access": self._access,
            "refresh": self._refresh,
            "consumed": self._consumed,
        }
        fd, tmp = tempfile.mkstemp(dir=self._state_path.parent, prefix=".oauth-state-")
        try:
            with os.fdopen(fd, "w") as fh:
                json.dump(state, fh)
            os.replace(tmp, self._state_path)
        except BaseException:
            Path(tmp).unlink(missing_ok=True)
            raise

    # ---- client registration -----------------------------------------------------------

    async def get_client(self, client_id: str) -> OAuthClientInformationFull | None:
        return self._clients.get(client_id)

    async def register_client(self, client_info: OAuthClientInformationFull) -> None:
        # Registration is open by design (that is how Claude registers itself), so cap it.
        # A client holding a live refresh token is never evicted.
        assert client_info.client_id is not None
        self._clients[client_info.client_id] = client_info
        if len(self._clients) > MAX_CLIENTS:
            in_use = {r["client_id"] for r in self._refresh.values()}
            evictable = sorted(
                (c for c in self._clients.values() if c.client_id not in in_use),
                key=lambda c: c.client_id_issued_at or 0,
            )
            for client in evictable[: len(self._clients) - MAX_CLIENTS]:
                del self._clients[client.client_id]  # type: ignore[arg-type]
        self._save()

    # ---- authorization: consent page, then Google ---------------------------------------

    async def authorize(
        self, client: OAuthClientInformationFull, params: AuthorizationParams
    ) -> str:
        if params.resource is not None and _normalize(params.resource) != self._resource:
            raise AuthorizeError("invalid_target", f"unknown resource {params.resource!r}")
        now = time.time()
        self._pending = {t: p for t, p in self._pending.items() if p.expires_at > now}
        while len(self._pending) >= MAX_PENDING:
            self._pending.pop(next(iter(self._pending)))
        txn = secrets.token_urlsafe(32)
        assert client.client_id is not None
        self._pending[txn] = _PendingLogin(client.client_id, params, now + LOGIN_TTL)
        return f"{self._issuer}{LOGIN_PATH}?txn={txn}"

    def _live_pending(self, txn: object) -> _PendingLogin | None:
        pending = self._pending.get(txn) if isinstance(txn, str) else None
        if pending is None or pending.expires_at < time.time():
            return None
        return pending

    async def login(self, request: Request) -> Response:
        """GET shows the consent page; POST (the owner's click) hands off to Google."""
        if request.method == "POST":
            form = await request.form()
            txn = form.get("txn")
        else:
            form = None
            txn = request.query_params.get("txn")

        pending = self._live_pending(txn)
        if pending is None:
            # Check if this txn was recently completed — ChatGPT re-POSTs to verify after callback.
            now2 = time.time()
            self._completed_logins = {t: e for t, e in self._completed_logins.items() if e["expires_at"] > now2}
            if isinstance(txn, str) and txn in self._completed_logins:
                log.info("login %s for completed txn — 200", request.method)
                return _page("Sign-in complete. Return to your AI assistant.")
            if request.method == "POST" and form is not None:
                log.warning(
                    "POST /login with no valid txn — form keys: %s; body preview: %.200s",
                    list(form.keys()),
                    str(dict(form))[:200],
                )
            return _page("This sign-in link has expired. Start the connection again from your AI assistant.", status=400)
        client = self._clients.get(pending.client_id)
        if client is None:
            return _page("Unknown client. Start the connection again from your AI assistant.", status=400)
        assert isinstance(txn, str)

        if form is None and request.query_params.get("go") != "1":
            response = _consent_page(txn, client, pending.params)
            # The Google session is ambient, so the click must come from this page: a
            # cross-site POST would not carry a SameSite=Strict cookie.
            response.set_cookie(
                TXN_COOKIE,
                txn,
                max_age=LOGIN_TTL,
                path=LOGIN_PATH,
                secure=self._secure_cookie,
                httponly=True,
                samesite="strict",
            )
            return response

        cookie = request.cookies.get(TXN_COOKIE, "")
        if not hmac.compare_digest(cookie.encode(), txn.encode()):
            return _page("Please open the sign-in link from your AI assistant and click Continue.", status=403)

        pending.google_verifier = secrets.token_urlsafe(48)
        pending.nonce = secrets.token_urlsafe(24)
        query = urlencode(
            {
                "client_id": self._google_id,
                "redirect_uri": self.callback_url,
                "response_type": "code",
                "scope": "openid email",
                "state": txn,
                "nonce": pending.nonce,
                "code_challenge": _s256(pending.google_verifier),
                "code_challenge_method": "S256",
                "login_hint": self._owner_email,
            }
        )
        return RedirectResponse(
            f"{GOOGLE_AUTH_URL}?{query}", status_code=302, headers={"Cache-Control": "no-store"}
        )

    async def google_callback(self, request: Request) -> Response:
        txn = request.query_params.get("state")
        pending = self._live_pending(txn)
        if pending is None or pending.google_verifier is None:
            return _page("This sign-in link has expired. Start the connection again from your AI assistant.", status=400)
        assert isinstance(txn, str)
        # Single use whatever the outcome: a failed sign-in starts again from Claude.
        del self._pending[txn]

        if request.query_params.get("error"):
            return _page("Google sign-in was cancelled.", status=400)
        code = request.query_params.get("code")
        if not code:
            return _page("Google did not return a sign-in code.", status=400)

        try:
            email = await self._verified_email(code, pending)
        except (httpx2.HTTPError, ValueError, TypeError, KeyError) as exc:
            log.warning("google sign-in failed: %s", exc)
            return _page("Google sign-in failed. Please try connecting again from your AI assistant.", status=502)
        if email is None or not hmac.compare_digest(email.encode(), self._owner_email.encode()):
            log.warning("google sign-in refused for %s", email)
            return _page("This Google account is not allowed to use this server.", status=403)

        client = self._clients.get(pending.client_id)
        if client is None:
            return _page("Unknown client. Start the connection again from your AI assistant.", status=400)
        now = time.time()
        auth_code = secrets.token_urlsafe(32)
        self._codes = {c: a for c, a in self._codes.items() if a.expires_at > now}
        self._codes[auth_code] = AuthorizationCode(
            code=auth_code,
            scopes=pending.params.scopes or [SCOPE],
            expires_at=now + CODE_TTL,
            client_id=pending.client_id,
            code_challenge=pending.params.code_challenge,
            redirect_uri=pending.params.redirect_uri,
            redirect_uri_provided_explicitly=pending.params.redirect_uri_provided_explicitly,
            resource=pending.params.resource,
            subject=OWNER,
        )
        log.warning("owner signed in; client=%s", client.client_name or client.client_id)
        redirect_url = construct_redirect_uri(
            str(pending.params.redirect_uri), code=auth_code, state=pending.params.state
        )
        log.warning("callback redirect to %s", str(pending.params.redirect_uri))
        self._completed_logins[txn] = {"expires_at": now + 120}
        response = RedirectResponse(redirect_url, status_code=302, headers={"Cache-Control": "no-store"})
        response.delete_cookie(TXN_COOKIE, path=LOGIN_PATH)
        return response

    async def _verified_email(self, code: str, pending: _PendingLogin) -> str | None:
        """Redeem Google's code and return the email from its ID token, or None if unverified.

        The ID token comes straight from Google's token endpoint over TLS, which OpenID Connect
        Core 3.1.3.7 accepts in place of checking its signature. The claims are still checked.
        """
        r = await self._http.post(
            GOOGLE_TOKEN_URL,
            data={
                "code": code,
                "client_id": self._google_id,
                "client_secret": self._google_secret,
                "redirect_uri": self.callback_url,
                "grant_type": "authorization_code",
                "code_verifier": pending.google_verifier,
            },
        )
        if r.status_code != 200:
            raise ValueError(f"token endpoint returned {r.status_code}")
        claims = _jwt_claims(r.json()["id_token"])
        audience = claims.get("aud")
        audiences = audience if isinstance(audience, list) else [audience]
        if claims.get("iss") not in GOOGLE_ISSUERS:
            raise ValueError("id_token issuer is not Google")
        if self._google_id not in audiences:
            raise ValueError("id_token was issued to another client")
        if not isinstance(claims.get("exp"), (int, float)) or claims["exp"] < time.time():
            raise ValueError("id_token has expired")
        if not hmac.compare_digest(str(claims.get("nonce", "")).encode(), pending.nonce.encode()):
            raise ValueError("id_token nonce does not match")
        if claims.get("email_verified") is not True or not isinstance(claims.get("email"), str):
            return None
        return claims["email"].strip().lower()

    # ---- codes and tokens --------------------------------------------------------------

    async def load_authorization_code(
        self, client: OAuthClientInformationFull, authorization_code: str
    ) -> AuthorizationCode | None:
        code = self._codes.get(authorization_code)
        if code is None:
            log.warning("load_authorization_code: code not found (pool size=%d)", len(self._codes))
        elif code.client_id != client.client_id:
            log.warning(
                "load_authorization_code: client_id mismatch — code has %s, request has %s",
                code.client_id,
                client.client_id,
            )
        return code if code is not None and code.client_id == client.client_id else None

    async def exchange_authorization_code(
        self, client: OAuthClientInformationFull, authorization_code: AuthorizationCode
    ) -> OAuthToken:
        if self._codes.pop(authorization_code.code, None) is None:
            raise TokenError("invalid_grant", "authorization code already used")
        assert client.client_id is not None
        return self._issue(client.client_id, authorization_code.scopes, authorization_code.resource)

    async def load_refresh_token(
        self, client: OAuthClientInformationFull, refresh_token: str
    ) -> RefreshToken | None:
        digest = _digest(refresh_token)
        record = self._refresh.get(digest)
        if record is None:
            # The SDK rejects the grant as soon as this returns None, so reuse detection has
            # to happen here rather than in exchange_refresh_token.
            self._detect_reuse(digest)
            return None
        if record["client_id"] != client.client_id:
            return None
        return RefreshToken(
            token=refresh_token,
            client_id=record["client_id"],
            scopes=record["scopes"],
            expires_at=record["expires_at"],
            subject=OWNER,
        )

    async def exchange_refresh_token(
        self, client: OAuthClientInformationFull, refresh_token: RefreshToken, scopes: list[str]
    ) -> OAuthToken:
        digest = _digest(refresh_token.token)
        record = self._refresh.pop(digest, None)
        if record is None:
            self._detect_reuse(digest)
            raise TokenError("invalid_grant", "refresh token already used")
        family = record.get("family")
        self._consumed[digest] = {"family": family, "expires_at": record["expires_at"]}
        self._drop_access_minted_by(digest)
        assert client.client_id is not None
        return self._issue(client.client_id, scopes, record.get("resource"), family=family)

    def _detect_reuse(self, digest: str) -> None:
        """A spent refresh token presented again means the chain is compromised (RFC 9700)."""
        spent = self._consumed.pop(digest, None)
        if spent is None:
            return
        family = spent.get("family")
        log.warning("refresh token reuse detected; revoking token family")
        if family:
            self._refresh = {d: r for d, r in self._refresh.items() if r.get("family") != family}
            self._access = {d: r for d, r in self._access.items() if r.get("family") != family}
            self._consumed = {d: r for d, r in self._consumed.items() if r.get("family") != family}
        self._save()

    def _issue(
        self, client_id: str, scopes: list[str], resource: str | None, family: str | None = None
    ) -> OAuthToken:
        access, refresh = secrets.token_urlsafe(32), secrets.token_urlsafe(32)
        now = int(time.time())
        family = family or secrets.token_urlsafe(16)
        refresh_digest = _digest(refresh)
        self._refresh[refresh_digest] = {
            "client_id": client_id,
            "scopes": scopes,
            "expires_at": now + REFRESH_TTL,
            "resource": resource,
            "family": family,
        }
        self._access[_digest(access)] = {
            "client_id": client_id,
            "scopes": scopes,
            "expires_at": now + ACCESS_TTL,
            "resource": resource,
            "refresh": refresh_digest,
            "family": family,
        }
        self._save()
        return OAuthToken(
            access_token=access,
            token_type="Bearer",
            expires_in=ACCESS_TTL,
            scope=" ".join(scopes),
            refresh_token=refresh,
        )

    def _drop_access_minted_by(self, refresh_digest: str) -> None:
        self._access = {d: r for d, r in self._access.items() if r.get("refresh") != refresh_digest}

    async def load_access_token(self, token: str) -> AccessToken | None:
        if self._static is not None and hmac.compare_digest(token.encode(), self._static):
            # Audience-bound like every other token, so it is not exempt from the check.
            return AccessToken(
                token=token, client_id=STATIC_CLIENT_ID, scopes=[SCOPE], resource=self._resource
            )
        record = self._access.get(_digest(token))
        if record is None or record["expires_at"] < time.time():
            return None
        if record.get("resource") and _normalize(record["resource"]) != self._resource:
            return None
        return AccessToken(
            token=token,
            client_id=record["client_id"],
            scopes=record["scopes"],
            expires_at=record["expires_at"],
            resource=record.get("resource"),
            subject=OWNER,
        )

    async def revoke_token(self, token: AccessToken | RefreshToken) -> None:
        digest = _digest(token.token)
        if isinstance(token, RefreshToken):
            refresh_digest: str | None = digest
        else:
            record = self._access.pop(digest, None)
            refresh_digest = record.get("refresh") if record else None
        if refresh_digest:
            self._refresh.pop(refresh_digest, None)
            self._drop_access_minted_by(refresh_digest)
        self._save()


# ---- HTML ----------------------------------------------------------------------------------

_STYLE = """
body{font:16px/1.5 system-ui,-apple-system,sans-serif;max-width:26rem;margin:10vh auto;padding:0 1.25rem;color:#1f2328;background:#f6f8fa}
main{background:#fff;border:1px solid #d0d7de;border-radius:12px;padding:1.5rem}
h1{font-size:1.25rem;margin:0 0 .75rem}p{margin:.5rem 0}
.meta{color:#57606a;font-size:.9rem}.warn{color:#9a6700}
.button{display:block;box-sizing:border-box;text-align:center;text-decoration:none;width:100%;font:inherit;font-weight:600;padding:.6rem;margin-top:.75rem;border:0;border-radius:8px;background:#1f883d;color:#fff}
"""

_TRUSTED_REDIRECT_HOSTS = (
    "claude.ai",
    "claude.com",
    "chatgpt.com",
    "chat.openai.com",
    "openai.com",
    "oauth-redirect.googleusercontent.com",  # Gemini app custom connectors
    "vertexaisearch.cloud.google.com",  # Gemini Enterprise
    "localhost",  # Gemini CLI, Claude Code
    "127.0.0.1",
)


def _page(body: str, *, status: int = 200) -> HTMLResponse:
    return HTMLResponse(
        f"<!doctype html><meta charset=utf-8>"
        f'<meta name=viewport content="width=device-width,initial-scale=1">'
        f"<title>Wallapop MCP</title><style>{_STYLE}</style><main>{body}</main>",
        status_code=status,
        headers=_SECURITY_HEADERS,
    )


def _consent_page(
    txn: str, client: OAuthClientInformationFull, params: AuthorizationParams
) -> HTMLResponse:
    # Client name and redirect URI come from open registration: escape both, and show the
    # redirect host so a sign-in that would not return to Claude is visible.
    name = html.escape(client.client_name or "An unnamed client")
    host = urlparse(str(params.redirect_uri)).hostname or "?"
    warning = (
        ""
        if host in _TRUSTED_REDIRECT_HOSTS
        else f"<p class=warn>This will return to <b>{html.escape(host)}</b>, not a known AI assistant. "
        f"Continue only if you expect that.</p>"
    )
    return _page(
        f"<h1>Allow access to your Wallapop account?</h1>"
        f"<p><b>{name}</b> is asking to use your Wallapop account.</p>"
        f"<p class=meta>Returns to <b>{html.escape(host)}</b></p>{warning}"
        # A link, not a form: browsers check CSP form-action against every redirect after a
        # form submit, and the chain ends on the client's own redirectors (e.g. Gemini's
        # googleusercontent page 302s onward), which no allowlist here can name.
        f'<a class=button href="{LOGIN_PATH}?{html.escape(urlencode({"txn": txn, "go": "1"}))}">'
        f"Continue with Google</a>"
    )
