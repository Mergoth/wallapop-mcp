"""Entrypoint for wallapop-server."""

import sys

import uvicorn

from wallapop_server.app import build_app
from wallapop_server.settings import ConfigError, Settings


def main() -> None:
    try:
        settings = Settings.from_env()
    except ConfigError as exc:
        sys.stderr.write(f"Configuration error: {exc}\n")
        sys.exit(2)

    app = build_app(settings)
    uvicorn.run(app, host=settings.host, port=settings.port, log_level="info")


if __name__ == "__main__":
    main()
