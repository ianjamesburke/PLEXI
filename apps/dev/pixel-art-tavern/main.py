#!/usr/bin/env python3
"""Pixel Art Tavern — AI NPCs converse via ai_query + PGAP canvas.

Three named characters take turns speaking. One active ai_query at a time,
fired via schedule_task. Speech bubbles persist until replaced by new dialogue.
"""
from __future__ import annotations

import copy
from dataclasses import dataclass
from plexi_sdk import App, RenderContext

# ── Design constants ──────────────────────────────────────────────────────────
PIXEL   = 4.0    # logical px per sprite pixel
COL_W   = 12     # sprite grid width in pixels
COL_H   = 16     # sprite grid height in pixels
PAD     = 16.0
CAPTION = 12.0
BODY    = 14.0
HEADING = 18.0

TURN_DELAY_SECS = 0.8   # pause after response before next query
ANIM_FPS        = 3.0   # idle animation frame rate

# ── Palette ───────────────────────────────────────────────────────────────────
_ = None   # transparent
W = "#cdd6f4"  # white
S = "#a6adc8"  # shadow
G = "#45475a"  # gray
K = "#1e1e2e"  # black / outline
B = "#89b4fa"  # blue
R = "#f38ba8"  # red
Y = "#f9e2af"  # yellow
N = "#c9a46e"  # skin/wood
O = "#fab387"  # orange
P = "#cba6f7"  # purple
T = "#94e2d5"  # teal

# Tavern-themed bubble palette
BUBBLE_BG    = "#2a1f0f"   # deep parchment brown (border)
BUBBLE_FILL  = "#f5e6c8"   # warm parchment
BUBBLE_GOLD  = "#8b6914"   # aged wood gold (border accent)

# Sprites: list of rows (top→bottom), each row is a list of COL_W color|None
# Frame 0 = normal, Frame 1 = arms-raised / eyes-closed (idle blink)

BRAM_FRAMES = [
    # frame 0 — arms neutral
    [
        [_, _, _, K, K, K, K, K, K, _, _, _],
        [_, _, K, N, N, N, N, N, N, K, _, _],
        [_, _, K, N, N, N, N, N, N, K, _, _],
        [_, _, K, N, G, N, N, G, N, K, _, _],
        [_, _, K, N, N, N, N, N, N, K, _, _],
        [_, _, K, N, N, Y, N, N, N, K, _, _],
        [_, _, _, K, K, K, K, K, K, _, _, _],
        [_, K, O, O, O, O, O, O, O, O, K, _],
        [_, K, O, O, O, O, O, O, O, O, K, _],
        [K, O, O, O, O, O, O, O, O, O, O, K],
        [K, O, O, O, O, O, O, O, O, O, O, K],
        [_, K, O, O, O, O, O, O, O, O, K, _],
        [_, K, O, O, O, O, O, O, O, O, K, _],
        [_, _, K, K, O, O, O, O, K, K, _, _],
        [_, _, _, K, O, O, O, O, K, _, _, _],
        [_, _, _, K, K, _, _, K, K, _, _, _],
    ],
    # frame 1 — wiping bar (arms slightly up)
    [
        [_, _, _, K, K, K, K, K, K, _, _, _],
        [_, _, K, N, N, N, N, N, N, K, _, _],
        [_, _, K, N, N, N, N, N, N, K, _, _],
        [_, _, K, N, G, N, N, G, N, K, _, _],
        [_, _, K, N, N, N, N, N, N, K, _, _],
        [_, _, K, N, N, Y, N, N, N, K, _, _],
        [_, _, _, K, K, K, K, K, K, _, _, _],
        [K, O, O, O, O, O, O, O, O, O, O, K],
        [K, O, O, O, O, O, O, O, O, O, O, K],
        [_, K, O, O, O, O, O, O, O, O, K, _],
        [_, K, O, O, O, O, O, O, O, O, K, _],
        [_, _, K, O, O, O, O, O, O, K, _, _],
        [_, _, K, O, O, O, O, O, O, K, _, _],
        [_, _, K, K, O, O, O, O, K, K, _, _],
        [_, _, _, K, O, O, O, O, K, _, _, _],
        [_, _, _, K, K, _, _, K, K, _, _, _],
    ],
]

