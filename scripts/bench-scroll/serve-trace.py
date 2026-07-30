#!/usr/bin/env python3
"""Tiny same-origin server for smoothness.html A/B runs.

Serves this directory over HTTP and accepts POST /trace, writing the
body to the file given as argv[2]. Exits after one trace is received.
Usage: serve-trace.py <port> <out.json>
"""
import http.server
import pathlib
import sys
import threading

PORT = int(sys.argv[1])
OUT = pathlib.Path(sys.argv[2])
HERE = pathlib.Path(__file__).parent


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(HERE), **kw)

    def do_POST(self):
        if self.path != "/trace":
            self.send_error(404)
            return
        n = int(self.headers.get("Content-Length", 0))
        OUT.write_bytes(self.rfile.read(n))
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")
        threading.Thread(target=self.server.shutdown, daemon=True).start()

    def log_message(self, *a):
        pass


http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
