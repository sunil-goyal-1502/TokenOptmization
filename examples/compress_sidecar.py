#!/usr/bin/env python3
"""
HTTP sidecar for TokenOpt `external_compress` (LLMLingua-2 when installed).

POST /compress  JSON: {"blocks": [{"id", "kind", "content"}, ...]}
Returns:        {"blocks": [{"id", "content"}, ...]}

Install LLMLingua-2:
  pip install llmlingua

Run:
  python3 examples/compress_sidecar.py
  # CompileOptions.external_compress_url = "http://127.0.0.1:8790/compress"
"""

from __future__ import annotations

import json
import os
from http.server import BaseHTTPRequestHandler, HTTPServer

_COMPRESSOR = None


def get_compressor():
    global _COMPRESSOR
    if _COMPRESSOR is not None:
        return _COMPRESSOR
    try:
        from llmlingua import PromptCompressor

        _COMPRESSOR = PromptCompressor(
            model_name=os.environ.get("LLMLINGUA_MODEL", "NousResearch/Llama-2-7b-chat-hf"),
            device_map=os.environ.get("LLMLINGUA_DEVICE", "cpu"),
            use_llmlingua2=True,
        )
        return _COMPRESSOR
    except ImportError:
        return None


def compress_text(text: str, rate: float = 0.5) -> str:
    comp = get_compressor()
    if comp is None or len(text) < 200:
        if len(text) > 200:
            head, tail = text[:80], text[-80:]
            return f"{head}\n[compressed stub]\n{tail}"
        return text
    try:
        out = comp.compress_prompt(text, rate=rate)
        if isinstance(out, dict):
            return out.get("compressed_prompt", text)
        return str(out)
    except Exception:
        head, tail = text[:80], text[-80:]
        return f"{head}\n[compress error fallback]\n{tail}"


class Handler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        if self.path != "/compress":
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length))
        rate = float(body.get("rate", 0.5))
        out = []
        for b in body.get("blocks", []):
            text = b.get("content", "")
            out.append({"id": b["id"], "content": compress_text(text, rate=rate)})
        payload = json.dumps({"blocks": out}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *_args: object) -> None:
        return


if __name__ == "__main__":
    host = os.environ.get("COMPRESS_HOST", "127.0.0.1")
    port = int(os.environ.get("COMPRESS_PORT", "8790"))
    print(f"compress sidecar on http://{host}:{port}/compress (llmlingua={'yes' if get_compressor() else 'stub'})")
    HTTPServer((host, port), Handler).serve_forever()
