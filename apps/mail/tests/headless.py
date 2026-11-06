"""Mail in a headless Sway against a real IMAP server (Dovecot, TLS) and a
test SMTP server (STARTTLS), started the way an installed app is: through
nexa-run, confined to network, files.documents and files.pictures. Driven
through MAIL_TEST; screenshots go to $OUT. (scripts/test-mail-app.sh)"""
import email.utils
import json
import os
import subprocess
import sys
import time

OUT = os.environ.get("OUT", "/tmp")
SMOKE = bool(os.environ.get("SMOKE"))
HOME = os.environ["HOME"]
T = "/tmp/mt"
STATE, CMD = os.path.join(T, "state.json"), os.path.join(T, "cmd")
MAILDIR = "/tmp/mailhome/ama@test.local/Maildir"
CA = "/tmp/tls/ca.pem"
fails = 0
proc = None


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"), flush=True)
    fails += not ok


def state():
    try:
        return json.load(open(STATE))
    except (OSError, ValueError):
        return {}


def wait(cond, secs=15.0):
    end = time.time() + secs
    while time.time() < end:
        try:
            if cond(state()):
                return True
        except (KeyError, TypeError, IndexError):
            pass
        time.sleep(0.05)
    return False


def send(*lines):
    with open(CMD, "w") as f:
        for line in lines:
            f.write(line + "\n")
    time.sleep(0.25)


def shot(name):
    time.sleep(0.6)
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def imap(cmd, folder=""):
    """What the server itself says (curl, as a second opinion): every line
    it sent, from curl -v, since curl leaves some answers (UID FETCH) out
    of its output."""
    r = subprocess.run(["curl", "-sv", "--cacert", CA, "-u", "ama@test.local:hunter2", f"imaps://localhost:993/{folder}", "-X", cmd],
                       capture_output=True, text=True)
    return r.stdout + "".join(l[2:] + "\n" for l in r.stderr.splitlines() if l.startswith("< "))


def seed(n, frm, subject, body, extra=""):
    os.makedirs(os.path.join(MAILDIR, "new"), exist_ok=True)
    date = email.utils.formatdate(time.time() - (10 - n) * 3600, localtime=False)
    msg = f"From: {frm}\r\nTo: Ama <ama@test.local>\r\n{extra}Subject: {subject}\r\nDate: {date}\r\nMessage-ID: <m{n}@test.local>\r\n{body}"
    with open(os.path.join(MAILDIR, "new", f"{1700000000 + n}.{n}.seed"), "w") as f:
        f.write(msg)


def seed_all():
    seed(1, "Kofi Mensah <kofi@test.local>", "Lunch on Friday?", "\r\nAre you free for lunch on Friday? There's a new place near the office.\r\n\r\nKofi\r\n")
    seed(2, "Shop <deals@shop.example>", "Big sale this week",
         "MIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><head><style>p{color:red}</style>"
         "<script>alert('x')</script></head><body><p>Big sale &amp; free delivery</p>"
         "<img src=\"https://track.shop.example/open.gif?u=ama\" width=1 height=1><a href=\"https://shop.example/sale\">See the deals</a></body></html>\r\n")
    seed(3, "Accounts <billing@test.local>", "Your invoice for October",
         "MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n"
         "Your invoice is attached.\r\n--b1\r\nContent-Type: application/pdf; name=\"invoice.pdf\"\r\nContent-Disposition: attachment; "
         "filename=\"invoice.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQKJcOkw7zDtsOfCg==\r\n--b1--\r\n")
    seed(4, "Abena <abena@test.local>", "Trip photos", "\r\nI'll send the photos tonight.\r\n",
         "Cc: Kofi <kofi@test.local>, Yaw <yaw@test.local>\r\n")
    seed(5, "Café Ghana <info@cafe.example>", "=?ISO-8859-1?Q?Caf=E9_opening?=", "Content-Type: text/plain; charset=iso-8859-1\r\n\r\nWe open on Monday. Bienvenue \xe0 tous.\r\n")
    os.system(f"chown -R vmail:vmail /tmp/mailhome")


def start():
    global proc
    if os.path.exists(STATE):
        os.remove(STATE)
    proc = subprocess.Popen(["nexa-run", "mail"], env=dict(os.environ, MAIL_TEST=T, MAIL_CA_FILE=CA), stderr=open("/tmp/mail.err", "a"))
    return wait(lambda s: s.get("page"), 15)


def stop():
    proc.terminate()
    proc.wait(5)


os.makedirs(T, exist_ok=True)
if not os.path.exists(CMD):
    os.mkfifo(CMD)
seed_all()

