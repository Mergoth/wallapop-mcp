"""Tests for upload file garbage collection and lifespan management."""

import asyncio
import os
import sys
import time
from pathlib import Path

from wallapop_server.app import build_app
from wallapop_server.settings import Settings
from wallapop_server.upload import _gc_once


def test_gc_once_removes_old_files_and_keeps_young_and_other_suffixes(
    tmp_path: Path,
) -> None:
    async def _test() -> None:
        upload_dir = tmp_path / "uploads"
        upload_dir.mkdir()

        now = time.time()
        old_time = now - 7200  # 2 hours ago

        # Create old files with targeted suffixes
        old_meta = upload_dir / "old.meta"
        old_bin = upload_dir / "old.bin"
        old_consumed = upload_dir / "old.consumed"
        for p in (old_meta, old_bin, old_consumed):
            p.write_bytes(b"old")
            os.utime(p, (old_time, old_time))

        # Create young files with targeted suffixes
        young_meta = upload_dir / "young.meta"
        young_bin = upload_dir / "young.bin"
        young_consumed = upload_dir / "young.consumed"
        for p in (young_meta, young_bin, young_consumed):
            p.write_bytes(b"young")
            os.utime(p, (now, now))

        # Create files with other suffixes (even if old)
        old_txt = upload_dir / "old.txt"
        old_other = upload_dir / "old.other"
        old_meta_bak = upload_dir / "old.meta.bak"
        for p in (old_txt, old_other, old_meta_bak):
            p.write_bytes(b"other")
            os.utime(p, (old_time, old_time))

        # Run GC sweep with max_age = 3600 (1 hour)
        await _gc_once(upload_dir, max_age=3600)

        # Old targeted files must be removed
        assert not old_meta.exists()
        assert not old_bin.exists()
        assert not old_consumed.exists()

        # Young targeted files must be kept
        assert young_meta.exists()
        assert young_bin.exists()
        assert young_consumed.exists()

        # Other suffixes must be kept
        assert old_txt.exists()
        assert old_other.exists()
        assert old_meta_bak.exists()

    asyncio.run(_test())


def test_gc_loop_runs_while_app_runs_and_cancels_at_shutdown(
    tmp_path: Path,
    fake_upstream_path: Path,
) -> None:
    async def _test() -> None:
        upload_dir = tmp_path / "uploads"

        settings = Settings(
            upstream_token="u" * 32,
            gateway_registration_token=None,
            gateway_internal_url="http://mcp-gateway:8081",
            host="127.0.0.1",
            port=8000,
            advertise_url="http://wallapop-mcp:8000/mcp",
            command=sys.executable,
            args=(str(fake_upstream_path.resolve()),),
            child_env={},
            upload_dir=upload_dir,
            upload_secret=b"testsecret",
            no_register=True,
        )

        app = build_app(settings)

        # Directory created on lifespan startup
        assert not upload_dir.exists()

        async with app.router.lifespan_context(app):
            assert upload_dir.exists()
            gc_tasks = [
                t
                for t in asyncio.all_tasks()
                if getattr(t.get_coro(), "__name__", "") == "_gc_loop"
            ]
            assert len(gc_tasks) == 1
            gc_task = gc_tasks[0]
            assert not gc_task.done()

        # After shutdown, task must be cancelled and done
        assert gc_task.done()
        assert gc_task.cancelled()

    asyncio.run(_test())
