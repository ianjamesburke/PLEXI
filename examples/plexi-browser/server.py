#!/usr/bin/env python3
"""
server.py — Test HTTP server for Plexi manifest discovery

Serves /.well-known/plexi.json on localhost:7777.
Returns 404 for all other paths.
"""

import json
from http.server import BaseHTTPRequestHandler, HTTPServer

MANIFEST = {
    "name": "Plexi Demo Site",
    "version": "1.0.0",
    "description": "A demonstration of Plexi manifest discovery",
    "display": "standalone",
    "permissions": [],
    "theme": {
        "background": "#1e1e2e",
        "foreground": "#cdd6f4",
        "accent": "#89b4fa",
    },
    "draw": [
        {"type": "rect", "x": 0, "y": 0, "w": 800, "h": 600, "fill": "#1e1e2e", "radius": 0},

        # Header bar
        {"type": "rect", "x": 0, "y": 0, "w": 800, "h": 56, "fill": "#181825", "radius": 0},
        {"type": "text", "x": 24, "y": 18, "text": "Plexi Demo Site", "size": 18, "color": "#cdd6f4", "bold": True},
        {"type": "text", "x": 24, "y": 38, "text": "Native UI via /.well-known/plexi.json", "size": 11, "color": "#6c7086"},
        {"type": "line", "x1": 0, "y1": 56, "x2": 800, "y2": 56, "color": "#313244", "width": 1},

        # Section label
        {"type": "text", "x": 24, "y": 80, "text": "BUTTONS", "size": 10, "color": "#6c7086", "bold": True},

        # Primary button
        {"type": "rect", "x": 24, "y": 96, "w": 120, "h": 36, "fill": "#89b4fa", "radius": 6},
        {"type": "text", "x": 52, "y": 108, "text": "Connect", "size": 13, "color": "#1e1e2e", "bold": True},

        # Secondary button
        {"type": "rect", "x": 156, "y": 96, "w": 120, "h": 36, "fill": "#313244", "radius": 6},
        {"type": "text", "x": 184, "y": 108, "text": "Settings", "size": 13, "color": "#cdd6f4"},

        # Danger button
        {"type": "rect", "x": 288, "y": 96, "w": 120, "h": 36, "fill": "#f38ba8", "radius": 6},
        {"type": "text", "x": 322, "y": 108, "text": "Delete", "size": 13, "color": "#1e1e2e", "bold": True},

        # Ghost / outline button
        {"type": "rect", "x": 420, "y": 96, "w": 120, "h": 36, "fill": "#1e1e2e", "radius": 6},
        {"type": "rect", "x": 420, "y": 96, "w": 120, "h": 36, "fill": "#313244", "radius": 6},
        {"type": "text", "x": 452, "y": 108, "text": "Cancel", "size": 13, "color": "#89b4fa"},

        # Section label
        {"type": "text", "x": 24, "y": 152, "text": "DROPDOWN (simulated)", "size": 10, "color": "#6c7086", "bold": True},

        # Dropdown box
        {"type": "rect", "x": 24, "y": 168, "w": 220, "h": 36, "fill": "#313244", "radius": 6},
        {"type": "text", "x": 36, "y": 180, "text": "Sort by: Recently modified", "size": 13, "color": "#cdd6f4"},
        {"type": "text", "x": 216, "y": 180, "text": "▾", "size": 13, "color": "#6c7086"},

        # Dropdown open state below
        {"type": "rect", "x": 24, "y": 204, "w": 220, "h": 128, "fill": "#313244", "radius": 6},
        {"type": "text", "x": 36, "y": 218, "text": "✓  Recently modified", "size": 13, "color": "#89b4fa"},
        {"type": "line", "x1": 36, "y1": 236, "x2": 228, "y2": 236, "color": "#45475a", "width": 1},
        {"type": "text", "x": 36, "y": 246, "text": "    Name A → Z", "size": 13, "color": "#cdd6f4"},
        {"type": "text", "x": 36, "y": 270, "text": "    Name Z → A", "size": 13, "color": "#cdd6f4"},
        {"type": "text", "x": 36, "y": 294, "text": "    File size", "size": 13, "color": "#cdd6f4"},
        {"type": "text", "x": 36, "y": 318, "text": "    Date created", "size": 13, "color": "#cdd6f4"},

        # Section label
        {"type": "text", "x": 24, "y": 358, "text": "LIST", "size": 10, "color": "#6c7086", "bold": True},

        # List widget
        {"type": "list", "items": [
            {"label": "Wikipedia",    "secondary": "wikipedia.py — Browse articles"},
            {"label": "Plexi Browser","secondary": "browser.py — Manifest discovery"},
            {"label": "Git Log",      "secondary": "git-log — Interactive commit history"},
            {"label": "Process Monitor","secondary": "process-monitor — Live process table"},
        ], "selected": 1, "item_height": 44},

        # Status bar
        {"type": "rect", "x": 0, "y": 572, "w": 800, "h": 28, "fill": "#181825", "radius": 0},
        {"type": "text", "x": 24, "y": 580, "text": "4 apps installed", "size": 11, "color": "#6c7086"},
        {"type": "text", "x": 680, "y": 580, "text": "X-Plexi-Client: 1", "size": 11, "color": "#a6e3a1"},
    ],
}

MANIFEST_PATH = "/.well-known/plexi.json"


class PlexiHandler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):  # noqa: A002 — must match base signature
        plexi_client = self.headers.get("X-Plexi-Client", "-")
        print(f"[{self.address_string()}] {self.command} {self.path} → {args[1]} (X-Plexi-Client: {plexi_client})")

    def do_GET(self):
        if self.path == MANIFEST_PATH:
            body = json.dumps(MANIFEST, indent=2).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            # Echo back the Plexi client header if present
            if self.headers.get("X-Plexi-Client"):
                self.send_header("X-Plexi-Client", self.headers["X-Plexi-Client"])
            self.end_headers()
            self.wfile.write(body)
        else:
            body = b'{"error": "Not Found"}'
            self.send_response(404)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)


if __name__ == "__main__":
    host = "localhost"
    port = 7777
    server = HTTPServer((host, port), PlexiHandler)
    print(f"Plexi test server running at http://{host}:{port}")
    print(f"Manifest available at http://{host}:{port}{MANIFEST_PATH}")
    print("Press Ctrl+C to stop.")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nServer stopped.")
