"""A local HTTPS server for the `network` tests (not part of NexaOS).

Usage: python3 server.py DIR
Makes a certificate valid only for the IP address 127.0.0.1 (DIR/cert.pem,
also the file the client trusts through NEXALANG_TEST_CA), so asking for
https://localhost:PORT/ fails the certificate check. Prints "ready PORT",
then answers until a request for /stop."""
import gzip, http.server, json, os, ssl, subprocess, sys, threading, time

d = sys.argv[1]
cert, key = os.path.join(d, "cert.pem"), os.path.join(d, "key.pem")
subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
                "-subj", "/CN=nexalang test", "-addext", "subjectAltName=IP:127.0.0.1",
                "-keyout", key, "-out", cert], check=True, capture_output=True)

PNG = bytes.fromhex("89504e470d0a1a0a0000000d4948445200000001000000010806000000"
                    "1f15c4890000000d49444154789c6360000002000154a24f5d0000000049454e44ae426082")


class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def send(self, status, body=b"", ctype="text/plain; charset=utf-8", extra=()):
        self.send_response(status)
        if ctype:
            self.send_header("Content-Type", ctype)
        for k, v in extra:
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""

    def any(self):
        p = self.path
        port = self.server.server_address[1]
        if p == "/hello":
            self.send(200, b"hello\n")
        elif p == "/json":
            self.send(200, b'{"name": "Ama", "n": 3}', "application/json")
        elif p.startswith("/echo"):
            got = self.body().decode("utf-8", "replace")
            names = ["x-test", "x-empty", "content-type", "accept", "user-agent", "cookie", "authorization"]
            heads = {n: self.headers.get(n) for n in names if self.headers.get(n) is not None}
            out = {"method": self.command, "path": p, "body": got, "headers": heads}
            self.send(200, json.dumps(out, sort_keys=True).encode(), "application/json")
        elif p.startswith("/status/"):
            self.send(int(p[8:]), b"nope")
        elif p == "/redirect":
            self.send(302, b"", None, [("Location", "/hello")])
        elif p == "/loop":
            self.send(302, b"", None, [("Location", "/loop")])
        elif p == "/to-http":
            self.send(302, b"", None, [("Location", f"http://127.0.0.1:{port}/hello")])
        elif p == "/big":
            self.send(200, b"a" * (2 << 20))
        elif p == "/big-chunked":
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            chunk = b"b" * 65536
            try:
                for _ in range(64):
                    self.wfile.write(b"%x\r\n" % len(chunk) + chunk + b"\r\n")
                self.wfile.write(b"0\r\n\r\n")
            except OSError:
                pass  # the client stopped reading: that's the point
        elif p == "/bomb":
            self.send(200, gzip.compress(b"\0" * (64 << 20)), "text/plain", [("Content-Encoding", "gzip")])
        elif p == "/gzip":
            self.send(200, gzip.compress("unpacked fine ✓\n".encode()), "text/plain; charset=utf-8", [("Content-Encoding", "gzip")])
        elif p == "/binary":
            self.send(200, PNG, "image/png")
        elif p == "/slow":
            time.sleep(3)
            self.send(200, b"too late\n")
        elif p == "/headers":
            self.send(200, b"", "text/plain", [("ETag", '"abc"'), ("X-One", "1"), ("X-One", "2"), ("Set-Cookie", "session=secret")])
        elif p == "/html":
            self.send(200, b"<!doctype html><title>x</title>", "text/html; charset=utf-8")
        elif p == "/empty":
            self.send(204, b"", None)
        elif p == "/close":
            self.close_connection = True  # nothing sent at all
        elif p == "/stop":
            self.send(200, b"bye\n")
            threading.Thread(target=self.server.shutdown).start()
        else:
            self.send(404, b"no such test page")

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_HEAD = any


class Server(http.server.ThreadingHTTPServer):
    def handle_error(self, request, client_address):
        pass  # the client hanging up early is what several tests check


srv = Server(("127.0.0.1", 0), H)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(cert, key)
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
print("ready", srv.server_address[1], flush=True)
srv.serve_forever()
