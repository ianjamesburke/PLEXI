"""Stub server contract tests. Run: python3 -m unittest clients/phone-web/test_server.py"""

import json
import subprocess
import sys
import threading
import time
import tempfile
import unittest
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import server  # noqa: E402


def envelope(request_id: str, text: str) -> dict:
    return {"schema_version": 1, "request_id": request_id, "conversation_id": "local-stub",
            "content": [{"type": "text", "text": text}]}


class StubServerTest(unittest.TestCase):
    def setUp(self) -> None:
        self.store = server.StubStore(echo_delay=0.2)
        self.httpd = server.build_server("127.0.0.1", 0, self.store)
        self.base = f"http://127.0.0.1:{self.httpd.server_address[1]}"
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def tearDown(self) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()

    def request(self, path: str, body: dict | None = None) -> tuple[int, dict | str]:
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.base + path, data=data, method="POST" if data is not None or path.endswith("/cancel") else "GET",
                                     headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req) as res:
                raw = res.read().decode()
                status = res.status
        except urllib.error.HTTPError as err:
            raw, status = err.read().decode(), err.code
        try:
            return status, json.loads(raw)
        except json.JSONDecodeError:
            return status, raw

    def test_page_has_composer_and_manifest(self) -> None:
        status, html = self.request("/")
        self.assertEqual(status, 200)
        self.assertIn('<textarea id="message"', html)
        self.assertIn('rel="manifest"', html)
        status, manifest = self.request("/manifest.webmanifest")
        self.assertEqual((status, manifest["display"]), (200, "standalone"))

    def test_send_is_queued_then_echoed(self) -> None:
        status, receipt = self.request("/api/turns", envelope("r1", "hello"))
        self.assertEqual((status, receipt["state"]), (202, "queued"))
        time.sleep(0.5)
        _, page = self.request("/api/conversation?after=0")
        kinds = [(e["kind"], e.get("text") or e.get("state")) for e in page["events"]]
        self.assertEqual(kinds, [("user", "hello"), ("receipt", "queued"),
                                 ("stub_reply", "Stub echo: hello"), ("receipt", "succeeded")])

    def test_cancel_before_echo_wins(self) -> None:
        self.request("/api/turns", envelope("r2", "stop me"))
        status, receipt = self.request("/api/turns/r2/cancel")
        self.assertEqual((status, receipt["state"]), (200, "cancelled"))
        time.sleep(0.5)
        _, page = self.request("/api/conversation?after=0")
        self.assertNotIn("stub_reply", [e["kind"] for e in page["events"]])

    def test_duplicate_request_id(self) -> None:
        self.request("/api/turns", envelope("r3", "once"))
        status, _ = self.request("/api/turns", envelope("r3", "once"))
        self.assertEqual(status, 200)
        status, body = self.request("/api/turns", envelope("r3", "different"))
        self.assertEqual((status, body["error"]), (409, "request_id_reused_with_different_payload"))

    def test_rejects_bad_envelope(self) -> None:
        status, body = self.request("/api/turns", {"schema_version": 2, "request_id": "x", "content": []})
        self.assertEqual((status, body["error"]), (400, "unsupported_schema_version"))

    def test_static_path_traversal_blocked(self) -> None:
        status, _ = self.request("/../server.py")
        self.assertEqual(status, 404)

    def test_token_blocks_api_but_not_page(self) -> None:
        self.httpd.shutdown(); self.httpd.server_close()
        self.httpd = server.build_server("127.0.0.1", 0, self.store, token="test-token")
        self.base = f"http://127.0.0.1:{self.httpd.server_address[1]}"
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        status, _ = self.request("/api/status")
        self.assertEqual(status, 401)
        req = urllib.request.Request(self.base + "/api/status", headers={"Authorization": "Bearer test-token"})
        with urllib.request.urlopen(req) as res:
            self.assertEqual(res.status, 200)

    def test_host_backend_uses_fake_cli(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "plexi"
            fake.write_text("#!/bin/sh\nprintf '%s\\n' '{\"request_id\":\"host-r1\",\"turn_id\":\"turn-host-r1\",\"state\":\"succeeded\",\"reply\":\"real-ish reply\"}'\n")
            fake.chmod(0o755)
            self.httpd.shutdown(); self.httpd.server_close()
            self.httpd = server.build_server("127.0.0.1", 0, server.HostStore(str(fake)), token="test-token")
            self.base = f"http://127.0.0.1:{self.httpd.server_address[1]}"
            threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
            req = urllib.request.Request(self.base + "/api/turns", data=json.dumps(envelope("host-r1", "hello")).encode(), method="POST", headers={"Content-Type":"application/json", "Authorization":"Bearer test-token"})
            with urllib.request.urlopen(req) as res: self.assertEqual(res.status, 202)
            time.sleep(0.2)
            req = urllib.request.Request(self.base + "/api/conversation?after=0", headers={"Authorization":"Bearer test-token"})
            with urllib.request.urlopen(req) as res: page = json.loads(res.read())
            reply = next(event for event in page["events"] if event["kind"] == "assistant_reply")
            self.assertEqual(reply["turn_id"], "turn-host-r1")
            self.assertEqual(reply["request_id"], "host-r1")

    def test_host_backend_passes_one_stable_conversation(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            argv_log = Path(temp) / "argv"
            fake = Path(temp) / "plexi"
            quoted = str(argv_log).replace("'", "'\\''")
            fake.write_text(
                "#!/bin/sh\n"
                f"printf '%s\\n' \"$*\" >> '{quoted}'\n"
                "printf '%s\\n' '{\"turn_id\":\"turn-x\",\"state\":\"succeeded\",\"reply\":\"ok\"}'\n"
            )
            fake.chmod(0o755)
            store = server.HostStore(str(fake))
            self.assertTrue(store.conversation_id.startswith("phone-"))
            store.submit(envelope("a", "one"))
            store.submit(envelope("b", "two"))
            time.sleep(0.3)
            lines = argv_log.read_text().splitlines()
            self.assertEqual(len(lines), 2)
            needle = f"--conversation {store.conversation_id}"
            self.assertTrue(all(needle in line and "--desktop" not in line for line in lines))

    def test_host_backend_appends_error_to_failed_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "plexi"
            fake.write_text("#!/bin/sh\nprintf '%s\\n' '{\"state\":\"failed\",\"error\":\"permission denied\"}'\n")
            fake.chmod(0o755)
            store = server.HostStore(str(fake))
            store.submit(envelope("host-failed", "hello"))
            time.sleep(0.2)
            receipt = store.events[-1]
            self.assertEqual(receipt["state"], "failed")
            self.assertEqual(receipt["error"], "permission denied")

    def test_host_backend_reports_no_reply_when_cli_is_silent(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "plexi"
            fake.write_text("#!/bin/sh\nexit 0\n")
            fake.chmod(0o755)
            store = server.HostStore(str(fake))
            store.submit(envelope("host-silent", "hello"))
            time.sleep(0.2)
            receipt = store.events[-1]
            self.assertEqual(receipt["state"], "failed")
            self.assertEqual(receipt["error"], "no reply from the host Assistant within 120 s (a permission prompt may be waiting on the desktop)")

    def test_host_backend_rejects_reply_without_turn_id(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "plexi"
            fake.write_text("#!/bin/sh\nprintf '%s\\n' '{\"request_id\":\"host-noturn\",\"state\":\"succeeded\",\"reply\":\"orphan\"}'\n")
            fake.chmod(0o755)
            store = server.HostStore(str(fake))
            store.submit(envelope("host-noturn", "hello"))
            time.sleep(0.2)
            receipt = store.events[-1]
            self.assertEqual(receipt["state"], "failed")
            self.assertEqual(receipt["error"], "host reply missing turn_id")
            self.assertNotIn("assistant_reply", [event["kind"] for event in store.events])

    def test_host_backend_rejects_mismatched_request_id(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "plexi"
            fake.write_text("#!/bin/sh\nprintf '%s\\n' '{\"request_id\":\"someone-else\",\"turn_id\":\"turn-x\",\"state\":\"succeeded\",\"reply\":\"wrong turn\"}'\n")
            fake.chmod(0o755)
            store = server.HostStore(str(fake))
            store.submit(envelope("host-mine", "hello"))
            time.sleep(0.2)
            receipt = store.events[-1]
            self.assertEqual(receipt["state"], "failed")
            self.assertIn("someone-else", receipt["error"])
            self.assertNotIn("assistant_reply", [event["kind"] for event in store.events])

    def test_host_backend_surfaces_pending_desktop_approval(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "plexi"
            fake.write_text(
                "#!/bin/sh\nprintf '%s\\n' "
                "'{\"request_id\":\"host-wait\",\"state\":\"waiting_for_permission\","
                "\"status\":\"waiting for approval on desktop\",\"pending_request_id\":\"turn-desk\"}'\n"
            )
            fake.chmod(0o755)
            store = server.HostStore(str(fake))
            store.submit(envelope("host-wait", "hello"))
            time.sleep(0.2)
            receipt = store.events[-1]
            self.assertEqual(receipt["state"], "waiting_for_permission")
            self.assertEqual(receipt["status"], "waiting for approval on desktop (turn-desk)")
            self.assertNotIn("error", receipt)

    def test_host_backend_polls_until_the_desktop_decision_lands(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "plexi"
            fake.write_text(
                "#!/bin/sh\n"
                "for arg in \"$@\"; do\n"
                "  if [ \"$arg\" = \"--status-for\" ]; then\n"
                "    printf '%s\\n' '{\"request_id\":\"host-wait\",\"turn_id\":\"turn-desk\",\"state\":\"succeeded\",\"reply\":\"desktop said yes\"}'\n"
                "    exit 0\n"
                "  fi\n"
                "done\n"
                "printf '%s\\n' '{\"request_id\":\"host-wait\",\"state\":\"waiting_for_permission\","
                "\"status\":\"waiting for approval on desktop\",\"pending_request_id\":\"turn-desk\"}'\n"
            )
            fake.chmod(0o755)
            store = server.HostStore(str(fake))
            store.submit(envelope("host-wait", "hello"))
            deadline = time.time() + 3
            reply = None
            while time.time() < deadline:
                reply = next((event for event in store.events if event.get("kind") == "assistant_reply"), None)
                if reply:
                    break
                time.sleep(0.05)
            self.assertIsNotNone(reply)
            self.assertEqual(reply["text"], "desktop said yes")
            self.assertEqual(reply["turn_id"], "turn-desk")
            self.assertEqual(store.receipts["host-wait"]["state"], "succeeded")


class AddressDiscoveryTest(unittest.TestCase):
    def test_select_urls_prefers_default_wifi_then_tailscale(self) -> None:
        urls = server.select_urls("192.168.1.67", [
            ("lo0", "127.0.0.1"), ("en0", "192.168.1.67"), ("bridge0", "192.168.2.1"),
            ("utun3", "100.101.102.103"), ("awdl0", "169.254.1.2"),
        ])
        self.assertEqual(urls[0][1], "192.168.1.67")
        self.assertIn("LAN", urls[0][0])
        self.assertEqual(urls[1], ("Tailscale", "100.101.102.103"))
        self.assertNotIn("192.168.2.1", [address for _, address in urls])

    def test_select_urls_without_primary_prefers_real_interfaces(self) -> None:
        urls = server.select_urls(None, [("bridge0", "192.168.2.1"), ("en0", "192.168.1.67")])
        self.assertEqual(urls[0], ("other interface (en0)", "192.168.1.67"))
        self.assertNotIn("192.168.2.1", [address for _, address in urls])

    def test_virtual_primary_is_demoted_below_wifi(self) -> None:
        urls = server.select_urls("192.168.2.1", [("bridge0", "192.168.2.1"), ("en0", "192.168.1.67")])
        self.assertEqual(urls[0][1], "192.168.1.67")
        self.assertEqual(urls[-2][1], "192.168.2.1")
        self.assertIn("virtual interface", urls[-2][0])

    def test_virtual_and_duplicate_addresses_are_filtered(self) -> None:
        urls = server.select_urls(None, [
            ("en0", "192.168.1.67"), ("en1", "192.168.1.67"), ("docker0", "172.17.0.1"),
            ("veth123", "10.0.0.2"), ("lo", "127.0.0.1"),
        ])
        self.assertEqual([address for _, address in urls], ["192.168.1.67", "127.0.0.1"])

    def test_parses_ifconfig_output(self) -> None:
        output = "en0: flags=8863<UP>\n\tinet 192.168.1.67 netmask 0xffffff00\nlo0: flags=8049<UP>\n\tinet 127.0.0.1 netmask 0xff000000\n"
        self.assertEqual(server.parse_ifconfig_ipv4(output), [("en0", "192.168.1.67"), ("lo0", "127.0.0.1")])

    def test_parses_ip_o_ipv4_output(self) -> None:
        output = "2: en0    inet 192.168.1.67/24 brd 192.168.1.255 scope global en0\n1: lo    inet 127.0.0.1/8 scope host lo\n"
        self.assertEqual(server.parse_ip_o_ipv4(output), [("en0", "192.168.1.67"), ("lo", "127.0.0.1")])


def _completed(args: list[str], code: int, stdout: str = "", stderr: str = "") -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(args, code, stdout, stderr)


class TailscaleAddressTest(unittest.TestCase):
    def test_selects_ipv4_and_magicdns_from_mocked_commands(self) -> None:
        calls: list[list[str]] = []

        def run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            calls.append(list(args))
            if args == ["tailscale", "ip", "-4"]:
                return _completed(args, 0, stdout="100.101.102.103\n")
            if args == ["tailscale", "status", "--json"]:
                body = json.dumps({"Self": {"DNSName": "studio.tailnet.ts.net.", "TailscaleIPs": ["100.101.102.103"]}})
                return _completed(args, 0, stdout=body)
            raise AssertionError(args)

        address, name = server.resolve_tailscale_endpoint(run)
        self.assertEqual(address, "100.101.102.103")
        self.assertEqual(name, "studio.tailnet.ts.net")
        self.assertEqual(calls[0], ["tailscale", "ip", "-4"])

    def test_ip_without_magicdns_still_binds(self) -> None:
        def run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            if args[1] == "ip":
                return _completed(args, 0, stdout="100.64.0.8\n")
            return _completed(args, 1, stderr="status unavailable")

        address, name = server.resolve_tailscale_endpoint(run)
        self.assertEqual(address, "100.64.0.8")
        self.assertIsNone(name)

    def test_missing_tailscale_command_is_a_clear_error(self) -> None:
        def run(_args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            raise FileNotFoundError("tailscale")

        with self.assertRaises(server.TailscaleUnavailable) as raised:
            server.resolve_tailscale_endpoint(run)
        self.assertIn("not found", str(raised.exception))
        self.assertIn("--tailscale", str(raised.exception))

    def test_daemon_down_includes_command_stderr(self) -> None:
        detail = "failed to connect to local tailscaled; is tailscaled running?"

        def run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            return _completed(args, 1, stderr=detail)

        with self.assertRaises(server.TailscaleUnavailable) as raised:
            server.resolve_tailscale_endpoint(run)
        self.assertIn("not running", str(raised.exception))
        self.assertIn(detail, str(raised.exception))

    def test_non_tailscale_address_is_rejected(self) -> None:
        def run(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
            return _completed(args, 0, stdout="192.168.1.20\n")

        with self.assertRaises(server.TailscaleUnavailable) as raised:
            server.resolve_tailscale_endpoint(run)
        self.assertIn("100.64.0.0/10", str(raised.exception))

    def test_select_tailscale_ipv4_skips_blank_lines(self) -> None:
        self.assertEqual(server.select_tailscale_ipv4("\n100.77.1.9\n"), "100.77.1.9")
        with self.assertRaises(server.TailscaleUnavailable):
            server.select_tailscale_ipv4("")

    def test_magicdns_ignores_malformed_status(self) -> None:
        self.assertIsNone(server.magicdns_name_from_status("not-json"))
        self.assertIsNone(server.magicdns_name_from_status(json.dumps({"Self": {"DNSName": ""}})))
        self.assertEqual(
            server.magicdns_name_from_status(json.dumps({"Self": {"DNSName": "phone.example.ts.net."}})),
            "phone.example.ts.net",
        )


if __name__ == "__main__":
    unittest.main()
