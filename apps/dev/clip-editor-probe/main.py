#!/usr/bin/env python3
"""Clip Editor Probe — deterministic editor-like fixture for Pi/agent orchestration.

Exposes:
- ``clip.state`` (read_only=True): inspect clips, revisions, and applied operations.
- ``clip.crop`` (read_only=False): crop a specific clip with ``expected_revision`` and ``operation_id``.
- ``clip.move`` (read_only=False): move a specific clip with ``expected_revision`` and ``operation_id``.
- ``clip.updated`` event stream: emitted whenever a clip is mutated.
"""

from __future__ import annotations

import json
from typing import Any

from plexi_sdk import effects, events, log, state, tools

STREAM = "clip.updated"

DEFAULT_CLIPS: dict[str, dict[str, Any]] = {
    "intro": {
        "clip_id": "intro",
        "title": "Intro Slate",
        "start_ms": 0,
        "end_ms": 3000,
        "crop_preset": "16:9",
        "revision": "rev-1",
    },
    "main": {
        "clip_id": "main",
        "title": "Main Interview",
        "start_ms": 3000,
        "end_ms": 12000,
        "crop_preset": "16:9",
        "revision": "rev-1",
    },
    "outro": {
        "clip_id": "outro",
        "title": "Outro Bumper",
        "start_ms": 12000,
        "end_ms": 15000,
        "crop_preset": "16:9",
        "revision": "rev-1",
    },
}


def _clips() -> dict[str, dict[str, Any]]:
    raw = state.get("clips")
    if isinstance(raw, dict) and raw:
        return json.loads(json.dumps(raw))
    return json.loads(json.dumps(DEFAULT_CLIPS))


def _ops() -> dict[str, dict[str, Any]]:
    raw = state.get("operations")
    if isinstance(raw, dict):
        return json.loads(json.dumps(raw))
    return {}


def _next_revision(rev: str) -> str:
    if rev.startswith("rev-"):
        try:
            num = int(rev.split("-", 1)[1])
            return f"rev-{num + 1}"
        except ValueError:
            pass
    return f"{rev}-next"


@tools.tool(
    "clip.state",
    "Read the current clip timeline state, clip revisions, and operation receipts.",
    {"clip_id": str},
    read_only=True,
)
def _tool_state(clip_id: str = "") -> dict[str, Any]:
    clips = _clips()
    ops = _ops()
    selected = str(state.get("selected_clip", "intro") or "intro")
    target = clip_id.strip() if clip_id else ""
    if target:
        clip = clips.get(target)
        return {
            "ok": clip is not None,
            "clip": clip,
            "selected_clip": selected,
            "operations": ops,
        }
    return {
        "ok": True,
        "clips": clips,
        "selected_clip": selected,
        "operations": ops,
    }


@tools.tool(
    "clip.crop",
    "Apply a crop preset or trim range to a clip, checking expected_revision and deduplicating operation_id.",
    {
        "clip_id": str,
        "crop_preset": str,
        "expected_revision": str,
        "operation_id": str,
    },
    read_only=False,
)
def _tool_crop(
    clip_id: str,
    crop_preset: str,
    expected_revision: str = "",
    operation_id: str = "",
) -> tools.Reply:
    clips = _clips()
    ops = _ops()
    cid = clip_id.strip()
    op_id = operation_id.strip()

    if op_id and op_id in ops:
        cached = dict(ops[op_id])
        cached["deduplicated"] = True
        log.info(f"clip-editor-probe: deduplicated crop operation_id={op_id} clip_id={cid}")
        return tools.Reply(cached, [])

    clip = clips.get(cid)
    if clip is None:
        return tools.Reply(
            {
                "ok": False,
                "error_code": "not_found",
                "error": f"unknown clip_id '{cid}'",
            },
            [],
        )

    current_rev = str(clip.get("revision", "rev-1"))
    exp_rev = expected_revision.strip()
    if exp_rev and exp_rev != current_rev:
        log.warn(
            f"clip-editor-probe: stale revision on clip_id={cid} expected={exp_rev} actual={current_rev}"
        )
        return tools.Reply(
            {
                "ok": False,
                "error_code": "stale_revision",
                "error": f"clip '{cid}' is at revision {current_rev}, expected {exp_rev}",
                "clip_id": cid,
                "expected_revision": exp_rev,
                "actual_revision": current_rev,
            },
            [],
        )

    new_rev = _next_revision(current_rev)
    clip["crop_preset"] = crop_preset.strip() or "16:9"
    clip["revision"] = new_rev
    clips[cid] = clip

    receipt = {
        "ok": True,
        "deduplicated": False,
        "operation_id": op_id or f"crop-{cid}-{new_rev}",
        "clip_id": cid,
        "crop_preset": clip["crop_preset"],
        "revision_before": current_rev,
        "revision_after": new_rev,
        "clip": clip,
    }
    if op_id:
        ops[op_id] = receipt

    log.info(
        f"clip-editor-probe: cropped clip_id={cid} preset={clip['crop_preset']} {current_rev}->{new_rev}"
    )
    effs = [
        effects.SetState({"clips": clips, "operations": ops, "selected_clip": cid}),
        effects.EmitEvent(
            event=STREAM,
            actor="agent",
            summary=f"Cropped {cid} to {clip['crop_preset']} ({new_rev})",
            resource_id=cid,
            resource_scope="document",
            revision_before=current_rev,
            revision_after=new_rev,
            payload_json=json.dumps(receipt),
        ),
        effects.SetStatus(f"{cid}: {clip['crop_preset']} ({new_rev})"),
    ]
    return tools.Reply(receipt, effs)


