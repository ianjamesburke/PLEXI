"""Phone needs-you routes stay API-shaped and call the host CLI in host mode."""

from __future__ import annotations

import json
import os
import stat
import threading
import unittest
import urllib.error
import urllib.request
from pathlib import Path

import server


class NeedsYouApiTest(unittest.TestCase):
    def test_stub_lists_empty_and_refuses_resolve(self) -> None:
        httpd = server.build_server("127.0.0.1", 0, server.StubStore(), None)
        port = httpd.server_address[1]
        thread = threading.Thread(target=httpd.serve_forever, daemon=True)
        thread.start()
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/needs-you") as response:
                body = json.load(response)
            self.assertEqual(body, {"ok": True, "items": []})
            request = urllib.request.Request(
                f"http://127.0.0.1:{port}/api/needs-you/ny_1/resolve",
                data=json.dumps({"decision": "approve"}).encode(),
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with self.assertRaises(urllib.error.HTTPError) as raised:
                urllib.request.urlopen(request)
            self.assertEqual(raised.exception.code, 503)
        finally:
            httpd.shutdown()

    def test_host_mode_shells_out_to_needs_you(self) -> None:
        import tempfile

        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        fake = Path(tmp.name) / "plexi"
        fake.write_text(
            "#!/bin/sh\n"
            "printf '%s\\n' \"$*\" >> \"$FAKE_PLEXI_LOG\"\n"
            "if [ \"$1\" = needs-you ] && [ \"$2\" = list ]; then\n"
            "  printf '%s\\n' '{\"ok\":true,\"items\":[{\"id\":\"req_1\",\"kind\":\"approval_click\",\"summary\":\"play e2e4\"}]}'\n"
            "  exit 0\n"
            "fi\n"
            "printf '%s\\n' '{\"ok\":true,\"id\":\"req_1\",\"resolution\":\"approved\",\"already\":false}'\n"
            "exit 0\n"
        )
        fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
        log = Path(tmp.name) / "log"
        os.environ["FAKE_PLEXI_LOG"] = str(log)
        httpd = server.build_server("127.0.0.1", 0, server.HostStore(str(fake)), "secret-token")
        port = httpd.server_address[1]
        thread = threading.Thread(target=httpd.serve_forever, daemon=True)
        thread.start()
        try:
            denied = urllib.request.Request(f"http://127.0.0.1:{port}/api/needs-you")
            with self.assertRaises(urllib.error.HTTPError) as raised:
                urllib.request.urlopen(denied)
            self.assertEqual(raised.exception.code, 401)
            request = urllib.request.Request(
                f"http://127.0.0.1:{port}/api/needs-you",
                headers={"Authorization": "Bearer secret-token"},
            )
            with urllib.request.urlopen(request) as response:
                body = json.load(response)
            self.assertEqual(body["items"][0]["id"], "req_1")
            resolve = urllib.request.Request(
                f"http://127.0.0.1:{port}/api/needs-you/req_1/resolve",
                data=json.dumps({"decision": "approve"}).encode(),
                headers={"Authorization": "Bearer secret-token", "Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(resolve) as response:
                resolved = json.load(response)
            self.assertEqual(resolved["resolution"], "approved")
            self.assertFalse(resolved["already"])
            log_text = log.read_text()
            self.assertIn("needs-you list --json", log_text)
            self.assertIn("needs-you resolve req_1 --approve", log_text)
        finally:
            httpd.shutdown()


if __name__ == "__main__":
    unittest.main()
