from __future__ import annotations

import warnings


class ScrollState:
    """Pure data model for vertical scroll state.

    Offset is always clamped to [0, max_offset] after any mutation.
    Negative inputs for content_height and viewport_height are clamped to 0
    with a warning rather than raising, so callers with stale layout data
    degrade gracefully.
    """

    def __init__(
        self,
        content_height: float,
        viewport_height: float,
        offset: float = 0.0,
    ) -> None:
        self._content_height: float = self._validate_dimension(content_height, "content_height")
        self._viewport_height: float = self._validate_dimension(viewport_height, "viewport_height")
        self._offset: float = self._clamp(offset)

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _validate_dimension(value: float, name: str) -> float:
        if value < 0.0:
            warnings.warn(
                f"ScrollState: {name} must be >= 0, got {value!r}; clamping to 0.0",
                stacklevel=3,
            )
            return 0.0
        return float(value)

    def _clamp(self, offset: float) -> float:
        return max(0.0, min(offset, self.max_offset))

    # ------------------------------------------------------------------
    # Properties
    # ------------------------------------------------------------------

    @property
    def content_height(self) -> float:
        return self._content_height

    @content_height.setter
    def content_height(self, value: float) -> None:
        self._content_height = self._validate_dimension(value, "content_height")
        self._offset = self._clamp(self._offset)

    @property
    def viewport_height(self) -> float:
        return self._viewport_height

    @viewport_height.setter
    def viewport_height(self, value: float) -> None:
        self._viewport_height = self._validate_dimension(value, "viewport_height")
        self._offset = self._clamp(self._offset)

    @property
    def offset(self) -> float:
        return self._offset

    @property
    def max_offset(self) -> float:
        return max(0.0, self._content_height - self._viewport_height)

    # ------------------------------------------------------------------
    # Mutation
    # ------------------------------------------------------------------

    def scroll_by(self, dy: float) -> None:
        """Positive dy scrolls down (reveals content below)."""
        self._offset = self._clamp(self._offset + dy)

    def scroll_to(self, y: float) -> None:
        """Set offset directly. Clamped to [0, max_offset]."""
        self._offset = self._clamp(y)

    def ensure_visible(self, y: float, margin: float = 0.0) -> None:
        """Adjust offset so content-coord y is inside the viewport.

        If y is above the viewport (y < offset + margin), scroll up so
        y lands at offset + margin.
        If y is below the viewport (y > offset + viewport_height - margin),
        scroll down so y lands at offset + viewport_height - margin.
        Otherwise no-op. Result is always clamped.
        """
        # Clamp margin to avoid nonsensical values larger than half the viewport.
        safe_margin = max(0.0, margin)

        if y < self._offset + safe_margin:
            # Scroll up: bring y to the top edge (plus margin).
            self._offset = self._clamp(y - safe_margin)
        elif y > self._offset + self._viewport_height - safe_margin:
            # Scroll down: bring y to the bottom edge (minus margin).
            self._offset = self._clamp(y - self._viewport_height + safe_margin)
