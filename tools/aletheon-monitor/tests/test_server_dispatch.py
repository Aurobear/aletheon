"""Regression tests for the server dispatch contract.

`call_tool` does `result = await handler(get_client(), arguments)`, so every
entry in `_HANDLERS` must return an awaitable. This guards the historical bug
where `aletheon_check_install` mapped to a sync function and blew up with
`TypeError: object dict can't be used in 'await' expression`.
"""
import asyncio
import inspect

from src.server import _HANDLERS, validate_installation


def test_validate_installation_is_coroutine_function():
    assert inspect.iscoroutinefunction(validate_installation)


def test_check_install_handler_returns_awaitable_dict():
    result = asyncio.run(_HANDLERS["aletheon_check_install"](None, {}))
    # No daemon in the test env → ok:False, but it must be a dict, not a crash.
    assert isinstance(result, dict)
    assert "ok" in result
