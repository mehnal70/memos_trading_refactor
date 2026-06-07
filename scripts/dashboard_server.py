#!/usr/bin/env python3
# scripts/dashboard_server.py — Memos RTC mobil izleme paneli için minimal HTTP sunucu.
#
# SADECE iki yol sunar (data/ dizinini AÇMAZ → trader.db sızmaz):
#   GET /               → web/index.html (mobil panel)
#   GET /snapshot.json  → data/snapshot.json (motorun her 1s yazdığı canlı durum, no-cache)
# Salt-okuma; hiçbir kontrol/emir yok. 0.0.0.0'a bağlanır → LAN + tünel (Tailscale) üzerinden erişilir.
#
# Kullanım: python3 scripts/dashboard_server.py [PORT]   (default 8090)
import http.server, socketserver, os, sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INDEX = os.path.join(REPO, "web", "index.html")
SNAP  = os.path.join(REPO, "data", "snapshot.json")
PORT  = int(sys.argv[1]) if len(sys.argv) > 1 else 8090

class H(http.server.BaseHTTPRequestHandler):
    def _send(self, code, body, ctype):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        try:
            if path == "/" or path == "/index.html":
                with open(INDEX, "rb") as f:
                    self._send(200, f.read(), "text/html; charset=utf-8")
            elif path == "/snapshot.json":
                try:
                    import time
                    age = int(time.time() - os.path.getmtime(SNAP))  # motor canlı mı → dosya tazeliği
                    with open(SNAP, "rb") as f:
                        body = f.read()
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json; charset=utf-8")
                    self.send_header("Cache-Control", "no-store")
                    self.send_header("X-Snapshot-Age", str(age))  # panel bayatlığı buradan okur
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                except FileNotFoundError:
                    self._send(503, b'{"error":"snapshot yok - motor calisiyor mu?"}', "application/json")
            else:
                self._send(404, b"not found", "text/plain")
        except BrokenPipeError:
            pass

    def log_message(self, *a):  # sessiz (stdout'u kirletme)
        pass

class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True

if __name__ == "__main__":
    with Server(("0.0.0.0", PORT), H) as httpd:
        print(f"📲 Memos panel: http://0.0.0.0:{PORT}  (Ctrl-C ile dur)")
        httpd.serve_forever()