if SMOKE:
    check(start(), "from the package: it starts")
    send("setup Ama|ama@test.local|hunter2", "server localhost|993|0|localhost|587|1", "signin")
    check(wait(lambda s: s["page"] == "mail" and len(s["list"]) >= 5, 30), "signs in and lists the Inbox", state().get("setup_msg"))
    subprocess.run(["grim", os.path.join(OUT, "mail-from-package.png")])
    stop()
    print(f"{fails} failed")
    sys.exit(1 if fails else 0)

explain = subprocess.run(["nexa-run", "--explain", "mail"], capture_output=True, text=True).stdout
check("network" in explain and "files.documents" in explain and "files.pictures" in explain and "files.home" not in explain,
      "nexa-run: Mail may use the internet, Documents and Pictures, not the whole home", explain)

# ---- setting up
check(start(), "starts")
check(state()["page"] == "setup", "first start: add an account")
shot("mail-setup.png")
send("setup Ama Bôa|ama@test.local|wrong password", "server example.com|143|2|example.com|25|2", "signin")
check(wait(lambda s: "only allowed to this computer" in s["setup_msg"], 10), "a plain connection to another computer is refused, before any password goes out",
      state().get("setup_msg"))
send("server localhost|993|0|localhost|587|1", "signin")
check(wait(lambda s: "didn't accept the email address or password" in s["setup_msg"], 15), "a wrong password: said so, with the app password hint",
      state().get("setup_msg"))
shot("mail-wrong-password.png")
send("setup Ama Bôa|ama@test.local|hunter2", "signin")
check(wait(lambda s: s["page"] == "mail" and len(s["list"]) == 5, 30), "signed in: the Inbox's five messages", state().get("setup_msg"))
s = state()
names = [f["name"] for f in s["folders"]]
check({"Inbox", "Sent", "Trash", "Drafts", "Junk"} <= set(names), "the folders, known by their roles", names)
check(next(f for f in s["folders"] if f["name"] == "Inbox")["unseen"] == 5, "five unread")
subjects = [m["subject"] for m in s["list"]]
check(subjects[0] == "Café opening" and "Lunch on Friday?" in subjects, "newest first; an encoded Latin-1 subject", subjects)
secrets = open(os.path.join(HOME, ".local/share/mail/secrets")).read()
check("hunter2" not in secrets and oct(os.stat(os.path.join(HOME, ".local/share/mail/secrets")).st_mode & 0o777) == "0o600",
      "the password is kept encrypted, only for you")

# ---- reading
i = subjects.index("Big sale this week")
send(f"open {i}")
check(wait(lambda s: s["reading"] and "Big sale & free delivery" in s["reading"]["text"]), "an HTML newsletter, as text")
r = state()["reading"]
check(r["html"] and "alert" not in r["text"] and "color" not in r["text"] and "track.shop" not in r["text"] and
      "See the deals <https://shop.example/sale>" in r["text"], "no script, no style, no tracking picture; the link kept", r["text"])
check(wait(lambda s: s["list"][i]["seen"], 5), "opened: read")
shot("mail-reading.png")
i = [m["subject"] for m in state()["list"]].index("Your invoice for October")
send(f"open {i}")
check(wait(lambda s: s["reading"] and s["reading"]["attachments"] == 1), "an attachment")
send("save 0")
saved = os.path.join(HOME, "Documents", "Mail attachments", "invoice.pdf")
check(wait(lambda s: "Saved to Documents/Mail attachments" in s["status"]) and open(saved, "rb").read().startswith(b"%PDF-1.4"),
      "saved into Documents, decoded", state().get("status"))
send("flag")
time.sleep(1.5)
check("\\Flagged" in imap("UID FETCH 1:* (UID FLAGS)", "INBOX"), "flagged on the server too")

# ---- replying
i = [m["subject"] for m in state()["list"]].index("Trip photos")
send(f"open {i}")
wait(lambda s: s["reading"] and "photos tonight" in s["reading"]["text"])
send("reply 1")
check(wait(lambda s: s["composes"] == 1), "Reply All opens a message")
c = state()["compose"]
check(c["to"] == "Abena <abena@test.local>" and "kofi@test.local" in c["cc"] and "yaw@test.local" in c["cc"] and "ama@test.local" not in c["cc"]
      and c["subject"] == "Re: Trip photos" and "> I'll send the photos tonight." in c["body"],
      "to the sender, everyone else in Cc (not me), Re:, the quote", c)
send("body Lovely, thank you!\\n\\n> I'll send the photos tonight.")
shot("mail-reply.png")
send("send")
check(wait(lambda s: s["sends"] == 1 and s["composes"] == 0, 20), "sent", state().get("compose"))
outbox = sorted(os.listdir("/tmp/outbox"))
sent = open(os.path.join("/tmp/outbox", outbox[-1]), encoding="utf-8", errors="replace").read() if outbox else ""
check("In-Reply-To: <m4@test.local>" in sent and "Subject: Re: Trip photos" in sent and "Lovely, thank you!" in sent,
      "the reply on the wire: threaded, the text", sent[:400])
