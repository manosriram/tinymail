#!/usr/bin/env python3
"""Serves the real ui/ with a mocked Tauri backend so design prototypes can be
compared in a browser:  python3 prototypes/serve.py [port]  →  open /prototypes/

/?p=N&theme=light|dark  renders ui/index.html with prototypes/themes/pN.css
layered over ui/styles.css (p1 is the shipped stylesheet, no override).
"""
import http.server
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
UI = ROOT / "ui"
PROTO = ROOT / "prototypes"


class Handler(http.server.SimpleHTTPRequestHandler):
    def send_text(self, body: str, ctype: str):
        data = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        path, _, query = self.path.partition("?")
        if path in ("/", "/index.html"):
            p = re.search(r"(?:^|&)p=(\d)", query)
            theme = f'<link rel="stylesheet" href="/prototypes/themes/p{p.group(1)}.css" />' if p and p.group(1) != "1" else ""
            html = (UI / "index.html").read_text()
            html = html.replace('<link rel="stylesheet" href="styles.css" />', f'<link rel="stylesheet" href="styles.css" />{theme}')
            html = html.replace('<script type="module"', '<script src="/prototypes/mock-tauri.js"></script><script type="module"')
            return self.send_text(html, "text/html")
        if path == "/fonts/fonts.css":
            extra = (PROTO / "fonts" / "fonts.css").read_text().replace("url(./", "url(/prototypes/fonts/")
            return self.send_text((UI / "fonts" / "fonts.css").read_text() + "\n" + extra, "text/css")
        return super().do_GET()

    def translate_path(self, path):
        base = ROOT if path.startswith("/prototypes/") else UI
        return str(base / path.split("?")[0].lstrip("/"))

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
