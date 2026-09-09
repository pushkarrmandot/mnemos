"""Method registry: `@method("name")` decorates a plain handler function,
`DISPATCH_TABLE` collects them.
"""

from __future__ import annotations

from typing import Any, Callable

Handler = Callable[[dict[str, Any]], dict[str, Any]]

DISPATCH_TABLE: dict[str, Handler] = {}


def method(name: str) -> Callable[[Handler], Handler]:
    def register(fn: Handler) -> Handler:
        DISPATCH_TABLE[name] = fn
        return fn

    return register
