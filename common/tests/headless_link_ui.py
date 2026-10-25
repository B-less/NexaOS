"""Nexa Link's screens, driven like a person would, against a real second
device (the "phone": nexa-linkd in another container on the same private
network, driven by scripts/test-nexa-link-ui.sh over a FIFO):

  Settings > Nexa Link: Pair a device shows a QR code and the code; the
  phone types it; both show six digits; "Yes, they match"; the phone is
  listed, nearby. The pop-ups: a file offered (Save), received (Open),
  a link (Copy/Open), Ring (Stop). Files: Send to a Device.

Screenshots go to $OUT."""
import os
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
for sub in ("common", "settings-app", "shell", "file-manager"):
    sys.path.insert(0, os.path.join(REPO, sub))

import gi  # noqa: E402
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

from nexa_common import link, linkui  # noqa: E402

OUT = os.environ.get("OUT", "/tmp")
SHARED = os.environ.get("PHONE_DIR", "/shared")
PHONE_CMD, PHONE_OUT = os.path.join(SHARED, "phone.cmd"), os.path.join(SHARED, "phone.out")
failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""), flush=True)
    if not cond:
        failures.append(label)


def pump(seconds=0.2, until=None):
    end = time.time() + seconds
    while True:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        if until and until():
            return True
        if time.time() > end:
            return bool(until and until())
        time.sleep(0.02)


def phone(cmd):
    """Run a shell command on the phone; its output."""
    if os.path.exists(PHONE_OUT):
        os.remove(PHONE_OUT)
    with open(PHONE_CMD, "w") as f:
        f.write(cmd + "\n")
    end = time.time() + 60
    while time.time() < end and not os.path.exists(PHONE_OUT):
        pump(0.1)
    pump(0.2)
    return open(PHONE_OUT).read() if os.path.exists(PHONE_OUT) else ""


def shot(name):
    subprocess.run(["grim", os.path.join(OUT, name)], check=False)


def find(widget, kind, text=None):
    if isinstance(widget, kind) and (text is None or (hasattr(widget, "get_label") and (widget.get_label() or "").startswith(text))
                                     or (hasattr(widget, "get_text") and text in (widget.get_text() or ""))):
        return widget
    if isinstance(widget, Gtk.Container):
        for c in widget.get_children():
            hit = find(c, kind, text)
            if hit:
                return hit
    return None


# ---- Settings > Nexa Link ---------------------------------------------------
from nexa_settings import window as SW  # noqa: E402
from nexa_settings import link_page  # noqa: E402

win = SW.SettingsWindow()
win.show_all()
win._on_nav(None, "Nexa Link")
pump(2.0, until=lambda: "This device" in win.link_page.me_label.get_text())
check("the page shows this device", win.link_page.me_label.get_text().startswith("This device: "), win.link_page.me_label.get_text())
shot("1-settings.png")

dialog = link_page.PairDialog(win)
pump(5, until=lambda: find(dialog.body, link_page.QrCode) is not None)
qr = find(dialog.body, link_page.QrCode)
code_label = find(dialog.body, Gtk.Label, "NEXA-LINK:1:")
check("Pair a device shows a QR code and the code", qr is not None and code_label is not None)
shot("2-pair-qr.png")
code = code_label.get_text() if code_label else ""
with open(PHONE_CMD, "w") as f:  # the phone types it (and answers yes)
    f.write(f"nexa-linkd pair-accept '{code}' --json --yes > /tmp/pair.json 2>&1\n")
pump(15, until=lambda: find(dialog.body, Gtk.Button, "Yes, they match") is not None)
yes = find(dialog.body, Gtk.Button, "Yes, they match")
digits = [w for w in dialog.body.get_children() if isinstance(w, Gtk.Label) and w.get_label() and "xx-large" in w.get_label()]
check("both screens: six digits to compare", yes is not None and bool(digits))
shot("3-pair-digits.png")
if yes:
    yes.clicked()
