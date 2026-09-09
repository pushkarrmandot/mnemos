"""`@method(...)` registers plain handler functions into the shared
`DISPATCH_TABLE`. These tests use their own throwaway names so they don't
collide with the real handlers registered elsewhere in the package at import
time, and clean up after themselves so the shared module-level dict doesn't
leak state into other tests.
"""

from mnemos_worker.dispatch import DISPATCH_TABLE, method


def _unregister(name: str) -> None:
    DISPATCH_TABLE.pop(name, None)


def test_method_registers_the_function_under_its_name():
    try:

        @method("test_dispatch.ping")
        def handler(params):
            return {"pong": True}

        assert DISPATCH_TABLE["test_dispatch.ping"] is handler
    finally:
        _unregister("test_dispatch.ping")


def test_method_returns_the_original_function_unchanged():
    try:

        def handler(params):
            return params

        decorated = method("test_dispatch.identity")(handler)
        assert decorated is handler
        assert decorated({"a": 1}) == {"a": 1}
    finally:
        _unregister("test_dispatch.identity")


def test_registered_handler_is_callable_from_the_table():
    try:

        @method("test_dispatch.add")
        def handler(params):
            return {"sum": params["a"] + params["b"]}

        result = DISPATCH_TABLE["test_dispatch.add"]({"a": 2, "b": 3})
        assert result == {"sum": 5}
    finally:
        _unregister("test_dispatch.add")


def test_registering_the_same_name_twice_overwrites_the_earlier_handler():
    try:

        @method("test_dispatch.dup")
        def first(params):
            return {"which": "first"}

        @method("test_dispatch.dup")
        def second(params):
            return {"which": "second"}

        assert DISPATCH_TABLE["test_dispatch.dup"] is second
        assert DISPATCH_TABLE["test_dispatch.dup"]({}) == {"which": "second"}
    finally:
        _unregister("test_dispatch.dup")
