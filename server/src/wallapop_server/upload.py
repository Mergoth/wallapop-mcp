"""PUT /upload/{ticket_id} — receives a photo binary and stores it for Rust to consume."""

import asyncio
import base64
import hashlib
import hmac
import logging
import time
from pathlib import Path

from starlette.requests import Request
from starlette.responses import JSONResponse

logger = logging.getLogger(__name__)

MAX_UPLOAD_BYTES = 15 * 1024 * 1024  # 15 MiB
GC_INTERVAL = 600  # seconds between GC sweeps
GC_MAX_AGE = 3600  # files older than 1 h are eligible


def _check_magic(data: bytes) -> bool:
    if data[:3] == b"\xff\xd8\xff":
        return True
    if data[:4] == b"\x89PNG":
        return True
    return bool(len(data) >= 12 and data[:4] == b"RIFF" and data[8:12] == b"WEBP")


def _verify_sig(ticket_id: str, exp_str: str, sig: str, secret: bytes) -> bool:
    msg = f"{ticket_id}.{exp_str}".encode()
    expected = hmac.new(secret, msg, hashlib.sha256).digest()
    expected_b64 = base64.urlsafe_b64encode(expected).rstrip(b"=").decode()
    return hmac.compare_digest(expected_b64, sig)


async def handle_upload(request: Request) -> JSONResponse:
    ticket_id: str = request.path_params["ticket_id"]
    upload_dir: Path = request.app.state.upload_dir
    secret: bytes = request.app.state.upload_secret

    # Validate ticket_id: hex-only, 1–64 chars — prevents path traversal.
    if not ticket_id or len(ticket_id) > 64 or not all(
        c in "0123456789abcdef" for c in ticket_id
    ):
        return JSONResponse({"error": "invalid_ticket_id"}, status_code=400)

    exp_str = request.query_params.get("exp", "")
    sig = request.query_params.get("sig", "")
    if not exp_str or not sig:
        return JSONResponse({"error": "missing_exp_or_sig"}, status_code=400)

    try:
        exp = int(exp_str)
    except ValueError:
        return JSONResponse({"error": "invalid_exp"}, status_code=400)

    if not _verify_sig(ticket_id, exp_str, sig, secret):
        return JSONResponse({"error": "invalid_sig"}, status_code=403)

    if time.time() > exp:
        return JSONResponse({"error": "ticket_expired"}, status_code=410)

    meta_path = upload_dir / f"{ticket_id}.meta"
    bin_path = upload_dir / f"{ticket_id}.bin"
    consumed_path = upload_dir / f"{ticket_id}.consumed"

    if not meta_path.exists():
        return JSONResponse({"error": "ticket_not_found"}, status_code=404)
    if bin_path.exists():
        return JSONResponse({"error": "already_uploaded"}, status_code=409)
    if consumed_path.exists():
        return JSONResponse({"error": "ticket_consumed"}, status_code=410)

    body = await request.body()
    if len(body) == 0:
        return JSONResponse({"error": "empty_body"}, status_code=400)
    if len(body) > MAX_UPLOAD_BYTES:
        return JSONResponse({"error": "photo_too_large"}, status_code=413)

    if not _check_magic(body):
        return JSONResponse({"error": "not_a_supported_image_format"}, status_code=415)

    bin_path.write_bytes(body)
    logger.info("upload ticket %s stored (%d bytes)", ticket_id, len(body))
    return JSONResponse({"ref": f"upload:{ticket_id}", "bytes": len(body)}, status_code=201)


async def _gc_once(upload_dir: Path, *, max_age: float = GC_MAX_AGE) -> None:
    if not upload_dir.exists():
        return
    cutoff = time.time() - max_age
    for path in upload_dir.iterdir():
        if path.suffix in {".meta", ".bin", ".consumed"}:
            try:
                if path.stat().st_mtime < cutoff:
                    path.unlink(missing_ok=True)
            except OSError:
                pass


async def _gc_loop(
    upload_dir: Path,
    *,
    interval: float = GC_INTERVAL,
    max_age: float = GC_MAX_AGE,
) -> None:
    while True:
        try:
            await _gc_once(upload_dir, max_age=max_age)
        except Exception:
            logger.exception("upload GC error")
        await asyncio.sleep(interval)