LYRA_FRAMES = [
    # frame 0 — staff raised
    [
        [_, _, K, P, P, K, K, P, P, K, _, _],
        [_, _, K, P, P, P, P, P, P, K, _, _],
        [_, K, N, N, N, N, N, N, N, N, K, _],
        [_, K, N, G, N, N, N, G, N, N, K, _],
        [_, K, N, N, N, N, N, N, N, N, K, _],
        [_, K, N, N, P, N, N, N, N, N, K, _],
        [_, _, K, K, K, K, K, K, K, K, _, _],
        [_, _, K, B, B, B, B, B, B, K, _, _],
        [_, _, K, B, B, B, B, B, B, K, _, _],
        [_, K, B, B, B, B, B, B, B, B, K, _],
        [K, S, B, B, B, B, B, B, B, B, S, K],
        [_, K, B, B, B, B, B, B, B, B, K, _],
        [_, _, K, B, B, B, B, B, B, K, _, _],
        [_, _, K, K, B, B, B, B, K, K, _, _],
        [_, _, _, K, B, B, B, B, K, _, _, _],
        [_, _, _, K, K, _, _, K, K, _, _, _],
    ],
    # frame 1 — staff sparkling (slight arm movement)
    [
        [_, _, K, P, P, K, K, P, P, K, _, _],
        [_, _, K, P, P, P, P, P, P, K, _, _],
        [_, K, N, N, N, N, N, N, N, N, K, _],
        [_, K, N, G, N, N, N, G, N, N, K, _],
        [_, K, N, N, N, N, N, N, N, N, K, _],
        [_, K, N, N, P, N, N, N, N, N, K, _],
        [_, _, K, K, K, K, K, K, K, K, _, _],
        [_, K, T, B, B, B, B, B, B, T, K, _],
        [_, _, K, B, B, B, B, B, B, K, _, _],
        [_, K, B, B, B, B, B, B, B, B, K, _],
        [K, S, B, B, B, B, B, B, B, B, S, K],
        [_, K, B, B, B, B, B, B, B, B, K, _],
        [_, _, K, B, B, B, B, B, B, K, _, _],
        [_, _, K, K, B, B, B, B, K, K, _, _],
        [_, _, _, K, B, B, B, B, K, _, _, _],
        [_, _, _, K, K, _, _, K, K, _, _, _],
    ],
]

DREXL_FRAMES = [
    # frame 0 — resting
    [
        [_, K, K, K, K, K, K, K, K, K, K, _],
        [K, G, G, G, G, G, G, G, G, G, G, K],
        [K, G, N, N, N, N, N, N, N, N, G, K],
        [K, G, N, G, N, N, N, G, N, N, G, K],
        [K, G, N, N, N, N, N, N, N, N, G, K],
        [K, G, N, N, R, N, N, N, N, N, G, K],
        [_, K, K, K, K, K, K, K, K, K, K, _],
        [K, G, G, R, R, R, R, R, R, G, G, K],
        [K, G, G, R, R, R, R, R, R, G, G, K],
        [K, G, G, R, R, R, R, R, R, G, G, K],
        [G, G, G, R, R, R, R, R, R, G, G, G],
        [K, G, G, R, R, R, R, R, R, G, G, K],
        [K, G, G, R, R, R, R, R, R, G, G, K],
        [_, K, K, K, R, R, R, R, K, K, K, _],
        [_, _, _, K, R, R, R, R, K, _, _, _],
        [_, _, _, K, K, _, _, K, K, _, _, _],
    ],
    # frame 1 — shifting weight
    [
        [_, K, K, K, K, K, K, K, K, K, K, _],
        [K, G, G, G, G, G, G, G, G, G, G, K],
        [K, G, N, N, N, N, N, N, N, N, G, K],
        [K, G, N, G, N, N, N, G, N, N, G, K],
        [K, G, N, N, N, N, N, N, N, N, G, K],
        [K, G, N, N, R, N, N, N, N, N, G, K],
        [_, K, K, K, K, K, K, K, K, K, K, _],
        [_, K, G, R, R, R, R, R, R, G, K, _],
        [K, G, G, R, R, R, R, R, R, G, G, K],
        [K, G, G, R, R, R, R, R, R, G, G, K],
        [G, G, G, R, R, R, R, R, R, G, G, G],
        [K, G, G, R, R, R, R, R, R, G, G, K],
        [K, G, G, R, R, R, R, R, R, G, G, K],
        [_, K, K, K, R, R, R, R, K, K, K, _],
        [_, _, _, K, R, R, R, R, K, _, _, _],
        [_, _, _, K, K, _, _, K, K, _, _, _],
    ],
]