check(wait(lambda s: "MESSAGES 1" in imap('STATUS "Sent" (MESSAGES)'), 5), "a copy in Sent")
check("\\Answered" in imap("UID FETCH 4 (FLAGS)", "INBOX"), "the original marked as answered")

# ---- writing, with a picture
os.makedirs(os.path.join(HOME, "Pictures"), exist_ok=True)
with open(os.path.join(HOME, "Pictures", "beach day.png"), "wb") as f:
    f.write(b"\x89PNG\r\n\x1a\n" + os.urandom(5000))
send("write", "to not-an-address", "subject Hello", "send")
check(wait(lambda s: "isn't an email address" in s["compose"]["msg"]), "a bad address: said so, nothing sent", state().get("compose"))
send(f"to Me <ama@test.local>", "subject Beach day", "body The photo from Saturday.", f"attach {HOME}/Pictures/beach day.png")
check(wait(lambda s: s["compose"]["attachments"] == 1), "a picture attached")
# the rest of the home folder is out of reach (/etc is readable by design: libraries, certificates)
os.makedirs(os.path.join(HOME, ".ssh"), exist_ok=True)
with open(os.path.join(HOME, ".ssh", "id_test"), "w") as f:
    f.write("private\n")
send(f"attach {HOME}/.ssh/id_test")
check(wait(lambda s: "can't read that file" in s["compose"]["msg"]), "a file outside Documents and Pictures can't be read", state()["compose"]["msg"])
send("send")
check(wait(lambda s: s["sends"] == 2 and s["composes"] == 0, 20), "sent with the picture")
time.sleep(1)
send("refresh")
check(wait(lambda s: any(m["subject"] == "Beach day" for m in s["list"]), 20), "and it arrives in the Inbox",
      f'status {state().get("status")!r}, list {[m["subject"] for m in state().get("list", [])]}, on the server {imap("UID SEARCH ALL", "INBOX").strip()!r}')
i = [m["subject"] for m in state()["list"]].index("Beach day")
send(f"open {i}")
check(wait(lambda s: s["reading"] and s["reading"]["attachments"] == 1 and "photo from Saturday" in s["reading"]["text"]), "with its picture",
      state().get("reading"))

# ---- search
send("search lunch")
check(wait(lambda s: [m["subject"] for m in s["list"]] == ["Lunch on Friday?"]), "search: the subjects and names here", state().get("list"))
send("search new place near", "search-server")
check(wait(lambda s: [m["subject"] for m in s["list"]] == ["Lunch on Friday?"], 15), "Enter: the server searches the text of every message",
      state().get("list"))
send("search ")

# ---- deleting
wait(lambda s: len(s["list"]) == 6)
i = [m["subject"] for m in state()["list"]].index("Big sale this week")
send(f"open {i}")
time.sleep(0.5)
send("delete")
check(wait(lambda s: "Moved to Trash" in s["status"] and len(s["list"]) == 5), "Delete: moved to Trash")
check(wait(lambda s: "MESSAGES 1" in imap('STATUS "Trash" (MESSAGES)'), 5), "on the server too")
send("folder Trash")
check(wait(lambda s: s["folder"] == "Trash" and len(s["list"]) == 1, 15), "Trash has it")
send("open 0")
time.sleep(0.5)
send("delete")
check(wait(lambda s: "MESSAGES 0" in imap('STATUS "Trash" (MESSAGES)'), 8), "in Trash: deleted for good")
send("folder Inbox")
wait(lambda s: s["folder"] == "Inbox")
stop()

# ---- offline
os.system("pkill -f 'dovecot -c' ; pkill -f imap-login")
time.sleep(1)
start()
check(wait(lambda s: s["page"] == "mail" and len(s["list"]) == 5, 10), "offline: the mail is still listed")
i = [m["subject"] for m in state()["list"]].index("Lunch on Friday?")
send(f"open {i}")
check(wait(lambda s: s["reading"] and "free for lunch" in s["reading"]["text"]), "and a message read before still reads")
check(wait(lambda s: any(w in s["status"] for w in ("didn't answer", "broke off", "can't find")), 25),
      "checking for mail: said why it couldn't", state().get("status"))
shot("mail-offline.png")
stop()

err = open("/tmp/mail.err").read()
bad = [l for l in err.splitlines() if ("CRITICAL" in l or "WARNING" in l) and "Glycin" not in l]
check(not bad, "no GTK warnings or criticals", "\n".join(bad[:5]))
print(f"{fails} failed")
sys.exit(1 if fails else 0)
