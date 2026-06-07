#!/usr/bin/env python3
"""Kraken — a text-adventure stress test for inventory-conditional rendering."""
from plexi_sdk import App
from plexi_sdk.ui import AppBar, Card, Column, FooterKeys, Label, Spacer


NODES = {
    "wake_up": {
        "title": "Below Deck",
        "text": "You wake to the ship lurching. Screams outside. Cabin is flooding ankle-deep.",
    },
    "deck": {
        "title": "Main Deck",
        "text": "Chaos. The kraken's tentacle sweeps across the bow. Water everywhere.",
    },
    "bridge": {
        "title": "Bridge",
        "text": "Captain is gone. A tentacle smashes through the window.",
    },
    "survive": {
        "title": "Rescued",
        "text": "You leap into the dark water. The life preserver keeps you afloat. You watch the ship go down. Rescued at dawn.",
    },
    "death": {
        "title": "Crushed",
        "text": "The tentacle crushes the bridge. Everything goes dark.",
    },
}


class Kraken(App):
    def on_init(self) -> None:
        self.node: str = self.state.get("node", "wake_up")
        self.inventory: list = self.state.get("inventory", [])
        self.deaths: int = self.state.get("deaths", 0)

    def view(self):
        node = NODES.get(self.node, NODES["wake_up"])
        children = [
            AppBar(title=f"Kraken: {node['title']}"),
            Card(children=[Label(node["text"])]),
            Spacer(grow=True),
        ]

        if self.node == "survive":
            children.insert(2, Label("YOU SURVIVED", bold=True))
            children.insert(3, Label(f"Deaths: {self.deaths}"))
            children.append(FooterKeys(shortcuts=[("r", "restart")]))
        elif self.node == "death":
            children.insert(2, Label("YOU DIED", bold=True))
            children.insert(3, Label(f"Deaths: {self.deaths}"))
            children.append(FooterKeys(shortcuts=[("r", "restart")]))
        else:
            children.append(FooterKeys(shortcuts=self._options()))

        return Column(children)

    def _options(self) -> list:
        if self.node == "wake_up":
            return [("1", "Rush to deck")]
        elif self.node == "deck":
            if "life_preserver" not in self.inventory:
                return [
                    ("1", "Grab life preserver"),
                    ("2", "Run to bridge"),
                ]
            else:
                return [
                    ("1", "Jump overboard"),
                    ("2", "Run to bridge"),
                ]
        elif self.node == "bridge":
            return [("1", "Brace for impact")]
        return []

    def on_key(self, key: str, mods: dict) -> None:
        if key == "r" and self.node in ("survive", "death"):
            self.node = "wake_up"
            self.inventory = []
            self.state.save({"node": self.node, "inventory": self.inventory, "deaths": self.deaths})
            self.emit.schedule_render()
            return

        if self.node == "wake_up":
            if key == "1":
                self.node = "deck"
        elif self.node == "deck":
            if "life_preserver" not in self.inventory:
                if key == "1":
                    self.inventory.append("life_preserver")
                elif key == "2":
                    self.node = "bridge"
            else:
                if key == "1":
                    self.node = "survive"
                elif key == "2":
                    self.node = "bridge"
        elif self.node == "bridge":
            if key == "1":
                self.node = "death"
                self.deaths += 1

        self.state.save({"node": self.node, "inventory": self.inventory, "deaths": self.deaths})
        self.emit.schedule_render()


Kraken().run()
