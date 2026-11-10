"""Tests for handle_upload endpoint."""

import time
from pathlib import Path

import pytest
from starlette.applications import Starlette
from starlette.routing import Route
from starlette.testclient import TestClient

import wallapop_server.upload as upload_module
from wallapop_server.upload import handle_upload


@pytest.fixture
def upload_client(tmp_path: Path, upload_secret: bytes) -> tuple[TestClient, Path, bytes]:
    upload_dir = tmp_path / "uploads"
    upload_dir.mkdir()
    app = Starlette(routes=[Route("/upload/{ticket_id}", handle_upload, methods=["PUT"])])
    app.state.upload_dir = upload_dir
    app.state.upload_secret = upload_secret
    return TestClient(app), upload_dir, upload_secret


def test_valid_jpeg_upload(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "0123456789abcdef"
    exp = int(time.time()) + 300
    from conftest import VALID_JPEG, create_meta

    sig = create_meta(upload_dir, ticket_id, exp, secret)

    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig={sig}", content=VALID_JPEG)
    assert response.status_code == 201
    assert response.json() == {"ref": f"upload:{ticket_id}", "bytes": len(VALID_JPEG)}

    bin_path = upload_dir / f"{ticket_id}.bin"
    assert bin_path.exists()
    assert bin_path.read_bytes() == VALID_JPEG


def test_valid_png_upload(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "abcdef0123456789"
    exp = int(time.time()) + 300
    from conftest import VALID_PNG, create_meta

    sig = create_meta(upload_dir, ticket_id, exp, secret)

    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig={sig}", content=VALID_PNG)
    assert response.status_code == 201
    assert response.json() == {"ref": f"upload:{ticket_id}", "bytes": len(VALID_PNG)}

    bin_path = upload_dir / f"{ticket_id}.bin"
    assert bin_path.exists()
    assert bin_path.read_bytes() == VALID_PNG


def test_valid_webp_upload(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "fedcba9876543210"
    exp = int(time.time()) + 300
    from conftest import VALID_WEBP, create_meta

    sig = create_meta(upload_dir, ticket_id, exp, secret)

    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig={sig}", content=VALID_WEBP)
    assert response.status_code == 201
    assert response.json() == {"ref": f"upload:{ticket_id}", "bytes": len(VALID_WEBP)}

    bin_path = upload_dir / f"{ticket_id}.bin"
    assert bin_path.exists()
    assert bin_path.read_bytes() == VALID_WEBP


def test_error_non_hex_id(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, _, _ = upload_client
    response = client.put("/upload/not-a-hex-id?exp=123&sig=abc", content=b"data")
    assert response.status_code == 400
    assert response.json() == {"error": "invalid_ticket_id"}


def test_error_id_longer_than_64(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, _, _ = upload_client
    long_id = "a" * 65
    response = client.put(f"/upload/{long_id}?exp=123&sig=abc", content=b"data")
    assert response.status_code == 400
    assert response.json() == {"error": "invalid_ticket_id"}


def test_error_missing_exp_or_sig(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, _, _ = upload_client
    ticket_id = "0123456789abcdef"
    # Missing sig
    r1 = client.put(f"/upload/{ticket_id}?exp=123", content=b"data")
    assert r1.status_code == 400
    assert r1.json() == {"error": "missing_exp_or_sig"}
    # Missing exp
    r2 = client.put(f"/upload/{ticket_id}?sig=abc", content=b"data")
    assert r2.status_code == 400
    assert r2.json() == {"error": "missing_exp_or_sig"}


def test_error_non_integer_exp(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, _, _ = upload_client
    ticket_id = "0123456789abcdef"
    response = client.put(f"/upload/{ticket_id}?exp=notanint&sig=abc", content=b"data")
    assert response.status_code == 400
    assert response.json() == {"error": "invalid_exp"}


def test_error_wrong_sig(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "0123456789abcdef"
    exp = int(time.time()) + 300
    from conftest import create_meta

    create_meta(upload_dir, ticket_id, exp, secret)

    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig=wrongsig", content=b"data")
    assert response.status_code == 403
    assert response.json() == {"error": "invalid_sig"}


def test_error_expired(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "0123456789abcdef"
    exp = int(time.time()) - 10
    from conftest import create_meta

    sig = create_meta(upload_dir, ticket_id, exp, secret)

    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig={sig}", content=b"data")
    assert response.status_code == 410
    assert response.json() == {"error": "ticket_expired"}


def test_error_no_meta(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, _, secret = upload_client
    ticket_id = "0123456789abcdef"
    exp = int(time.time()) + 300
    from conftest import sign_ticket

    sig = sign_ticket(ticket_id, exp, secret)

    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig={sig}", content=b"data")
    assert response.status_code == 404
    assert response.json() == {"error": "ticket_not_found"}


def test_error_already_uploaded(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "0123456789abcdef"
    exp = int(time.time()) + 300
    from conftest import create_meta

    sig = create_meta(upload_dir, ticket_id, exp, secret)
    (upload_dir / f"{ticket_id}.bin").write_bytes(b"existing binary")

    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig={sig}", content=b"data")
    assert response.status_code == 409
    assert response.json() == {"error": "already_uploaded"}


def test_error_consumed(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "0123456789abcdef"
    exp = int(time.time()) + 300
    from conftest import create_meta

    sig = create_meta(upload_dir, ticket_id, exp, secret)
    (upload_dir / f"{ticket_id}.consumed").write_bytes(b"consumed marker")

    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig={sig}", content=b"data")
    assert response.status_code == 410
    assert response.json() == {"error": "ticket_consumed"}


def test_error_empty_body(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "0123456789abcdef"
    exp = int(time.time()) + 300
    from conftest import create_meta

    sig = create_meta(upload_dir, ticket_id, exp, secret)

    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig={sig}", content=b"")
    assert response.status_code == 400
    assert response.json() == {"error": "empty_body"}


def test_error_over_max_upload_bytes(
    upload_client: tuple[TestClient, Path, bytes],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "0123456789abcdef"
    exp = int(time.time()) + 300
    from conftest import create_meta

    sig = create_meta(upload_dir, ticket_id, exp, secret)

    monkeypatch.setattr(upload_module, "MAX_UPLOAD_BYTES", 10)
    response = client.put(f"/upload/{ticket_id}?exp={exp}&sig={sig}", content=b"x" * 20)
    assert response.status_code == 413
    assert response.json() == {"error": "photo_too_large"}


def test_error_bad_magic_bytes(
    upload_client: tuple[TestClient, Path, bytes],
) -> None:
    client, upload_dir, secret = upload_client
    ticket_id = "0123456789abcdef"
    exp = int(time.time()) + 300
    from conftest import create_meta

    sig = create_meta(upload_dir, ticket_id, exp, secret)

    response = client.put(
        f"/upload/{ticket_id}?exp={exp}&sig={sig}",
        content=b"not-an-image-payload-data",
    )
    assert response.status_code == 415
    assert response.json() == {"error": "not_a_supported_image_format"}