@tools.tool(
    "clip.move",
    "Move a clip's start_ms/end_ms on the timeline, checking expected_revision and deduplicating operation_id.",
    {
        "clip_id": str,
        "start_ms": int,
        "end_ms": int,
        "expected_revision": str,
        "operation_id": str,
    },
    read_only=False,
)
def _tool_move(
    clip_id: str,
    start_ms: int,
    end_ms: int,
    expected_revision: str = "",
    operation_id: str = "",
) -> tools.Reply:
    clips = _clips()
    ops = _ops()
    cid = clip_id.strip()
    op_id = operation_id.strip()

    if op_id and op_id in ops:
        cached = dict(ops[op_id])
        cached["deduplicated"] = True
        return tools.Reply(cached, [])

    clip = clips.get(cid)
    if clip is None:
        return tools.Reply(
            {
                "ok": False,
                "error_code": "not_found",
                "error": f"unknown clip_id '{cid}'",
            },
            [],
        )

    current_rev = str(clip.get("revision", "rev-1"))
    exp_rev = expected_revision.strip()
    if exp_rev and exp_rev != current_rev:
        return tools.Reply(
            {
                "ok": False,
                "error_code": "stale_revision",
                "error": f"clip '{cid}' is at revision {current_rev}, expected {exp_rev}",
                "clip_id": cid,
                "expected_revision": exp_rev,
                "actual_revision": current_rev,
            },
            [],
        )

    new_rev = _next_revision(current_rev)
    clip["start_ms"] = int(start_ms)
    clip["end_ms"] = int(end_ms)
    clip["revision"] = new_rev
    clips[cid] = clip

    receipt = {
        "ok": True,
        "deduplicated": False,
        "operation_id": op_id or f"move-{cid}-{new_rev}",
        "clip_id": cid,
        "start_ms": clip["start_ms"],
        "end_ms": clip["end_ms"],
        "revision_before": current_rev,
        "revision_after": new_rev,
        "clip": clip,
    }
    if op_id:
        ops[op_id] = receipt

    effs = [
        effects.SetState({"clips": clips, "operations": ops, "selected_clip": cid}),
        effects.EmitEvent(
            event=STREAM,
            actor="agent",
            summary=f"Moved {cid} to {start_ms}..{end_ms}ms ({new_rev})",
            resource_id=cid,
            resource_scope="document",
            revision_before=current_rev,
            revision_after=new_rev,
            payload_json=json.dumps(receipt),
        ),
        effects.SetStatus(f"{cid}: {start_ms}..{end_ms}ms ({new_rev})"),
    ]
    return tools.Reply(receipt, effs)


def init(_size, _args) -> list:
    return [
        effects.SetTitle("Clip Editor Probe"),
        effects.SetState(
            {
                "clips": json.loads(json.dumps(DEFAULT_CLIPS)),
                "operations": {},
                "selected_clip": "intro",
            }
        ),
        effects.DeclareEventStreams(
            [
                effects.EventStreamDecl(
                    name=STREAM,
                    schema_json=json.dumps(
                        {
                            "type": "object",
                            "properties": {
                                "clip_id": {"type": "string"},
                                "revision_after": {"type": "string"},
                            },
                            "required": ["clip_id", "revision_after"],
                        }
                    ),
                    description="Fires whenever a clip is cropped or moved.",
                )
            ]
        ),
        tools.expose(),
        effects.SetStatus("3 clips ready"),
    ]


def update(event) -> list:
    dispatched = tools.dispatch(event)
    if dispatched is not None:
        return dispatched
    if isinstance(event, events.UiAction) and event.handler_id.startswith("select:"):
        cid = event.handler_id.split(":", 1)[1]
        return [effects.SetState({"selected_clip": cid})]
    return []


def view():
    clips = _clips()
    selected = str(state.get("selected_clip", "intro") or "intro")
    rows = []
    for cid in ("intro", "main", "outro"):
        clip = clips.get(cid, {})
        marker = "▸" if cid == selected else " "
        rows.append(
            {
                "type": "text",
                "text": (
                    f"{marker} [{cid}] {clip.get('title', cid)} · "
                    f"crop={clip.get('crop_preset', '16:9')} · "
                    f"{clip.get('start_ms', 0)}..{clip.get('end_ms', 0)}ms · "
                    f"{clip.get('revision', 'rev-1')}"
                ),
                "bold": cid == selected,
            }
        )
    return {
        "type": "column",
        "children": [
            {"type": "app_bar", "title": "Clip Editor Probe"},
            *rows,
        ],
    }
