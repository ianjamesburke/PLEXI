"""Badge/status color contract: semantic theme roles only, never literal
colors (see AUTHORING.md). Covers the bug where `Badge(color="blue")` used
to reach `to_node()` unvalidated and only failed later at host decode time.
"""

import pytest

from plexi_sdk.ui import BADGE_COLORS, Badge, Banner


class TestBadgeColorValidation:
    @pytest.mark.parametrize("color", BADGE_COLORS)
    def test_valid_colors_construct(self, color: str) -> None:
        badge = Badge("label", color=color)
        assert badge.to_node()["color"] == color

    def test_unknown_color_raises(self) -> None:
        with pytest.raises(ValueError, match=r"Badge.*blue"):
            Badge("label", color="blue")

    def test_tone_alias_is_validated_too(self) -> None:
        # `tone` overlays onto `color` in __post_init__; an invalid tone must
        # raise the same way an invalid `color` does.
        with pytest.raises(ValueError, match=r"Badge.*blue"):
            Badge("label", tone="blue")

    def test_theme_role_aliases_accepted(self) -> None:
        for alias in ("red", "green", "yellow"):
            badge = Badge("label", color=alias)
            assert badge.to_node()["color"] == alias


class TestBannerToneValidation:
    def test_default_untoned_banner_renders_neutral(self) -> None:
        banner = Banner("hello")
        node = banner.to_node()
        badge_node = node["children"][0]
        assert badge_node["color"] == "neutral"

    @pytest.mark.parametrize("tone", ["accent", "success", "warning", "danger", "red", "green", "yellow"])
    def test_valid_tones_construct(self, tone: str) -> None:
        banner = Banner("hello", tone=tone)
        node = banner.to_node()
        assert node["children"][0]["color"] == tone

    def test_unknown_tone_raises(self) -> None:
        with pytest.raises(ValueError, match=r"Banner.*blue"):
            Banner("hello", tone="blue")
