"""RFC 9728 protected-resource and RFC 8414 authorization-server configuration."""

import functools

from mcp.server.auth import routes as _auth_routes
from mcp.server.auth.settings import AuthSettings, ClientRegistrationOptions

# The only scope. Advertised and granted by default so a client that requests what the
# metadata lists is never refused for a scope it did not register.
SCOPE = "wallapop"


def build_auth_settings(*, resource_url: str, issuer_url: str) -> AuthSettings:
    """AuthSettings for the owner OAuth server.

    ``issuer_url`` stays a string so AuthSettings keeps it verbatim: RFC 8414 issuer comparison
    is exact. Revocation is not advertised because SDK 2.0.0 requires ``client_secret`` on it
    even for public clients, so Claude would get a 400.
    """
    return AuthSettings(
        issuer_url=issuer_url,
        resource_server_url=resource_url,
        client_registration_options=ClientRegistrationOptions(
            enabled=True, valid_scopes=[SCOPE], default_scopes=[SCOPE]
        ),
        required_scopes=[SCOPE],
    )


PUBLIC_CLIENT_AUTH = "none"
_patched = False


def enable_public_client_metadata() -> None:
    """Advertise ``token_endpoint_auth_method=none`` in the RFC 8414 metadata.

    SDK 2.0.0 hardcodes only the two client_secret methods. Claude is a public client (PKCE,
    no secret) and declines to register with a server that does not list ``none``, although
    the SDK accepts such clients at /register and /token. Idempotent.
    """
    global _patched
    if _patched:
        return
    original = _auth_routes.build_metadata

    @functools.wraps(original)
    def build_metadata(*args, **kwargs):
        metadata = original(*args, **kwargs)
        methods = list(metadata.token_endpoint_auth_methods_supported or [])
        if PUBLIC_CLIENT_AUTH not in methods:
            metadata.token_endpoint_auth_methods_supported = [PUBLIC_CLIENT_AUTH, *methods]
        return metadata

    _auth_routes.build_metadata = build_metadata
    _patched = True
