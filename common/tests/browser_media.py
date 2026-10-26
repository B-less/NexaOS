"""Opens the media test page in the real Nexa Browser window and prints
what played. Run inside the system image (scripts/test-browser-media.sh)."""
import json
import os
import sys

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk  # noqa: E402

from nexa_browser.window import BrowserWindow  # noqa: E402

win = BrowserWindow(sys.argv[1])
win.show_all()
out = {}


def poll():
    title = win.webview.get_title() or ""
    if title.startswith("RESULTS:"):
        out.update(json.loads(title[len("RESULTS:"):]))
        shot = os.environ.get("SHOT")
        if shot:
            os.system(f"grim {shot}")
        Gtk.main_quit()
        return False
    return True


def report():
    wv = win.webview
    print(f"... uri={wv.get_uri()} title={wv.get_title()!r} loading={wv.is_loading()}", flush=True)
    return True


def give_up():
    shot = os.environ.get("SHOT")
    if shot:
        os.system(f"grim {shot}")
    # What the page itself logged so far.
    def got(wv, res):
        try:
            v = wv.run_javascript_finish(res).get_js_value()
            print("page log:", v.to_string()[:3000])
        except Exception as e:
            print("page log unavailable:", e)
        Gtk.main_quit()
    win.webview.run_javascript("document.getElementById('log') ? document.getElementById('log').innerText : document.body.innerText",
                               None, got)
    return False


GLib.timeout_add(250, poll)
GLib.timeout_add_seconds(10, report)
GLib.timeout_add_seconds(90, give_up)
Gtk.main()
print(json.dumps(out, indent=1) if out else "NO RESULTS (page never finished)")
