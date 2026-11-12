"""View builders for Cli Op Fixture.

Pure functions from app data to a component tree: no effects, no state reads
beyond what ``main.py`` passes in.
"""

from plexi_sdk.ui import SPACE_MD, SurfaceHeader, Column, FooterKeys, Text


def build_view(status: str, last: str) -> Column:
    return Column([
        SurfaceHeader("Cli Op Fixture"),
        Text(status or "ready", bold=True),
        Text(last or "no result yet"),
        FooterKeys([("r", "read"), ("m", "mutate"), ("s", "slow"), ("c", "cancel")]),
    ], grow=True, padding=SPACE_MD)
