#!/usr/bin/env python3
"""Live checks for PR #2688 at bbbc770e on an installed plexi-beta host.

Check 5 (personal sign-off / Touch ID) is not run. Expiry is not a live
pass/fail: on this head every production filer sets expires_at to null.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

HOME = Path.home()
BIN = Path(os.environ.get("PLEXI_BIN", HOME / ".local/bin/plexi-beta"))
REPO = Path(os.environ.get("PLEXI_REPO", "/workspace"))
ROOT = Path("/tmp/verify-pr-2688")
EVID = ROOT / "evidence"
PROFILE = HOME / ".plexi-beta"
AUDIT = PROFILE / "permission-audit.jsonl"
LOG = PROFILE / "plexi.log"
WS = ROOT / "ws"
PHONE_TOKEN = "verify-pr-2688-token"
PHONE_PORT = 8787
MOCK_PORT = 8765

RESULTS: list[dict] = []


def note(text: str) -> None:
    print(text, flush=True)
    with (EVID / "notes.txt").open("a", encoding="utf-8") as handle:
        handle.write(text + "\n")


def record(name: str, ok: bool, command: str, detail: str, status: str | None = None) -> None:
    label = status or ("PASS" if ok else "FAIL")
    RESULTS.append({"check": name, "result": label, "command": command, "detail": detail})
    note(f"{label} {name}")
    note(f"  command: {command}")
    note(detail.rstrip())
    note("")


def run(args: list[str], timeout: float = 60, env: dict | None = None) -> subprocess.CompletedProcess:
    note(f"$ {' '.join(args)}")
    return subprocess.run(
        args,
        capture_output=True,
        text=True,
        timeout=timeout,
        env=env or base_env(),
        check=False,
    )


def base_env() -> dict:
    env = os.environ.copy()
    for key in list(env):
        if key.startswith("PLEXI_"):
            del env[key]
    env["DISPLAY"] = os.environ.get("DISPLAY", ":99")
    env["VK_DRIVER_FILES"] = os.environ.get(
        "VK_DRIVER_FILES", "/usr/share/vulkan/icd.d/lvp_icd.json"
    )
    env["WGPU_BACKEND"] = "vulkan"
    env["XDG_RUNTIME_DIR"] = os.environ.get("XDG_RUNTIME_DIR", "/tmp/runtime-ubuntu")
    env["PATH"] = f"{HOME / '.local/bin'}:{env.get('PATH', '')}"
    return env


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def both(out: Path, err: Path) -> str:
    parts = []
    if out.exists():
        parts.append(out.read_text(encoding="utf-8", errors="replace"))
    if err.exists():
        parts.append(err.read_text(encoding="utf-8", errors="replace"))
    return "\n".join(parts)


def parse_json_blob(text: str) -> dict | list | None:
    stripped = text.strip()
    for opener in ("[", "{"):
        start = stripped.find(opener)
        if start < 0:
            continue
        try:
            return json.loads(stripped[start:])
        except json.JSONDecodeError:
            continue
    return None


def pane_list() -> list[dict]:
    proc = run([str(BIN), "pane", "list"], timeout=20)
    write(EVID / "logs" / "pane-list.out", proc.stdout)
    write(EVID / "logs" / "pane-list.err", proc.stderr)
    data = parse_json_blob(proc.stdout)
    return data if isinstance(data, list) else []


def in_pane(pane: int, name: str, body: str, timeout: float = 70) -> bool:
    script = EVID / "jobs" / f"{name}.sh"
    done = EVID / "jobs" / f"{name}.done"
    write(
        script,
        "#!/bin/bash\nset -o pipefail\n" + body + f"\necho $? > '{done}'\n",
    )
    script.chmod(0o755)
    done.unlink(missing_ok=True)
    run(
        [str(BIN), "pane", "send", str(pane), "--submit", f"bash '{script}'"],
        timeout=40,
    )
    deadline = time.time() + timeout
    while time.time() < deadline:
        if done.exists():
            return True
        time.sleep(0.5)
    note(f"timeout waiting for in-pane job {name}")
    return False


def needs_list() -> list[dict]:
    proc = run([str(BIN), "needs-you", "list", "--json"], timeout=20)
    write(EVID / "logs" / "needs-list-last.out", proc.stdout)
    write(EVID / "logs" / "needs-list-last.err", proc.stderr)
    data = parse_json_blob(proc.stdout) or {}
    if isinstance(data, dict):
        items = data.get("items") or []
        return items if isinstance(items, list) else []
    return []


def audit_rows() -> list[dict]:
    if not AUDIT.exists():
        return []
    rows = []
    for line in AUDIT.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return rows


def rows_for(item_id: str, rows: list[dict] | None = None) -> list[dict]:
    rows = audit_rows() if rows is None else rows
    return [row for row in rows if row.get("operation_id") == item_id or item_id in json.dumps(row)]


def needs_you_rows(item_id: str, rows: list[dict] | None = None) -> list[dict]:
    rows = audit_rows() if rows is None else rows
    return [
        row
        for row in rows
        if row.get("kind") == "needs_you" and row.get("operation_id") == item_id
    ]


def grant_rows_for(item_id: str, rows: list[dict] | None = None) -> list[dict]:
    rows = audit_rows() if rows is None else rows
    grant_ids = {
        row.get("grant_id")
        for row in needs_you_rows(item_id, rows)
        if row.get("grant_id")
    }
    return [
        row
        for row in rows
        if row.get("kind") == "grant" and row.get("grant_id") in grant_ids
    ]


def find_items(items: list[dict], *, kind: str | None = None, summary: str | None = None, actor_prefix: str | None = None, run_tag: str | None = None) -> list[dict]:
    found = []
    for item in items:
        if kind and item.get("kind") != kind:
            continue
        if summary and summary not in str(item.get("summary", "")):
            continue
        if actor_prefix and not str(item.get("actor", "")).startswith(actor_prefix):
            continue
        if run_tag and item.get("run_tag") != run_tag:
            continue
        found.append(item)
    return found


def mcp_call(port: str, token: str, name: str, arguments: dict, rpc_id: int) -> dict:
    body = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": rpc_id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }
    ).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/mcp",
        data=body,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Connection": "close",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=40) as resp:
            raw = resp.read().decode()
            status = resp.status
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode()
        status = exc.code
    write(EVID / "logs" / f"mcp-{rpc_id}.json", raw)
    parsed = parse_json_blob(raw) or {"raw": raw, "http": status}
    if isinstance(parsed, dict):
        parsed["_http"] = status
    return parsed if isinstance(parsed, dict) else {"raw": raw}


def phone_resolve(item_id: str, decision: str) -> tuple[int, dict]:
    body = json.dumps({"decision": decision}).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{PHONE_PORT}/api/needs-you/{item_id}/resolve",
        data=body,
        headers={
            "Authorization": f"Bearer {PHONE_TOKEN}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=25) as resp:
            raw = resp.read().decode()
            status = resp.status
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode()
        status = exc.code
    parsed = parse_json_blob(raw) or {"raw": raw}
    return status, parsed if isinstance(parsed, dict) else {"raw": raw}


def cli_resolve(item_id: str, approve: bool) -> tuple[int, dict]:
    flag = "--approve" if approve else "--deny"
    proc = run([str(BIN), "needs-you", "resolve", item_id, flag], timeout=20)
    parsed = parse_json_blob(proc.stdout) or {"stdout": proc.stdout, "stderr": proc.stderr}
    return proc.returncode, parsed if isinstance(parsed, dict) else {"stdout": proc.stdout}


def play(rev: int, op: str, move: str, game: str = "game-1") -> dict:
    return {
        "game_id": game,
        "expected_revision": rev,
        "operation_id": op,
        "move": move,
    }


def app_call_body(input_json: dict, dest: Path) -> str:
    payload = json.dumps(input_json)
    quoted = payload.replace("'", "'\\''")
    return (
        f"'{BIN}' app call chess chess.play --json --input '{quoted}' "
        f"> '{dest}' 2>&1"
    )


def strip_config() -> None:
    path = PROFILE / "config.toml"
    text = path.read_text(encoding="utf-8") if path.exists() else ""
    marker = "# verify-pr-2686 posture allow"
    if marker in text:
        text = text.split(marker)[0].rstrip() + "\n"
    kept = []
    skip = False
    for line in text.splitlines(True):
        if line.strip() == "[permissions.personal_signoff]":
            skip = True
            continue
        if skip:
            if line.startswith("[") or line.startswith("# "):
                skip = False
            else:
                continue
        kept.append(line)
    text = "".join(kept)
    if 'backend = "local"' not in text:
        text = text.replace('backend = "openrouter"', 'backend = "local"', 1)
    live = "\n# verify-pr-2688 live config\n"
    if live not in text and "[ai.local]" not in text.split("# [ai.local]")[-1]:
        text += f"""{live}