@dataclass
class NPC:
    name: str
    title: str
    color: str
    system_prompt: str
    frames: list[list[list[str | None]]]
    cx: float = 0.0   # canvas center x (set during layout)
    speech: str = ""


NPCS: list[NPC] = [
    NPC(
        name="Bram",
        title="The Bartender",
        color=O,
        frames=BRAM_FRAMES,
        system_prompt=(
            "You are Bram, the bartender of The Rusty Flagon tavern. "
            "Speak only in dialogue — no action descriptions, no asterisks, no narration. "
            "You are gruff, warm, and world-weary. Keep it to 1-2 short sentences. "
            "Open with: 'Welcome to the Rusty Flagon. What'll it be?'"
        ),
    ),
    NPC(
        name="Lyra",
        title="The Mage",
        color=P,
        frames=LYRA_FRAMES,
        system_prompt=(
            "You are Lyra, an excitable young mage drinking at the tavern. "
            "Speak only in dialogue — no action descriptions, no asterisks, no narration. "
            "You are curious and enthusiastic, with a flair for magical metaphor. 1-2 sentences."
        ),
    ),
    NPC(
        name="Drexl",
        title="The Warrior",
        color=R,
        frames=DREXL_FRAMES,
        system_prompt=(
            "You are Drexl, a battle-hardened warrior nursing a drink at the tavern. "
            "Speak only in dialogue — no action descriptions, no asterisks, no narration. "
            "You are blunt and skeptical, especially of magic. 1-2 short sentences."
        ),
    ),
]


