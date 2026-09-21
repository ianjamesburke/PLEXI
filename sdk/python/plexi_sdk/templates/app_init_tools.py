"""Assistant-callable tools for __DISPLAY_NAME__.

Each tool is a plain Python function registered with ``@tools.tool``. Keep
this module free of UI code: it answers the Assistant, ``app_ui.py`` draws.
``main.py`` returns ``tools.expose()`` from ``init`` and calls
``tools.dispatch(event)`` first in ``update``.

Named ``app_tools`` (not ``tools``) so it never shadows ``plexi_sdk.tools``.
Read-only tools pass ``read_only=True`` and skip the write-grant prompt; a
mutating tool returns ``tools.Reply(output, effects)`` with its effects.
"""

from plexi_sdk import state, tools


@tools.tool(
    "demo.greet",
    "Greet someone by name.",
    {"name": str},
    {"greeting": str},
    read_only=True,
)
def greet(name: str) -> dict:
    return {"greeting": f"Hello, {name}!"}


@tools.tool(
    "demo.count",
    "Read the current counter value.",
    returns={"count": int},
    read_only=True,
)
def count() -> dict:
    return {"count": state.get("count", 0)}
