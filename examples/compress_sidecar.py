#!/usr/bin/env python3
"""
Stub HTTP sidecar for TokenOpt `external_compress` transform (LLMLingua-2 hook).

POST /compress with JSON: {"blocks": [{"id": "...", "kind": "...", "content": "..."}]}
Returns: {"blocks": [{"id": "...", "content": "<compressed>"}]}

Wire: CompileOptions.external_compress_url = "http://127.0.0.1:8790/compress"
      transforms.external_compress = true
"""

from __future__ import annotations

import json
from http.server import BaseHTTPRequestHandler, HTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        if self.path != "/compress":
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length))
        blocks = body.get("blocks", [])
        out = []
        for b in blocks:
            text = b.get("content", "")
            # Placeholder: truncate middle 40% (replace with LLMLingua-2 in production)
            if len(text) > 200:
                head = text[:80]
                tail = text[-80:]
                text = f"{head}\n[compressed]\n{tail}"
            out.append({"id": b["id"], "content": text})
        payload = json.dumps({"blocks": out}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *_args: object) -> None:
        return


if __name__ == "__main__":
    HTTPServer(("127.0.0.1", 8790), Handler).serve_forever()
