import logging
import sys

import uvicorn

from wallapop_gateway.app import build_app
from wallapop_gateway.config import ConfigError, load_settings


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s: %(message)s")
    try:
        settings = load_settings()
        app = build_app(settings)
    except ConfigError as exc:
        sys.exit(f"wallapop-gateway: {exc}")
    uvicorn.run(app, host=settings.host, port=settings.port, log_level="info")


if __name__ == "__main__":
    main()
