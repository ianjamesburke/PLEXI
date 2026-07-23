"""Binary-safe file I/O over the JSON bridge (stint 0509).

`FileWrite` crosses the bridge as base64 (`content_b64`) and `file_read_result`
comes back the same way, so arbitrary bytes — WAV, PNG, NULs, invalid UTF-8 —
round-trip exactly. These tests drive the real v3 runtime subprocess with the
same protocol the host speaks.
"""

import base64
import json
import os
import subprocess
import sys
import textwrap
import time
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[3]
SDK_PATH = str(REPO_ROOT / "sdk" / "python")

# Every byte value, several times over, including NULs and invalid UTF-8.
BINARY_PAYLOAD = bytes(range(256)) * 4


def _make_env():
    env = dict(os.environ)
    env["PYTHONPATH"] = SDK_PATH
    return env


def _spawn(app_source: str, tmp_path: Path):
    app_file = tmp_path / "file_io_app.py"
    app_file.write_text(textwrap.dedent(app_source))
    return subprocess.Popen(
        [sys.executable, "-m", "plexi_sdk._v3_process", str(app_file)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=_make_env(),
    )


def _send(proc, msg: dict) -> None:
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()


def _collect_until(proc, target_type: str, timeout: float = 5.0) -> list[dict]:
    seen = []
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        line = proc.stdout.readline()
        if not line:
            break
        ev = json.loads(line)
        seen.append(ev)
        if ev.get("type") == target_type:
            return seen
    return seen


def _init(proc, capabilities=None) -> list[dict]:
    _send(proc, {
        "type": "init",
        "app_id": "file-io-test",
        "workspace_root": "/tmp",
        "capabilities": capabilities or ["fs.read", "fs.write"],
        "feature_flags": [],
        "width": 640.0,
        "height": 360.0,
        "protocol": "pgap/3",
    })
    return _collect_until(proc, "ready")


WRITER_APP = """
    from plexi_sdk import effects

    PAYLOAD = bytes(range(256)) * 4

    def init(size, args):
        return [effects.write_bytes("out.bin", PAYLOAD)]

    def update(event):
        return []

    def view():
        return {"type": "text", "text": "writer"}
"""

READER_APP = """
    from plexi_sdk import effects, events, state

    def init(size, args):
        return [effects.read_bytes("in.bin")]

    def update(event):
        if isinstance(event, events.FileReadResult):
            digest = "none" if event.content is None else event.content.hex()
            return [effects.SetState({"read_hex": digest})]
        return []

    def view():
        return {"type": "text", "text": state.get("read_hex", "waiting")}
"""


def _render(proc, frame_id=1) -> list[dict]:
    _send(proc, {
        "type": "render",
        "frame_id": frame_id,
        "rect": {"x": 0.0, "y": 0.0, "w": 640.0, "h": 360.0},
    })
    return _collect_until(proc, "frame_done")


def test_file_write_emits_binary_exact_base64(tmp_path):
    proc = _spawn(WRITER_APP, tmp_path)
    try:
        events = _init(proc) + _render(proc)
        writes = [e for e in events if e.get("type") == "file_write"]
        assert writes, f"no file_write emitted; got {[e.get('type') for e in events]}"
        msg = writes[0]
        assert msg["path"] == "out.bin"
        assert "content" not in msg, "binary write must not use the text field"
        assert base64.b64decode(msg["content_b64"]) == BINARY_PAYLOAD
    finally:
        proc.kill()


def test_file_read_result_b64_decodes_to_exact_bytes(tmp_path):
    proc = _spawn(READER_APP, tmp_path)
    try:
        events = _init(proc) + _render(proc)
        reads = [e for e in events if e.get("type") == "file_read"]
        assert reads and reads[0]["path"] == "in.bin"

        _send(proc, {
            "type": "file_read_result",
            "content_b64": base64.b64encode(BINARY_PAYLOAD).decode("ascii"),
        })
        # The dispatched FileReadResult stores the hex digest; the next render's
        # tree proves the exact bytes arrived in the app.
        seen = _render(proc, frame_id=2)
        tree_text = json.dumps(seen)
        assert BINARY_PAYLOAD.hex() in tree_text, (
            f"read bytes did not round-trip; events: {[e.get('type') for e in seen]}"
        )
    finally:
        proc.kill()


def test_write_bytes_rejects_str_and_oversize():
    sys.path.insert(0, SDK_PATH)
    from plexi_sdk import effects

    with pytest.raises(TypeError, match="encode text"):
        effects.write_bytes("out.txt", "not bytes")  # type: ignore[arg-type]
    with pytest.raises(ValueError, match=str(effects.MAX_FILE_IO_BYTES)):
        effects.write_bytes("big.bin", b"\0" * (effects.MAX_FILE_IO_BYTES + 1))


def test_read_bytes_is_a_plain_file_read_effect():
    sys.path.insert(0, SDK_PATH)
    from plexi_sdk import effects

    effect = effects.read_bytes("clip.wav")
    assert isinstance(effect, effects.FileRead)
    assert effect.path == "clip.wav"
