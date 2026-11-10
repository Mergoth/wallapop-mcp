"""Tests verifying Dockerfile, docker-compose.yml, and deployment scripts."""

import re
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent.parent


def test_compose_file() -> None:
    compose_path = REPO_ROOT / "docker-compose.yml"
    assert compose_path.exists()
    content = compose_path.read_text(encoding="utf-8")

    assert "docker.sock" not in content
    assert "./state" not in content
    assert not re.search(r"\bGOOGLE_[A-Z0-9_]+", content)
    assert not re.search(r"\bMCP_[A-Z0-9_]+", content)

    # Service name
    assert "wallapop-mcp:" in content

    # Must have NO ports
    assert "ports:" not in content

    # External network mcp-net
    assert "mcp-net" in content
    assert re.search(r"networks:\s*\n\s*mcp-net:\s*\n\s*external:\s*true", content)

    # Security settings
    assert "read_only: true" in content
    assert "cap_drop: [ALL]" in content
    assert "no-new-privileges:true" in content

    # :? guarded required variables
    for var in ("GATEWAY_REGISTRATION_TOKEN", "WALLAPOP_UPSTREAM_TOKEN", "UPLOAD_SECRET"):
        pattern = rf"\$\{{{var}:(\?[^}}]*|\?)\}}"
        assert re.search(pattern, content), f"Variable {var} missing :? guard in docker-compose.yml"


def test_dockerfile() -> None:
    dockerfile_path = REPO_ROOT / "Dockerfile"
    assert dockerfile_path.exists()
    content = dockerfile_path.read_text(encoding="utf-8")

    assert "GIT_SHA" in content
    assert "/etc/wallapop-mcp-sha" in content
    assert "server/vendor" in content
    assert "gateway/" not in content
    assert "gateway" not in content


def test_scripts_syntax() -> None:
    deploy_script = REPO_ROOT / "scripts" / "deploy-nas.sh"
    vendor_script = REPO_ROOT / "scripts" / "vendor-gateway.sh"

    assert deploy_script.exists()
    assert vendor_script.exists()

    res_deploy = subprocess.run(
        ["bash", "-n", str(deploy_script)],
        check=False,
        capture_output=True,
        text=True,
    )
    assert res_deploy.returncode == 0, res_deploy.stderr

    res_vendor = subprocess.run(
        ["bash", "-n", str(vendor_script)],
        check=False,
        capture_output=True,
        text=True,
    )
    assert res_vendor.returncode == 0, res_vendor.stderr


def test_deploy_script_content_and_dry_run() -> None:
    deploy_script = REPO_ROOT / "scripts" / "deploy-nas.sh"
    content = deploy_script.read_text(encoding="utf-8")

    assert "| tail" not in content
    assert "/usr/local/bin/docker" in content
    assert "ServerAliveInterval=5" in content
    assert "server" in content
    assert "gateway" not in content

    # Compares remote sha with local sha
    assert "/etc/wallapop-mcp-sha" in content
    assert "GIT_SHA" in content
    assert "REMOTE_SHA" in content

    # Tar packing settings
    assert "COPYFILE_DISABLE=1" in content
    excludes = ("server/.venv", "server/.pytest_cache", "server/.ruff_cache", "__pycache__")
    for pattern in excludes:
        assert f"--exclude '{pattern}'" in content or f"--exclude {pattern}" in content

    # server/vendor must not be excluded
    exclude_lines = [line for line in content.splitlines() if "--exclude" in line]
    assert not any("server/vendor" in line for line in exclude_lines)

    # Dry-run execution exits 0 without ssh or cargo
    res_dry = subprocess.run(
        [str(deploy_script), "--dry-run"],
        check=False,
        capture_output=True,
        text=True,
    )
    assert res_dry.returncode == 0, res_dry.stderr
    assert "[dry-run]" in res_dry.stdout
    assert "server" in res_dry.stdout
    for pattern in excludes:
        assert pattern in res_dry.stdout