[ai.local]
base_url = "http://127.0.0.1:{MOCK_PORT}"
model_low = "mock-chess"
model_medium = "mock-chess"
model_high = "mock-chess"

[log]
level = "info"
"""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    write(EVID / "logs" / "config.toml", text)


def main() -> int:
    if EVID.exists():
        shutil.rmtree(EVID)
    (EVID / "logs").mkdir(parents=True)
    (EVID / "jobs").mkdir()
    note(f"commit {subprocess.check_output(['git','-C',str(REPO),'rev-parse','HEAD'], text=True).strip()}")
    note(f"binary {BIN}")
    if not BIN.is_file():
        record("preflight", False, f"test -x {BIN}", "installed binary missing")
        return 1
    ver = run([str(BIN), "--version"], timeout=15)
    note(f"version: {ver.stdout.strip()} {ver.stderr.strip()}")

    strip_config()
    write(
        ROOT / "move.json",
        json.dumps(
            {
                "tool_substr": "chess_play",
                "arguments": play(0, "op-assistant", "a2a3"),
            }
        ),
    )
    mock_log = open(EVID / "logs" / "mock-model.log", "w", encoding="utf-8")
    mock = subprocess.Popen(
        ["python3", str(ROOT / "mock_model.py")],
        stdout=mock_log,
        stderr=subprocess.STDOUT,
        env={**base_env(), "MOCK_CONTROL": str(ROOT / "move.json"), "MOCK_PORT": str(MOCK_PORT)},
    )
    phone_log = open(EVID / "logs" / "phone.log", "w", encoding="utf-8")
    phone_env = base_env()
    phone_env["PLEXI_PHONE_TOKEN"] = PHONE_TOKEN
    phone = subprocess.Popen(
        [
            "python3",
            str(REPO / "clients/phone-web/server.py"),
            "--backend",
            "host",
            "--plexi-bin",
            str(BIN),
            "--port",
            str(PHONE_PORT),
        ],
        stdout=phone_log,
        stderr=subprocess.STDOUT,
        env=phone_env,
        cwd=str(REPO),
    )
    time.sleep(0.4)

    run([str(BIN), "host", "stop"], timeout=20)
    if WS.exists():
        shutil.rmtree(WS)
    WS.mkdir(parents=True)
    run([str(BIN), "workspace", "init"], timeout=30, env={**base_env(), "PWD": str(WS)})
    # workspace init uses cwd
    subprocess.run(
        [str(BIN), "workspace", "init"],
        cwd=str(WS),
        capture_output=True,
        text=True,
        env=base_env(),
        check=False,
    )
    start = run(
        [str(BIN), "host", "start", "--ephemeral", "--timeout-secs", "90", "--pane", f"cwd={WS}"],
        timeout=120,
    )
    write(EVID / "logs" / "host-start.out", start.stdout)
    write(EVID / "logs" / "host-start.err", start.stderr)
    status = run([str(BIN), "host", "status", "--json"], timeout=20)
    write(EVID / "logs" / "host-status.json", status.stdout)
    ready = '"ready":true' in status.stdout.replace(" ", "") or '"ready": true' in status.stdout
    if start.returncode != 0 or not ready:
        record(
            "boot",
            False,
            f"{BIN} host start --ephemeral --timeout-secs 90 --pane cwd={WS}",
            f"exit {start.returncode}\n{start.stderr[-1500:]}\n{status.stdout}\n{status.stderr}",
        )
        mock.kill()
        phone.kill()
        return 1
    record("boot", True, f"{BIN} host start --ephemeral --timeout-secs 90", status.stdout.strip())

    install = run([str(BIN), "app", "install", str(REPO / "apps/chess"), "--yes"], timeout=60)
    write(EVID / "logs" / "app-install.out", install.stdout)
    write(EVID / "logs" / "app-install.err", install.stderr)
    if install.returncode != 0:
        record("chess-install", False, "app install chess --yes", both(EVID / "logs" / "app-install.out", EVID / "logs" / "app-install.err")[-2000:])
        return 1

    before = {row.get("id") for row in pane_list()}
    run([str(BIN), "pane", "new", "-n", "mcp-env", "--cwd", str(WS)], timeout=30)
    pane = 0
    token = ""
    port = ""
    for _ in range(20):
        rows = pane_list()
        fresh = [row for row in rows if row.get("title") == "mcp-env"]
        if not fresh:
            fresh = [row for row in rows if row.get("id") not in before and row.get("type") == "terminal"]
        if fresh:
            pane = int(fresh[-1]["id"])
            token_path = EVID / "jobs" / "mcp-token"
            port_path = EVID / "jobs" / "mcp-port"
            if in_pane(
                pane,
                "mcp-env",
                f"printenv PLEXI_HOST_MCP_TOKEN > '{token_path}'\nprintenv PLEXI_HOST_MCP_PORT > '{port_path}'\n",
                timeout=25,
            ):
                token = token_path.read_text(encoding="utf-8").strip() if token_path.exists() else ""
                port = port_path.read_text(encoding="utf-8").strip() if port_path.exists() else ""
                if token and port:
                    break
        time.sleep(1)
    note(f"terminal pane {pane} mcp port {port} token_len {len(token)}")
    if not pane or not token:
        record("mcp-env", False, "pane new -n mcp-env; printenv PLEXI_HOST_MCP_TOKEN", "MCP discovery env was empty")
        return 1

    opened = False
    for attempt in range(1, 8):
        if not in_pane(pane, f"open-chess-{attempt}", f"'{BIN}' app open chess", timeout=30):
            continue
        time.sleep(2)
        dest = EVID / "jobs" / f"ready-{attempt}.out"
        if not in_pane(pane, f"ready-{attempt}", f"'{BIN}' app call chess chess.state --json > '{dest}' 2>&1", timeout=40):
            continue
        text = dest.read_text(encoding="utf-8", errors="replace") if dest.exists() else ""
        if text and "tool_not_found" not in text:
            opened = True
            write(EVID / "logs" / "chess-state.txt", text)
            break
        time.sleep(2)
    if not opened:
        record("chess-ready", False, "in-pane app call chess chess.state", "chess.play never registered")
        return 1

    cli_input = play(0, "op-cli", "e2e4")
    mcp_input = play(1, "op-mcp", "e7e5")
    cli_out = EVID / "jobs" / "cli-call.out"
    setup = in_pane(
        pane,
        "callers",
        "\n".join(
            [
                app_call_body(cli_input, cli_out),
                f"'{BIN}' agent report --state blocked --agent verifier --detail 'need a decision' --event AskQuestion --session-id q-verify-2688 > '{EVID}/jobs/question.out' 2>&1",
                f"'{BIN}' agent report --state blocked --agent verifier --detail 'run is stuck' --session-id blocked-verify-2688 > '{EVID}/jobs/blocked.out' 2>&1",
                f"'{BIN}' app open assistant > '{EVID}/jobs/open-assistant.out' 2>&1",
            ]
        ),
        timeout=90,
    )
    mcp_reply = mcp_call(port, token, "chess__chess.play", mcp_input, 1)
    write(EVID / "logs" / "mcp-first.json", json.dumps(mcp_reply, indent=2))

    assistant_proc = None
    assistant_pane = ""
    for _ in range(30):
        rows = pane_list()
        hits = [
            row
            for row in rows
            if "assistant" in str(row.get("title", "")).lower()
            or str(row.get("app_id", "")) == "assistant"
            or str(row.get("manifest_id", "")) == "assistant"
        ]
        if hits:
            assistant_pane = str(hits[-1]["id"])
            break
        time.sleep(1)
    if assistant_pane:
        assistant_log = open(EVID / "logs" / "assistant-send.out", "w", encoding="utf-8")
        assistant_proc = subprocess.Popen(
            [
                str(BIN),
                "assistant",
                "send",
                "--text",
                "Play the chess move from the tool list.",
                "--pane-id",
                assistant_pane,
                "--json",
            ],
            stdout=assistant_log,
            stderr=subprocess.STDOUT,
            env=base_env(),
        )

    items: list[dict] = []
    deadline = time.time() + 50
    while time.time() < deadline:
        items = needs_list()
        kinds = {item.get("kind") for item in items}
        summaries = " ".join(str(item.get("summary", "")) for item in items)
        if (
            "op-cli" in summaries
            and "op-mcp" in summaries
            and "op-assistant" in summaries
            and "question" in kinds
            and "blocked_run" in kinds
        ):
            break
        time.sleep(1)
    write(EVID / "logs" / "list-check1.json", json.dumps(items, indent=2))
    cli_items = find_items(items, kind="approval_click", summary="op-cli", actor_prefix="pane:")
    mcp_items = find_items(items, kind="approval_click", summary="op-mcp", actor_prefix="mcp:")
    asst_items = find_items(items, kind="approval_click", summary="op-assistant")
    questions = find_items(items, kind="question", run_tag="q-verify-2688")
    blocked = find_items(items, kind="blocked_run", run_tag="blocked-verify-2688")
    check1_ok = bool(cli_items and mcp_items and asst_items and questions and blocked)
    detail1 = json.dumps(
        {
            "setup_done": setup,
            "cli_call": both(cli_out, Path("/dev/null"))[:800],
            "mcp": {k: mcp_reply.get(k) for k in ("result", "error", "_http") if k in mcp_reply},
            "assistant_pane": assistant_pane,
            "counts": {
                "cli": len(cli_items),
                "mcp": len(mcp_items),
                "assistant": len(asst_items),
                "question": len(questions),
                "blocked_run": len(blocked),
                "open": len(items),
            },
            "items": [
                {k: item.get(k) for k in ("id", "kind", "actor", "resource", "run_tag", "expires_at", "summary")}
                for item in items
            ],
        },
        indent=2,
    )
    record(
        "1-list",
        check1_ok,
        f"{BIN} needs-you list --json",
        detail1,
    )
    if not check1_ok:
        note("continuing with whichever items were filed")

    # Check 6 while the list is still the check-1 set.
    shot = EVID / "badge.png"
    shot_proc = run([str(BIN), "host", "screenshot", "--output", str(shot)], timeout=30)
    write(EVID / "logs" / "screenshot.out", shot_proc.stdout + shot_proc.stderr)
    state_txt = ""
    if pane:
        state = run([str(BIN), "pane", "state", str(pane)], timeout=20)
        write(EVID / "logs" / "pane-state.json", state.stdout)
        state_txt = state.stdout[:1500]
    listed = needs_list()
    write(EVID / "logs" / "list-check6.json", json.dumps(listed, indent=2))
    write(EVID / "logs" / "badge-expected.txt", str(len(listed)))
    record(
        "6-badge",
        shot.exists() and shot.stat().st_size > 1000 and len(listed) == len(items) and len(listed) > 0,
        f"{BIN} host screenshot --output {shot}",
        f"list count {len(listed)} screenshot bytes {shot.stat().st_size if shot.exists() else 0}\n"
        f"pane state has badge text: {'Needs you' in state_txt}\n{state_txt[:500]}",
    )

    if not cli_items or not mcp_items:
        record("2-cli-approve", False, "needs-you resolve", "missing cli or mcp item")
        record("3-phone", False, "POST /api/needs-you", "missing mcp item")
    else:
        cli_id = cli_items[0]["id"]
        mcp_id = mcp_items[0]["id"]
        others = [item["id"] for item in items if item["id"] not in (cli_id, mcp_id)]
        code, receipt = cli_resolve(cli_id, True)
        after = needs_list()
        after_ids = {item["id"] for item in after}
        write(EVID / "logs" / "resolve-cli.json", json.dumps({"code": code, "receipt": receipt, "open": list(after_ids)}, indent=2))
        cli_replay_path = EVID / "jobs" / "cli-replay.out"
        in_pane(pane, "cli-replay", app_call_body(cli_input, cli_replay_path), timeout=40)
        cli_replay = cli_replay_path.read_text(encoding="utf-8", errors="replace") if cli_replay_path.exists() else ""
        write(EVID / "logs" / "cli-replay.out", cli_replay)
        # MCP was filed at revision 1, so replay it only after the CLI move commits.
        phone_status, phone_body = phone_resolve(mcp_id, "approve")
        mcp_replay = mcp_call(port, token, "chess__chess.play", mcp_input, 2)
        write(EVID / "logs" / "mcp-replay.json", json.dumps(mcp_replay, indent=2))
        final_open = {item["id"] for item in needs_list()}
        cli_rows = needs_you_rows(cli_id)
        cli_grants = grant_rows_for(cli_id)
        mcp_text = json.dumps(mcp_replay)
        cli_proceeds = "revision_after" in cli_replay and "permission_required" not in cli_replay
        mcp_proceeds = "revision_after" in mcp_text and "permission_required" not in mcp_text
        others_held = all(item_id in after_ids for item_id in others if item_id != mcp_id)
        # MCP is approved in this same stretch; it must still have been open after the CLI resolve.
        mcp_held = mcp_id in after_ids
        record(
            "2-cli-approve",
            code == 0
            and receipt.get("ok") is True
            and receipt.get("already") is False
            and receipt.get("resolution") == "approved"
            and mcp_held
            and others_held
            and cli_proceeds
            and len(cli_rows) == 1
            and len(cli_grants) == 1,
            f"{BIN} needs-you resolve {cli_id} --approve",
            json.dumps(
                {
                    "receipt": receipt,
                    "exit": code,
                    "mcp_still_open": mcp_held,
                    "others_still_open": others_held,
                    "cli_replay": cli_replay[:800],
                    "needs_you_rows": len(cli_rows),
                    "grant_rows": len(cli_grants),
                },
                indent=2,
            ),
        )
        rows_before_repeat = audit_rows()
        ny_before = len(needs_you_rows(mcp_id, rows_before_repeat))
        grants_before = len(grant_rows_for(mcp_id, rows_before_repeat))
        phone_again_status, phone_again = phone_resolve(mcp_id, "approve")
        cli_again_code, cli_again = cli_resolve(mcp_id, True)
        rows_after = audit_rows()
        record(
            "3-phone",
            phone_status == 200
            and phone_body.get("ok") is True
            and mcp_proceeds
            and phone_again_status == 409
            and phone_again.get("already") is True
            and cli_again.get("ok") is False
            and cli_again.get("already") is True
            and cli_again_code != 0
            and len(needs_you_rows(mcp_id, rows_after)) == ny_before
            and len(grant_rows_for(mcp_id, rows_after)) == grants_before
            and ny_before == 1
            and grants_before == 1,
            f"POST /api/needs-you/{mcp_id}/resolve {{\"decision\":\"approve\"}}",
            json.dumps(
                {
                    "first": {"status": phone_status, "body": phone_body},
                    "replay": mcp_text[:800],
                    "phone_repeat": {"status": phone_again_status, "body": phone_again},
                    "cli_repeat": {"exit": cli_again_code, "body": cli_again},
                    "needs_you_rows": [ny_before, len(needs_you_rows(mcp_id, rows_after))],
                    "grant_rows": [grants_before, len(grant_rows_for(mcp_id, rows_after))],
                },
                indent=2,
            ),
        )

    # Check 4 deny. Click items have no deadline on this head.
    deny_input = play(2, "op-deny", "d2d4")
    deny_out = EVID / "jobs" / "deny-call.out"
    in_pane(pane, "deny-call", app_call_body(deny_input, deny_out), timeout=40)
    deny_items = find_items(needs_list(), summary="op-deny", kind="approval_click")
    if not deny_items:
        record("4-deny", False, "app call op-deny", both(deny_out, Path("/dev/null"))[:800])
    else:
        deny_id = deny_items[0]["id"]
        expires = deny_items[0].get("expires_at")
        code, receipt = cli_resolve(deny_id, False)
        deny_replay = EVID / "jobs" / "deny-replay.out"
        in_pane(pane, "deny-replay", app_call_body(deny_input, deny_replay), timeout=40)
        replay_text = deny_replay.read_text(encoding="utf-8", errors="replace") if deny_replay.exists() else ""
        state_out = EVID / "jobs" / "state-after-deny.out"
        in_pane(pane, "state-after-deny", f"'{BIN}' app call chess chess.state --json > '{state_out}' 2>&1", timeout=30)
        state_text = state_out.read_text(encoding="utf-8", errors="replace") if state_out.exists() else ""
        denied_rows = [row for row in needs_you_rows(deny_id) if row.get("decision") == "denied"]
        record(
            "4-deny",
            code == 0
            and receipt.get("ok") is True
            and receipt.get("resolution") == "denied"
            and "revision_after" not in replay_text
            and len(denied_rows) == 1
            and expires is None,
            f"{BIN} needs-you resolve {deny_id} --deny",
            json.dumps(
                {
                    "receipt": receipt,
                    "expires_at": expires,
                    "replay": replay_text[:600],
                    "state": state_text[:400],
                    "denied_audit_rows": len(denied_rows),
                },
                indent=2,
            ),
        )
        record(
            "4-expiry",
            True,
            "n/a on bbbc770e",
            "DROPPED. Click approvals, questions, and blocked runs are filed with expires_at null. "
            "The only production deadline was the personal sign-off challenge, which is not on this head. "
            f"The denied click item expires_at={expires}.",
            status="DROPPED",
        )

    # Check 7. Twenty pending click items, half CLI and half phone, each once.
    load_ids: list[str] = []
    calls = []
    for index in range(1, 21):
        op = f"op-load-{index:02d}"
        dest = EVID / "jobs" / f"load-{index:02d}.out"
        calls.append(app_call_body(play(0, op, "e2e4", f"load-{index}"), dest) + " &")
    calls.append("wait")
    in_pane(pane, "load", "\n".join(calls), timeout=90)
    load_items = find_items(needs_list(), summary="op-load-", kind="approval_click")
    load_items.sort(key=lambda item: str(item.get("summary")))
    write(EVID / "logs" / "list-load.json", json.dumps(load_items, indent=2))
    if len(load_items) != 20:
        record(
            "7-load",
            False,
            "20 in-pane app calls",
            f"filed {len(load_items)} load items\n" + "\n".join(
                (EVID / "jobs" / f"load-{i:02d}.out").read_text(encoding="utf-8", errors="replace")[:200]
                for i in range(1, 21)
                if (EVID / "jobs" / f"load-{i:02d}.out").exists()
            )[:2000],
        )
    else:
        half = load_items[:10]
        rest = load_items[10:]
        before_n = len(audit_rows())

        def settle_cli(item: dict) -> dict:
            code, body = cli_resolve(item["id"], True)
            return {"id": item["id"], "via": "cli", "code": code, "body": body}

        def settle_phone(item: dict) -> dict:
            status, body = phone_resolve(item["id"], "approve")
            return {"id": item["id"], "via": "phone", "status": status, "body": body}

        with ThreadPoolExecutor(max_workers=20) as pool:
            futures = [pool.submit(settle_cli, item) for item in half]
            futures += [pool.submit(settle_phone, item) for item in rest]
            first = [future.result() for future in as_completed(futures)]
        with ThreadPoolExecutor(max_workers=20) as pool:
            futures = [pool.submit(settle_cli, item) for item in half]
            futures += [pool.submit(settle_phone, item) for item in rest]
            second = [future.result() for future in as_completed(futures)]
        rows = audit_rows()
        problems = []
        for item in load_items:
            ny = needs_you_rows(item["id"], rows)
            grants = grant_rows_for(item["id"], rows)
            if len(ny) != 1 or len(grants) != 1:
                problems.append({"id": item["id"], "needs_you": len(ny), "grants": len(grants)})
        still_open = find_items(needs_list(), summary="op-load-")
        first_ok = all(
            (row["via"] == "cli" and row["body"].get("ok") is True)
            or (row["via"] == "phone" and row.get("status") == 200 and row["body"].get("ok") is True)
            for row in first
        )
        second_ok = all(
            (row["via"] == "cli" and row["body"].get("ok") is False and row["body"].get("already") is True)
            or (row["via"] == "phone" and row.get("status") == 409 and row["body"].get("already") is True)
            for row in second
        )
        write(EVID / "logs" / "load-first.json", json.dumps(first, indent=2))
        write(EVID / "logs" / "load-second.json", json.dumps(second, indent=2))
        record(
            "7-load",
            first_ok and second_ok and not problems and not still_open,
            "10x needs-you resolve --approve and 10x POST /api/needs-you/<id>/resolve concurrently",
            json.dumps(
                {
                    "first_ok": first_ok,
                    "second_ok": second_ok,
                    "audit_problems": problems,
                    "still_open": [item["id"] for item in still_open],
                    "audit_rows_added": len(rows) - before_n,
                },
                indent=2,
            ),
        )

    write(EVID / "results.json", json.dumps(RESULTS, indent=2))
    if assistant_proc and assistant_proc.poll() is None:
        assistant_proc.kill()
    mock.kill()
    phone.kill()
    run([str(BIN), "host", "stop"], timeout=20)
    failed = [row["check"] for row in RESULTS if row["result"] == "FAIL"]
    note("FAILED " + ", ".join(failed) if failed else "no product check failed")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
