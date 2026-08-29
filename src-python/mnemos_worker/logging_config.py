"""structlog -> line-delimited JSON on stderr (BACKEND_STANDARDS §2). Never
stdout — that's the JSON-RPC channel and a stray log line there corrupts
framing.
"""

from __future__ import annotations

import logging
import sys

import structlog


def configure(level: int = logging.INFO) -> None:
    logging.basicConfig(stream=sys.stderr, level=level, format="%(message)s")
    structlog.configure(
        processors=[
            structlog.processors.TimeStamper(fmt="iso", key="ts"),
            structlog.processors.add_log_level,
            structlog.processors.JSONRenderer(),
        ],
        logger_factory=structlog.PrintLoggerFactory(file=sys.stderr),
        wrapper_class=structlog.make_filtering_bound_logger(level),
        cache_logger_on_first_use=True,
    )


def get_logger(**initial_values: object) -> structlog.BoundLogger:
    return structlog.get_logger(**initial_values)
