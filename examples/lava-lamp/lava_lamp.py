#!/usr/bin/env python3
"""Lava Lamp — buoyancy-driven blobs with fake-metaball blending.

Blobs rise when hot, sink when cool. Nearby blobs render translucent bridge
circles between them to simulate the characteristic merging-and-splitting look.
"""
from __future__ import annotations

import math
import random
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext, dim

if __name__ == "__main__":
    pass  # entry point wired in Task 4
