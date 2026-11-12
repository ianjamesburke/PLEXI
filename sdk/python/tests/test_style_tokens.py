"""Keep SDK tokens shared with the host style scale in sync."""

import re
from pathlib import Path

from plexi_sdk import ui


STYLE = Path(__file__).resolve().parents[3] / "src/ui/style.rs"
SHARED = {
    "SPACE_XS": "SPACE_XS", "SPACE_SM": "SPACE_SM",
    "SPACE_MD": "SPACE_MD", "SPACE_XL": "SPACE_XL",
    "TEXT_HINT": "TEXT_HINT", "TEXT_CAPTION": "TEXT_CAPTION",
    "TEXT_BODY": "TEXT_BODY", "TEXT_TITLE": "TEXT_TITLE",
    "TEXT_TITLE_XL": "TEXT_TITLE_XL", "RADIUS_SM": "RADIUS_SM",
    "RADIUS_MD": "RADIUS_MD", "RADIUS_LG": "RADIUS_LG",
    "RADIUS_BADGE": "RADIUS_BADGE",
}


def host_value(source: str, name: str) -> float:
    match = re.search(rf"^pub const {name}: (?:f32|CornerRadius) = ([^;]+);", source, re.MULTILINE)
    assert match, f"missing host token {name}"
    value = match.group(1)
    if value.startswith("CornerRadius::same("):
        value = value.removeprefix("CornerRadius::same(").removesuffix(")")
    if value.startswith("RADIUS_"):
        return host_value(source, value)
    return float(value)


def test_shared_style_tokens_match_host():
    source = STYLE.read_text(encoding="utf-8")
    host_names = set(re.findall(r"^pub const ((?:SPACE|TEXT|RADIUS)_[A-Z_]+):", source, re.MULTILINE))
    sdk_names = {name for name in ui.__all__ if name.startswith(("SPACE_", "TEXT_", "RADIUS_"))}
    assert sdk_names & host_names <= SHARED.keys(), "new shared token needs a drift check"
    for sdk_name, host_name in SHARED.items():
        assert getattr(ui, sdk_name) == host_value(source, host_name), sdk_name
