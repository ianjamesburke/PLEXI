from __future__ import annotations


class KeyMap:
    """Declarative keyboard shortcut registry.

    Usage::

        km = KeyMap()
        km.bind("s", mod="cmd", action="save")
        km.bind("q", action="quit")

        def on_key(self, key, mods):
            action = km.handle(key, mods)
            if action == "save":
                save()
            elif action == "quit":
                quit()
    """

    def __init__(self) -> None:
        # key: (normalized_key, mod_string) -> action_name
        self._bindings: dict[tuple[str, str], str] = {}

    def bind(self, key: str, action: str, mod: str = "") -> None:
        """Bind a key + optional modifier to an action name.

        `mod` is a pipe-separated string of modifier names:
        "cmd", "shift", "ctrl", "alt". Order does not matter.
        Example: ``km.bind("s", "save", mod="cmd")``
        """
        normalized_mod = "|".join(sorted(mod.split("|"))) if mod else ""
        self._bindings[(key.lower(), normalized_mod)] = action

    def handle(self, key: str, mods: dict) -> "str | None":
        """Return the action name for this key+mods, or None if unbound.

        `mods` is the dict from on_key: {"shift": bool, "ctrl": bool,
        "alt": bool, "meta": bool}. "meta" is treated as "cmd".
        """
        active_mods = []
        if mods.get("meta") or mods.get("cmd"):
            active_mods.append("cmd")
        if mods.get("shift"):
            active_mods.append("shift")
        if mods.get("ctrl"):
            active_mods.append("ctrl")
        if mods.get("alt"):
            active_mods.append("alt")
        mod_str = "|".join(sorted(active_mods))
        return self._bindings.get((key.lower(), mod_str))
