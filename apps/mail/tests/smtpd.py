"""A small SMTP server for Mail's tests: STARTTLS (required before AUTH),
AUTH PLAIN and LOGIN, then MAIL/RCPT/DATA. Each message is kept in
OUTBOX, and one for a local user is put in their Dovecot Maildir.

    python3 smtpd.py PORT CERT KEY USERS_FILE MAILHOME OUTBOX"""
import base64
import os
import socket
import ssl
import sys
import threading
import time

PORT, CERT, KEY, USERS, MAILHOME, OUTBOX = sys.argv[1:7]
os.makedirs(OUTBOX, exist_ok=True)
users = {}
for line in open(USERS):
    name, _, rest = line.strip().partition(":")
    users[name] = rest.replace("{PLAIN}", "")
ctx = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
ctx.load_cert_chain(CERT, KEY)
count = [0]


def deliver(rcpts, data):
    count[0] += 1
    with open(os.path.join(OUTBOX, f"{count[0]}.eml"), "wb") as f:
        f.write(data)
    for r in rcpts:
        if r in users:
            new = os.path.join(MAILHOME, r, "Maildir", "new")
            os.makedirs(new, exist_ok=True)
            name = os.path.join(new, f"{time.time():.6f}.{count[0]}.test")
            with open(name, "wb") as f:
                f.write(data)
            os.system(f"chown -R vmail:vmail '{os.path.join(MAILHOME, r)}'")


def session(conn):
    f = conn.makefile("rwb")

    def say(s):
        f.write((s + "\r\n").encode())
        f.flush()

    tls = authed = False
    mail_from, rcpts = None, []
    say("220 localhost test smtpd")
    while True:
        line = f.readline()
        if not line:
            return
        cmd = line.decode(errors="replace").strip()
        up = cmd.upper()
        if up.startswith("EHLO") or up.startswith("HELO"):
            f.write(b"250-localhost\r\n")
            if not tls:
                f.write(b"250-STARTTLS\r\n")
            else:
                f.write(b"250-AUTH PLAIN LOGIN\r\n")
            say("250 SIZE 10000000")
        elif up == "STARTTLS" and not tls:
            say("220 go ahead")
            conn = ctx.wrap_socket(conn, server_side=True)
            f = conn.makefile("rwb")
            tls = True
        elif up.startswith("AUTH"):
            if not tls:
                say("530 must use STARTTLS first")
                continue
            parts = cmd.split()
            if parts[1].upper() == "PLAIN":
                blob = parts[2] if len(parts) > 2 else None
                if not blob:
                    say("334 ")
                    blob = f.readline().decode().strip()
                _, u, p = base64.b64decode(blob).decode().split("\0")
            else:
                say("334 VXNlcm5hbWU6")
                u = base64.b64decode(f.readline().strip()).decode()
                say("334 UGFzc3dvcmQ6")
                p = base64.b64decode(f.readline().strip()).decode()
            if users.get(u) == p:
                authed = True
                say("235 ok")
            else:
                say("535 bad credentials")
        elif up.startswith("MAIL FROM:"):
            if not authed:
                say("530 auth first")
                continue
            mail_from, rcpts = cmd[10:].strip().strip("<>"), []
            say("250 ok")
        elif up.startswith("RCPT TO:"):
            r = cmd[8:].strip().split()[0].strip("<>").lower()
            if r.endswith("@nowhere.invalid"):
                say("550 no such user")
                continue
            rcpts.append(r)
            say("250 ok")
        elif up == "DATA":
            say("354 end with .")
            lines = []
            while True:
                l = f.readline()
                if not l or l in (b".\r\n", b".\n"):
                    break
                lines.append(l[1:] if l.startswith(b"..") else l)
            deliver(rcpts, b"".join(lines))
            say("250 queued")
        elif up in ("RSET", "NOOP"):
            say("250 ok")
        elif up == "QUIT":
            say("221 bye")
            return
        else:
            say("502 not here")


srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", int(PORT)))
srv.listen(8)
while True:
    c, _ = srv.accept()
    threading.Thread(target=session, args=(c,), daemon=True).start()
