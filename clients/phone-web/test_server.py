"""Stub server contract tests. Run: python3 -m unittest clients/phone-web/test_server.py"""

import json
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
            fake.write_text("#!/bin/sh\nprintf '%s\\n' '{\"state\":\"succeeded\",\"reply\":\"real-ish reply\"}'\n")
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
            self.assertIn("assistant_reply", [event["kind"] for event in page["events"]])

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


class TailscaleBindTests(unittest.TestCase):
    def test_selects_first_tailscale_ipv4(self) -> None:
        self.assertEqual(server.select_tailscale_ipv4("100.100.1.2\n"), "100.100.1.2")

    def test_rejects_missing_tailscale_address(self) -> None:
        with self.assertRaises(server.TailscaleUnavailable):
            server.select_tailscale_ipv4("192.168.1.67\n")

    def test_magicdns_from_status(self) -> None:
        name = server.magicdns_name_from_status(json.dumps({"Self": {"DNSName": "mac.example.ts.net."}}))
        self.assertEqual(name, "mac.example.ts.net")

    def test_resolve_uses_injected_runner(self) -> None:
        def run(argv, **_kwargs):
            class Result:
                returncode = 0
                stdout = "100.64.0.8\n" if argv[:3] == ["tailscale", "ip", "-4"] else json.dumps({"Self": {"DNSName": "host.tail.ts.net."}})
                stderr = ""
            return Result()
        address, name = server.resolve_tailscale_endpoint(run)
        self.assertEqual(address, "100.64.0.8")
        self.assertEqual(name, "host.tail.ts.net")


if __name__ == "__main__":
    unittest.main()