class TavernApp(App):
    async def on_init(self, _ctx: RenderContext) -> None:
        self._npcs = copy.deepcopy(NPCS)
        self._turn_idx = 0
        self._state = "idle"   # "idle" | "thinking" | "pausing"
        self._anim_accum = 0.0
        self._anim_frame = 0
        self._pause_timer = 0.0
        self._total_elapsed = 0.0
        self._conversation: list[dict] = []
        self._started = False
        self.emit.info(f"pixel-art-tavern initialized, {len(self._npcs)} NPCs loaded")

    def on_render(self, ctx: RenderContext) -> None:
        dt = min(ctx.elapsed, 0.1)
        self._tick(dt)

        ctx.clear("#1a1208")   # deep tavern brown

        # Layout: distribute NPCs evenly
        sprite_w = COL_W * PIXEL
        sprite_h = COL_H * PIXEL
        usable_w = ctx.w - PAD * 2
        spacing = usable_w / len(self._npcs)
        floor_y = ctx.h - PAD - sprite_h - 24.0

        for i, npc in enumerate(self._npcs):
            npc.cx = PAD + spacing * i + spacing * 0.5

        # Tavern floor strip
        ctx.rect(0, floor_y + sprite_h, ctx.w, 24.0, fill="#3d2b1a", radius=0.0)

        # Start button overlay
        if not self._started:
            self._draw_start_prompt(ctx, floor_y, sprite_w, sprite_h)
        else:
            self._draw_scene(ctx, floor_y, sprite_w, sprite_h)

        # Title
        ctx.text(PAD, PAD, "The Rusty Flagon", size=HEADING, color=Y, bold=True, monospace=True)
        ctx.text(PAD, PAD + HEADING + 4, "A pixel art tavern", size=CAPTION, color=ctx.theme.muted, monospace=True)

    def _tick(self, dt: float) -> None:
        self._total_elapsed += dt
        self._anim_accum += dt
        if self._anim_accum >= 1.0 / ANIM_FPS:
            self._anim_accum -= 1.0 / ANIM_FPS
            self._anim_frame = 1 - self._anim_frame

        if self._state == "pausing":
            self._pause_timer -= dt
            if self._pause_timer <= 0:
                self._advance_turn()

    def _advance_turn(self) -> None:
        self._turn_idx = (self._turn_idx + 1) % len(self._npcs)
        self._state = "thinking"
        self.emit.info(f"NPC {self._npcs[self._turn_idx].name}: querying AI")
        self.emit.schedule_task(self._do_turn(self._turn_idx))

    async def _do_turn(self, idx: int) -> None:
        npc = self._npcs[idx]
        messages = list(self._conversation) + [
            {"role": "user", "content": "Continue the tavern conversation in character. 1-2 sentences max. Dialogue only."}
        ]
        try:
            resp = await self.emit.ai_query(
                model_tier="low",
                system=npc.system_prompt,
                messages=messages,
            )
            text = (resp.content or "").strip()
            # Replace previous speech — bubble persists until next response replaces it
            npc.speech = text
            self._conversation.append({"role": "assistant", "content": f"{npc.name}: {text}"})
            if len(self._conversation) > 25:
                self._conversation = self._conversation[-25:]
            self._state = "pausing"
            self._pause_timer = TURN_DELAY_SECS
            self.emit.info(f"NPC {npc.name} responded: {resp.tokens_in} in, {resp.tokens_out} out")
        except Exception as e:
            self.emit.error(f"NPC {npc.name} ai_query failed: {e}")
            self._state = "pausing"
            self._pause_timer = TURN_DELAY_SECS

    def _draw_start_prompt(self, ctx: RenderContext, floor_y: float,
                           sprite_w: float, sprite_h: float) -> None:
        for npc in self._npcs:
            self._draw_sprite(ctx, npc, floor_y, sprite_w, sprite_h, frame=0, active=False)

        # Prompt overlay
        pw, ph = 280.0, 70.0
        px = (ctx.w - pw) / 2
        py = floor_y - 100.0
        ctx.rect(px, py, pw, ph, fill="#000000aa", radius=8.0)
        ctx.text(ctx.w / 2, py + 14, "Press Space to start", size=BODY, color=W,
                 align="center_top", monospace=True, bold=False, elide=False, selectable=False)
        ctx.text(ctx.w / 2, py + 14 + BODY + 8, "NPCs will converse via AI",
                 size=CAPTION, color=ctx.theme.muted,
                 align="center_top", monospace=True, bold=False, elide=False, selectable=False)

    def _draw_scene(self, ctx: RenderContext, floor_y: float,
                    sprite_w: float, sprite_h: float) -> None:
        active_idx = self._turn_idx if self._state in ("thinking", "pausing") else -1

        for i, npc in enumerate(self._npcs):
            is_active = (i == active_idx)
            frame = self._anim_frame if not is_active else 0
            self._draw_sprite(ctx, npc, floor_y, sprite_w, sprite_h, frame=frame, active=is_active)
            self._draw_name_tag(ctx, npc, floor_y, sprite_h)

            # Thinking dots take precedence over persistent bubbles for the active NPC
            if is_active and self._state == "thinking":
                self._draw_thinking_dots(ctx, npc, floor_y)
            elif npc.speech:
                self._draw_speech_bubble(ctx, npc, floor_y)

    def _draw_sprite(self, ctx: RenderContext, npc: NPC, floor_y: float,
                     sprite_w: float, sprite_h: float, frame: int, active: bool) -> None:
        frames = npc.frames
        f = frame % len(frames)
        grid = frames[f]
        sx = npc.cx - sprite_w / 2
        sy = floor_y

        if active:
            glow_pad = 4.0
            ctx.rect(sx - glow_pad, sy - glow_pad,
                     sprite_w + glow_pad * 2, sprite_h + glow_pad * 2,
                     fill=npc.color + "33", radius=4.0)

        for row_i, row in enumerate(grid):
            for col_i, color in enumerate(row):
                if color is None:
                    continue
                ctx.rect(
                    sx + col_i * PIXEL,
                    sy + row_i * PIXEL,
                    PIXEL, PIXEL,
                    fill=color,
                )

    def _draw_name_tag(self, ctx: RenderContext, npc: NPC,
                       floor_y: float, sprite_h: float) -> None:
        label = f"{npc.name}"
        tag_y = floor_y + sprite_h + 6.0
        ctx.text(npc.cx, tag_y, label, size=CAPTION, color=npc.color,
                 align="center_top", bold=True, monospace=True, elide=False, selectable=False)

    def _draw_speech_bubble(self, ctx: RenderContext, npc: NPC, floor_y: float) -> None:
        max_w = 240.0
        bubble_h = 120.0   # tall enough for 3-4 wrapped lines
        bx = npc.cx - max_w / 2
        by = floor_y - bubble_h - 20.0

        # Border (aged wood gold)
        ctx.rect(bx - 2, by - 2, max_w + 4, bubble_h + 4, fill=BUBBLE_GOLD, radius=10.0)
        # Parchment fill
        ctx.rect(bx, by, max_w, bubble_h, fill=BUBBLE_BG, radius=8.0)
        ctx.rect(bx + 2, by + 2, max_w - 4, bubble_h - 4, fill=BUBBLE_FILL, radius=6.0)

        # Tail pointing down to sprite
        tail_x = npc.cx
        ctx.rect(tail_x - 5, by + bubble_h - 1, 10, 10, fill=BUBBLE_FILL)
        ctx.rect(tail_x - 3, by + bubble_h + 8, 6, 5, fill=BUBBLE_GOLD)

        # Name header (dark ink on parchment)
        ctx.text(bx + 10, by + 7, npc.name, size=CAPTION, color=BUBBLE_BG,
                 bold=True, monospace=True, elide=False, selectable=False)

        # Divider line
        ctx.rect(bx + 10, by + 7 + CAPTION + 4, max_w - 20, 1.0, fill=BUBBLE_GOLD + "88")

        # Dialogue text — word-wrapped within bubble bounds, dark ink on parchment
        ctx.markdown(bx + 10, by + 7 + CAPTION + 8, max_w - 20, npc.speech,
                     base_size=CAPTION, color="#2a1505")

    def _draw_thinking_dots(self, ctx: RenderContext, npc: NPC, floor_y: float) -> None:
        bx = npc.cx - 30.0
        by = floor_y - 40.0
        ctx.rect(bx - 2, by - 2, 64.0, 30.0, fill=BUBBLE_GOLD, radius=8.0)
        ctx.rect(bx, by, 60.0, 26.0, fill=BUBBLE_FILL, radius=6.0)
        frame_offset = int(self._total_elapsed * ANIM_FPS) % 3
        for j in range(3):
            col = BUBBLE_BG if j == frame_offset else BUBBLE_GOLD
            ctx.rect(bx + 12 + j * 14, by + 9, 8.0, 8.0, fill=col, radius=4.0)

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if key in ("space", "return") and not self._started:
            self._started = True
            self._state = "thinking"
            self.emit.info("tavern conversation started")
            self.emit.schedule_task(self._do_turn(0))


TavernApp().run()