pump(15, until=lambda: dialog.paired)
check("“Yes, they match”: paired", dialog.paired)
shot("4-paired.png")
dialog.destroy()
pump(9, until=lambda: any(d.get("online") for d in link.devices()))
win.link_page._last = None
win.link_page.refresh()
pump(2)
check("the phone is listed, nearby", any(d.get("online") and d["name"] == "OnePlus 6" for d in link.devices()), str(link.devices()))
shot("5-device-listed.png")
phone_id = next((d["id"] for d in link.devices()), "")
me = link.identity()["id"]

# ---- the pop-ups ------------------------------------------------------------
from nexa_shell import linknotify  # noqa: E402

notifier = linknotify.Notifier()
pump(2, until=lambda: notifier.sock is not None)
phone("head -c 200000 /dev/urandom > /tmp/IMG_2041.jpg; (nexa-linkd send %s /tmp/IMG_2041.jpg --json > /tmp/send.json 2>&1 &) ; echo started" % me)
pump(10, until=lambda: find(notifier.stack, Gtk.Button, "Save") is not None)
save = find(notifier.stack, Gtk.Button, "Save")
title = find(notifier.stack, Gtk.Label, "wants to send you a file")
check("a file offered: a pop-up with who, what and Save", save is not None and title is not None)
shot("6-popup-offer.png")
if save:
    save.clicked()
pump(10, until=lambda: os.path.exists(os.path.expanduser("~/Downloads/IMG_2041.jpg")))
check("Save: it arrives in Downloads", os.path.exists(os.path.expanduser("~/Downloads/IMG_2041.jpg")))
pump(3, until=lambda: find(notifier.stack, Gtk.Label, "Saved IMG_2041.jpg") is not None)
check("...and a pop-up says so, with Open", find(notifier.stack, Gtk.Label, "Saved IMG_2041.jpg") is not None
      and find(notifier.stack, Gtk.Button, "Open") is not None)
shot("7-popup-saved.png")
for c in notifier.stack.get_children():
    c.destroy()

phone("nexa-linkd text %s 'https://nexaos.example/notes' --json" % me)
pump(5, until=lambda: find(notifier.stack, Gtk.Button, "Copy") is not None)
check("a link from a paired device: shown with Copy and Open",
      find(notifier.stack, Gtk.Button, "Copy") is not None and find(notifier.stack, Gtk.Button, "Open") is not None)
shot("8-popup-link.png")
for c in notifier.stack.get_children():
    c.destroy()

phone("nexa-linkd ring %s" % me)
pump(5, until=lambda: find(notifier.stack, Gtk.Button, "Stop") is not None)
stop = find(notifier.stack, Gtk.Button, "Stop")
check("Ring: “OnePlus 6 is looking for this device”, with Stop",
      stop is not None and find(notifier.stack, Gtk.Label, "is looking for this device") is not None)
shot("9-popup-ring.png")
if stop:
    stop.clicked()
pump(0.5)
check("Stop closes it", find(notifier.stack, Gtk.Button, "Stop") is None)

# ---- Files: Send to a Device -------------------------------------------------
phone("nexa-linkd allow %s always" % me)
check("Send to a Device lists the phone", (phone_id, "OnePlus 6") in linkui.targets(), str(linkui.targets()))
os.makedirs(os.path.expanduser("~/Documents"), exist_ok=True)
doc = os.path.expanduser("~/Documents/report.pdf")
open(doc, "wb").write(b"%PDF-1.4 test")
said = []
linkui.send_files(phone_id, "OnePlus 6", [doc], lambda t, error=False: said.append(t))
pump(15, until=lambda: any(t.startswith("Sent report.pdf") for t in said))
got = phone("cat /root/Downloads/report.pdf")
check("...and sends the file to it", got.startswith("%PDF-1.4 test"), f"{said} / {got[:30]!r}")

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
