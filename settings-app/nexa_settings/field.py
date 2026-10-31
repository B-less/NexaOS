"""Settings' front page: the Living Field (docs/LIVING_INTERFACE.md).

The computer as the things it's made of: each object a hexagon cell with a
live one-line summary and a state shape (● fine, ▲ needs attention, ■ off),
in clusters that never move, joined by threads to what they affect. Select
a cell: it opens beside the field with its real controls; "All settings"
goes to the full page (the sidebar is the list view and reaches
everything too). Arrow keys move between cells by where they are, typing
searches, Escape steps back.
"""
import json
import math
import os
import subprocess
import threading
import time

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, GLib, Gtk, Pango

from nexa_common import brain, link, patterns, sysconf, themes

REFRESH_S = 5
PREFS = os.path.expanduser("~/.config/nexa/field.json")

CLUSTERS = [("pinned", "Pinned"), ("system", "Your system"), ("connect", "Connections"),
            ("you", "You"), ("work", "Work"), ("safety", "Safety")]

# key, name, cluster, icon, full page (Settings page or "app:<id>"), related, search words
OBJECTS = [
    ("performance", "Performance", "system", "nexa-pulse", "app:nexa-pulse", ("storage", "power", "intelligence"),
     "speed slow memory ram cpu processor mode"),
    ("storage", "Storage", "system", "drive-harddisk", "app:nexa-files", ("performance",), "disk space files trash free"),
    ("power", "Power", "system", "nexa-power", "Power", ("performance",), "battery sleep lock screen charge"),
    ("updates", "Updates", "system", "system-software-update", "Updates", ("security",), "update upgrade alpine version"),
    ("display", "Display", "system", "video-display", "Display", ("power",), "screen monitor resolution scale night light rotate"),
    ("sound", "Sound", "system", "audio-speakers", "Sound", ("bluetooth",), "volume speakers headphones microphone audio"),
    ("network", "Network", "connect", "nexa-network", "Wi-Fi", ("nexa", "security"), "wifi wi-fi internet wired"),
    ("bluetooth", "Bluetooth", "connect", "bluetooth", "Bluetooth", (), "headphones speaker mouse pair"),
    ("nexa", "Nexa Link", "connect", "nexa-network", "Nexa Link", ("network",), "phone pair drop share devices"),
    ("appearance", "Appearance", "you", "nexa-settings", "Personalization", (), "theme colour color wallpaper glass dark icons"),
    ("privacy", "Privacy & Learning", "you", "nexa-privacy", "Privacy", ("intelligence",), "patterns learning habits forget"),
    ("accounts", "Accounts", "you", "nexa-settings", "Users", (), "users password people"),
    ("time", "Time & Keyboard", "you", "nexa-clock", "Time & Keyboard", (), "clock date timezone keyboard layout"),
    ("workspaces", "Workspaces", "work", "view-grid-symbolic", None, ("intelligence",), "workspace project apps restore save"),
    ("apps", "Apps", "work", "nexa-store", "app:nexa-store-app", (), "store install software"),
    ("intelligence", "Intelligence", "work", "nexa-ai", None, ("performance", "privacy", "workspaces"), "microbrain agents automation ai"),
    ("security", "Security", "safety", "nexa-privacy", None, ("network", "updates"), "secure boot tpm firewall protection"),
]
BY_KEY = {o[0]: o for o in OBJECTS}
SHAPES = {"ok": "●", "warn": "▲", "off": "■", "unknown": "○"}
CELL_W, CELL_H = 150, 128


def load_prefs():
    try:
        with open(PREFS) as f:
            data = json.load(f)
        return {"pinned": [k for k in data.get("pinned", []) if k in BY_KEY]}
    except (OSError, ValueError, AttributeError):
        return {"pinned": []}


def save_prefs(prefs):
    try:
        os.makedirs(os.path.dirname(PREFS), exist_ok=True)
        with open(PREFS + ".tmp", "w") as f:
            json.dump(prefs, f)
        os.replace(PREFS + ".tmp", PREFS)
    except OSError:
        pass


def _gb(b):
    return f"{b / 1024 ** 3:.1f} GB"


def _battery():
    base = "/sys/class/power_supply"
    try:
        for name in sorted(os.listdir(base)):
            p = os.path.join(base, name)
            if open(os.path.join(p, "type")).read().strip() == "Battery":
                cap = int(open(os.path.join(p, "capacity")).read().strip())
                status = open(os.path.join(p, "status")).read().strip()
                return cap, status
    except (OSError, ValueError):
        pass
    return None


def gather():
    """Every object's (summary, state, details), off the UI thread, each
    part on its own short leash: one slow service can't stall the field."""
    out = {}

    def put(key, fn):
        try:
            out[key] = fn()
        except Exception as e:  # a part that can't answer says so, the rest carry on
            out[key] = ("Not available", "unknown", {"error": str(e)})

    def performance():
        s = brain.request("status", timeout=1)
        sample = s.get("sample") or {}
        if not sample.get("mem_total_mb"):
            return ("Measuring\u2026", "unknown", s)
        used = (sample.get("mem_total_mb", 0) - sample.get("mem_avail_mb", 0)) / 1024
        total = sample.get("mem_total_mb", 0) / 1024
        mode = s.get("budget", {}).get("mode", "auto").replace("-", " ").capitalize()
        state = "warn" if s.get("memory") in ("High", "Critical") or s.get("cpu") == "Critical" else "ok"
        return (f"{used:.1f} of {total:.1f} GB used · {mode}", state, s)

    def storage():
        st = os.statvfs(os.path.expanduser("~"))
        free, total = st.f_bavail * st.f_frsize, st.f_blocks * st.f_frsize
        state = "warn" if total and free / total < 0.08 else "ok"
        return (f"{_gb(free)} free of {_gb(total)}", state, {"free": free, "total": total})

    def power():
        b = _battery()
        if not b:
            return ("Mains power", "ok", {})
        cap, status = b
        state = "warn" if cap <= 15 and status == "Discharging" else "ok"
        return (f"Battery {cap} % · {status.lower()}", state, {"capacity": cap, "status": status})

    def updates():
        try:
            version = open("/etc/alpine-release").read().strip()
        except OSError:
            version = "?"
        return (f"Built on Alpine {version}", "unknown", {})

    def display():
        from nexa_common import displays
        on = [s for s in displays.screens() if s["enabled"]]
        if not on:
            return ("No screen", "unknown", {})
        m = on[0].get("mode") or {}
        size = f"{m.get('width')} × {m.get('height')}" if m else on[0]["name"]
        text = size + (f" · {round(on[0]['scale'] * 100)} %" if on[0]["scale"] != 1 else "")
        if len(on) > 1:
            text = f"{len(on)} screens · " + text
        night = displays.load_night()["enabled"]
        return (text + (" · night light" if night else ""), "ok", {"screens": on})

    def sound_state():
        from nexa_common import sound
        outs = sound.outputs()
        d = next((o for o in outs if o["default"]), None)
        if not d:
            return ("No speakers", "off", {})
        return (f"{d['description'][:28]} · {'muted' if d['mute'] else str(d['volume']) + ' %'}",
                "off" if d["mute"] else "ok", d)

    def network():
        from nexa_common import wifi
        try:
            ssid = wifi.status().get("connected_to")
        except Exception:
            ssid = None
        wired = []
        for n in sorted(os.listdir("/sys/class/net")):
            p = f"/sys/class/net/{n}"
            try:
                if (open(f"{p}/type").read().strip() == "1" and not os.path.isdir(f"{p}/wireless")
                        and open(f"{p}/operstate").read().strip() == "up"):
                    wired.append(n)
            except OSError:
                continue
        if ssid:
            return (f"Wi-Fi: {ssid}", "ok", {"ssid": ssid})
        if wired:
            return ("Wired", "ok", {"wired": wired})
        return ("Offline", "off", {})

    def bluetooth():
        from nexa_common import bluetooth as bt
        a = bt.adapter()
        if not a:
            return ("No adapter", "off", {})
        return ("On" if a["powered"] else "Off", "ok" if a["powered"] else "off", a)

    def nexa():
        devs = link.devices()
        near = sum(1 for d in devs if d.get("online"))
        drop = link.status().get("drop_open")
        text = f"{len(devs)} paired · {near} nearby" + (" · taking drops" if drop else "")
        return (text, "ok", {"devices": devs, "drop": drop})

    def appearance():
        s = themes.load_settings()
        return (f"{themes.THEMES[s['theme']]['label']} · glass {'on' if s.get('glass') else 'off'}", "ok", s)

    def privacy():
        p = patterns.load_config()
        try:
            habits = brain.request("config", timeout=1)["config"]["features"]["habits"]
        except (brain.BrainError, KeyError):
            habits = False
        on = [n for n, v in (("Patterns", p.get("enabled")), ("habit learning", habits)) if v]
        return ("Learning: " + (", ".join(on) if on else "off"), "ok", {"patterns": p.get("enabled"), "habits": habits})

    def accounts():
        import pwd
        from nexa_common import sessions
        me = pwd.getpwuid(os.getuid()).pw_name
        others = sessions.others()
        signed = [p["display"] for p in others if p["signed_in"]]
        if signed:
            return (f"{me} · {', '.join(signed)} also signed in", "ok", {"others": others})
        if others:
            return (f"{me} · {len(others) + 1} people can sign in", "ok", {"others": others})
        return (me, "ok", {"others": []})

    def clock():
        try:
            layout = sysconf.read_keyboard_layout()
        except Exception:
            layout = "?"
        return (f"{time.strftime('%H:%M')} · keyboard {layout}", "ok", {})

    def workspaces():
        ws = brain.request("workspaces", timeout=1).get("workspaces", [])
        return (f"{len(ws)} saved" if ws else "None saved yet", "ok", {"workspaces": ws})

    def apps():
        count = 0
        for d in ("/usr/share/applications", os.path.expanduser("~/.local/share/applications")):
            try:
                count += sum(1 for f in os.listdir(d) if f.endswith(".desktop"))
            except OSError:
                pass
        return (f"{count} installed", "ok", {})

    def intelligence():
        s = brain.request("status", timeout=1)
        me = s.get("self", {})
        on = sum(1 for v in (s.get("features") or {}).values() if v)
        return (f"{on} features on · {me.get('pss_mb') or 0:.1f} MB", "ok" if s.get("enabled") else "off", s)

    def security():
        r = subprocess.run(["nexa-doctor", "security"], capture_output=True, text=True, timeout=4)
        data = (json.loads(r.stdout or "{}") or {}).get("result") or {}
        sb = str(data.get("secure_boot", "unknown"))
        tpm = (data.get("tpm") or {}).get("summary", "")
        problems = (data.get("keys") or {}).get("problems") or []
        if problems:
            # Cleared keys or Windows' key gone: Windows won't start.
            return (problems[0]["title"], "warn", {"secure_boot": sb, "tpm": tpm, "problem": problems[0]})
        state = "ok" if "on" in sb.lower() and "off" not in sb.lower() else "warn"
        return (f"Secure Boot: {sb.split('(')[0].strip()}", state, {"secure_boot": sb, "tpm": tpm})

    for key, fn in (("performance", performance), ("storage", storage), ("power", power), ("updates", updates),
                    ("display", display), ("sound", sound_state),
                    ("network", network), ("bluetooth", bluetooth), ("nexa", nexa), ("appearance", appearance),
                    ("privacy", privacy), ("accounts", accounts), ("time", clock), ("workspaces", workspaces),
                    ("apps", apps), ("intelligence", intelligence), ("security", security)):
        put(key, fn)
    return out


class Cell(Gtk.Button):
    """One object: a hexagon with its name, live summary and state shape."""

    def __init__(self, field, key):
        super().__init__()
        self.field, self.key = field, key
        self.obj = BY_KEY[key]
        self.state, self.summary = "unknown", "…"
        self.lit = self.dim = False
        self.set_relief(Gtk.ReliefStyle.NONE)
        self.get_style_context().add_class("lf-cell")
        self.set_size_request(CELL_W, CELL_H)
        overlay = Gtk.Overlay()
        area = Gtk.DrawingArea()
        area.set_has_window(False)  # clicks belong to the button
        area.connect("draw", self._draw)
        overlay.add(area)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, valign=Gtk.Align.CENTER, halign=Gtk.Align.CENTER)
        icon = Gtk.Image.new_from_icon_name(self.obj[3], Gtk.IconSize.LARGE_TOOLBAR)
        icon.set_pixel_size(28)
        box.pack_start(icon, False, False, 0)
        head = Gtk.Box(spacing=4, halign=Gtk.Align.CENTER)
        self.shape = Gtk.Label()
        self.shape.get_style_context().add_class("lf-shape")
        head.pack_start(self.shape, False, False, 0)
        name = Gtk.Label(label=self.obj[1])
        name.get_style_context().add_class("lf-name")
        head.pack_start(name, False, False, 0)
        box.pack_start(head, False, False, 0)
        self.sub = Gtk.Label(label="…", justify=Gtk.Justification.CENTER)
        self.sub.set_line_wrap(True)
        self.sub.set_max_width_chars(16)
        self.sub.set_lines(2)
        self.sub.set_ellipsize(Pango.EllipsizeMode.END)
        self.sub.get_style_context().add_class("lf-sub")
        box.pack_start(self.sub, False, False, 0)
        overlay.add_overlay(box)
        overlay.set_overlay_pass_through(box, True)
        self.add(overlay)
        self.area = area
        self.connect("clicked", lambda *_a: field.open(key))
        # Arrows first here: the wrapping container would otherwise take them.
        self.connect("key-press-event", lambda _w, ev: field._on_key(self, ev))
        self.connect("focus-in-event", lambda *_a: field.focus_changed(key))

    def update(self, summary, state):
        self.summary, self.state = summary, state
        self.sub.set_text(summary)
        self.shape.set_text(SHAPES.get(state, SHAPES["unknown"]))
        for s in SHAPES:
            self.shape.get_style_context().remove_class("lf-" + s)
        self.shape.get_style_context().add_class("lf-" + state)
        words = {"ok": "fine", "warn": "needs attention", "off": "off", "unknown": "state unknown"}[state]
        self.get_accessible().set_name(f"{self.obj[1]}: {summary}, {words}")
        self.set_tooltip_text(f"{self.obj[1]}: {summary}")
        self.area.queue_draw()

    def _draw(self, area, cr):
        w, h = area.get_allocated_width(), area.get_allocated_height()
        r = min(w / 2, h / math.sqrt(3)) - 3
        cx, cy = w / 2, h / 2
        cr.new_path()
        for i in range(6):  # flat-top hexagon: wide cells read well in rows
            a = i * math.pi / 3
            cr.line_to(cx + r * math.cos(a), cy + r * math.sin(a) * 0.86)
        cr.close_path()
        pal = self.field.colours
        sel = self.field.selected == self.key
        alpha = 0.25 if self.dim else 1.0
        cr.set_source_rgba(*pal["surface"], (0.85 if sel else 0.6) * alpha)
        cr.fill_preserve()
        if sel:
            cr.set_source_rgba(*pal["accent"], 0.95)
            cr.set_line_width(2.5)
        elif self.lit:
            cr.set_source_rgba(*pal["accent2"], 0.8)
            cr.set_line_width(1.8)
        else:
            cr.set_source_rgba(*pal["accent"], 0.28 * alpha)
            cr.set_line_width(1.0)
        cr.stroke()
        return False


def draw_threads(f, area, cr):
    """The lines between related objects, drawn on the field itself before
    its cells (so they run under them)."""
    if True:
        pal = f.colours
        drawn = set()
        for key, cell in f.cells.items():
            for other in BY_KEY[key][5]:
                pair = tuple(sorted((key, other)))
                if pair in drawn or other not in f.cells:
                    continue
                drawn.add(pair)
                a = cell.translate_coordinates(area, CELL_W // 2, CELL_H // 2)
                b = f.cells[other].translate_coordinates(area, CELL_W // 2, CELL_H // 2)
                if not a or not b:
                    continue
                hot = f.selected in pair
                cr.set_source_rgba(*(pal["accent2"] if hot else pal["accent"]), 0.85 if hot else 0.16)
                cr.set_line_width(2.0 if hot else 1.0)
                cr.move_to(*a)
                # A gentle curve: threads read as connections, not a grid.
                mx, my = (a[0] + b[0]) / 2, (a[1] + b[1]) / 2 - 24
                cr.curve_to(mx, my, mx, my, *b)
                cr.stroke()
        return False


class LivingField(Gtk.Box):
    def __init__(self, win):
        super().__init__(orientation=Gtk.Orientation.HORIZONTAL, spacing=0)
        self.win = win
        self.prefs = load_prefs()
        self.selected = None
        self.cells = {}
        self.data = {}
        self._load_colours()
        left = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        left.set_border_width(6)
        title = Gtk.Label(label="Your computer", xalign=0)
        title.set_name("pagetitle")
        left.pack_start(title, False, False, 0)
        hint = Gtk.Label(label="Everything NexaOS is made of. Select something to change it here; "
                               "the list on the left has every setting too.", xalign=0)
        hint.set_name("fieldlabel")
        hint.set_line_wrap(True)
        left.pack_start(hint, False, False, 0)
        self.search = Gtk.SearchEntry(placeholder_text="Find a setting, or say what you want (“dim the screen”)")
        self.search.connect("search-changed", lambda *_a: self._filter())
        self.search.connect("activate", lambda *_a: self._search_go())
        left.pack_start(self.search, False, False, 0)
        # Clusters in reading order, wrapping into rows when the window is
        # narrow; their order never changes.
        self.grid = Gtk.FlowBox(selection_mode=Gtk.SelectionMode.NONE, homogeneous=False,
                                min_children_per_line=1, max_children_per_line=6,
                                column_spacing=18, row_spacing=12, valign=Gtk.Align.START)
        self.grid.set_border_width(6)
        self.grid.connect("draw", lambda area, cr: draw_threads(self, area, cr))  # before the cells
        scroller = Gtk.ScrolledWindow()
        scroller.set_policy(Gtk.PolicyType.AUTOMATIC, Gtk.PolicyType.AUTOMATIC)
        scroller.set_min_content_width(240)
        scroller.add(self.grid)
        left.pack_start(scroller, True, True, 0)
        self.pack_start(left, True, True, 0)
        self.panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.panel.set_border_width(14)
        self.panel.set_size_request(320, -1)
        self.panel.get_style_context().add_class("lf-panel")
        self.panel.set_no_show_all(True)
        self.pack_start(self.panel, False, False, 0)
        self._build_clusters()
        self.connect("key-press-event", self._on_key)
        self.connect("map", lambda *_a: self.refresh())
        GLib.timeout_add_seconds(REFRESH_S, self._tick)

    # ---- layout -----------------------------------------------------------
    def _load_colours(self):
        t = themes.THEMES[themes.load_settings()["theme"]]

        def rgb(h):
            h = h.lstrip("#")
            return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))
        surface = t["surface"] if isinstance(t["surface"], tuple) else (20, 28, 58)
        self.colours = {"accent": rgb(t["accent"]), "accent2": rgb(t["accent2"]),
                        "surface": tuple(c / 255 for c in surface)}

    def _build_clusters(self):
        for child in self.grid.get_children():
            child.destroy()
        self.cells = {}
        for ckey, cname in CLUSTERS:
            keys = self.prefs["pinned"] if ckey == "pinned" else [o[0] for o in OBJECTS if o[2] == ckey]
            if not keys:
                continue
            col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
            head = Gtk.Label(label=cname.upper(), xalign=0)
            head.get_style_context().add_class("lf-cluster")
            col.pack_start(head, False, False, 6)
            for i, key in enumerate(keys):
                if key in self.cells:
                    continue  # pinned ones aren't repeated in their own cluster
                cell = Cell(self, key)
                # Honeycomb: every other cell half a cell to the right.
                cell.set_margin_start(CELL_W // 2 if i % 2 else 0)
                cell.set_margin_end(0 if i % 2 else CELL_W // 2)
                col.pack_start(cell, False, False, 0)
                self.cells[key] = cell
            self.grid.add(col)
            col.get_parent().set_can_focus(False)  # the cells take focus, not the wrapper
        for key, (summary, state, _d) in self.data.items():
            if key in self.cells:
                self.cells[key].update(summary, state)
        self.grid.show_all()
        self.grid.queue_draw()

    # ---- data -------------------------------------------------------------
    def _tick(self):
        if self.get_mapped():
            self.refresh()
        return True

    def refresh(self):
        def work():
            data = gather()
            GLib.idle_add(self._show, data)
        threading.Thread(target=work, daemon=True).start()

    def _show(self, data):
        self.data = data
        for key, (summary, state, _d) in data.items():
            if key in self.cells:
                self.cells[key].update(summary, state)
        if self.selected and self.panel.get_visible():
            self._fill_panel(self.selected, keep=True)
        return False

    # ---- selection, search, keyboard ----------------------------------------
    def focus_changed(self, key):
        if self.selected != key and not self.panel.get_visible():
            self.selected = key
            self._light()

    def _light(self):
        related = set(BY_KEY[self.selected][5]) if self.selected else set()
        related |= {k for k, o in BY_KEY.items() if self.selected in o[5]}
        for k, c in self.cells.items():
            c.lit = k in related
            c.area.queue_draw()
        self.grid.queue_draw()

    def open(self, key):
        self.selected = key
        self._light()
        self._fill_panel(key)

    def close_panel(self):
        self.panel.hide()
        if self.selected in self.cells:
            self.cells[self.selected].grab_focus()

    def _filter(self):
        q = self.search.get_text().strip().lower()
        for k, c in self.cells.items():
            o = BY_KEY[k]
            c.dim = bool(q) and not any(w in f"{o[1]} {o[6]}".lower() for w in q.split())
            c.area.queue_draw()

    def _search_go(self):
        q = self.search.get_text().strip()
        match = [k for k, c in self.cells.items() if not c.dim]
        if q and match:
            self.cells[match[0]].grab_focus()
            self.open(match[0])
            return
        if q:  # not an object: maybe a command ("dim the screen")
            try:
                hits = brain.ask(q).get("hits", [])
            except brain.BrainError:
                hits = []
            if hits and not hits[0].get("confirm"):
                text = brain.run(hits[0]["action"]) or f"Done: {hits[0]['title']}."
                self._message(text)

    def _message(self, text):
        self.selected = None
        self._light()
        self._panel_reset("Nexa")
        label = Gtk.Label(label=text, xalign=0)
        label.set_line_wrap(True)
        self.panel.pack_start(label, False, False, 0)
        self._show_panel()

    def _on_key(self, _w, event):
        key = event.keyval
        if key == Gdk.KEY_Escape:
            if self.panel.get_visible():
                self.close_panel()
            elif self.search.get_text():
                self.search.set_text("")
            return True
        moves = {Gdk.KEY_Left: (-1, 0), Gdk.KEY_Right: (1, 0), Gdk.KEY_Up: (0, -1), Gdk.KEY_Down: (0, 1)}
        if key in moves and not self.search.has_focus():
            self._move(*moves[key])
            return True
        if not self.search.has_focus() and event.string and event.string.isprintable() and event.string.strip():
            self.search.grab_focus_without_selecting()
            return self.search.event(event)
        return False

    def _move(self, dx, dy):
        """To the nearest cell in that direction, by position on screen."""
        cur = self.cells.get(self.selected) or next(iter(self.cells.values()))
        a = cur.get_allocation()
        ox, oy = cur.translate_coordinates(self.grid, 0, 0) or (a.x, a.y)
        best, best_d = None, None
        for k, c in self.cells.items():
            if c is cur:
                continue
            x, y = c.translate_coordinates(self.grid, 0, 0) or (0, 0)
            vx, vy = x - ox, y - oy
            if vx * dx + vy * dy <= 0:
                continue
            along = abs(vx * dx + vy * dy)
            across = abs(vx * dy - vy * dx)
            d = along + 2.5 * across
            if best_d is None or d < best_d:
                best, best_d = k, d
        if best:
            # Selection follows the arrow itself, not the focus-in event
            # (which only comes while the window has the keyboard).
            self.selected = best
            self._light()
            self.cells[best].grab_focus()

    # ---- the open panel (NexaMorph) ------------------------------------------
    def _panel_reset(self, title):
        for child in self.panel.get_children():
            child.destroy()
        head = Gtk.Box(spacing=8)
        t = Gtk.Label(label=title, xalign=0)
        t.set_name("pagetitle")
        head.pack_start(t, True, True, 0)
        close = Gtk.Button.new_from_icon_name("window-close-symbolic", Gtk.IconSize.BUTTON)
        close.set_relief(Gtk.ReliefStyle.NONE)
        close.set_tooltip_text("Close (Escape)")
        close.get_accessible().set_name("Close")
        close.connect("clicked", lambda *_a: self.close_panel())
        head.pack_end(close, False, False, 0)
        self.panel.pack_start(head, False, False, 0)

    def _text(self, text, name="value"):
        label = Gtk.Label(label=text, xalign=0)
        label.set_name(name)
        label.set_line_wrap(True)
        label.set_max_width_chars(40)
        self.panel.pack_start(label, False, False, 0)
        return label

    def _button(self, label, fn, primary=False):
        b = Gtk.Button(label=label)
        if primary:
            b.set_name("actionbtn")
        b.connect("clicked", lambda *_a: fn())
        self.panel.pack_start(b, False, False, 0)
        return b

    def _switch(self, label, active, fn):
        row = Gtk.Box(spacing=10)
        row.pack_start(Gtk.Label(label=label, xalign=0), True, True, 0)
        sw = Gtk.Switch(active=bool(active), valign=Gtk.Align.CENTER)
        sw.get_accessible().set_name(label)
        sw.connect("notify::active", lambda s, _p: fn(s.get_active()))
        row.pack_end(sw, False, False, 0)
        self.panel.pack_start(row, False, False, 0)
        return sw

    def _act(self, fn, done=None):
        """Run fn off the UI thread; then refresh and show its message."""
        def work():
            try:
                msg = fn() or done
            except Exception as e:  # shown, not raised into GTK
                msg = str(e)
            GLib.idle_add(self._after, msg)
        threading.Thread(target=work, daemon=True).start()

    def _after(self, msg):
        if msg:
            self.status = getattr(self, "status", None)
            if self.status and self.status.get_parent() is self.panel:
                self.status.set_text(msg)
            else:
                self.status = self._text(msg, "fieldlabel")
                self.status.show()
        self.refresh()
        return False

    def _portal(self, key):
        page = BY_KEY[key][4]
        if not page:
            return
        if page.startswith("app:"):
            try:
                brain.launch(page[4:])
            except brain.BrainError as e:
                self._after(str(e))
            return
        self.win._on_nav(None, page)

    def _pin_button(self, key):
        pinned = key in self.prefs["pinned"]

        def toggle():
            if key in self.prefs["pinned"]:
                self.prefs["pinned"].remove(key)
            else:
                self.prefs["pinned"].append(key)
            save_prefs(self.prefs)
            self._build_clusters()
            self.open(key)
        b = Gtk.Button(label="Unpin" if pinned else "Pin to the start")
        b.connect("clicked", lambda *_a: toggle())
        return b

    def _fill_panel(self, key, keep=False):
        summary, state, d = self.data.get(key, ("…", "unknown", {}))
        o = BY_KEY[key]
        self._panel_reset(o[1])
        self.status = None
        self._text(summary)
        if d.get("error"):
            self._text(d["error"], "fieldlabel")
        getattr(self, "_fill_" + key, lambda _d: None)(d)
        foot = Gtk.Box(spacing=8)
        if o[4]:
            more = Gtk.Button(label="All settings" if not o[4].startswith("app:") else "Open the app")
            more.connect("clicked", lambda *_a: self._portal(key))
            foot.pack_start(more, False, False, 0)
        foot.pack_end(self._pin_button(key), False, False, 0)
        self.panel.pack_end(foot, False, False, 0)
        related = [BY_KEY[r][1] for r in o[5]]
        if related:
            self.panel.pack_end(Gtk.Label(label="Connected to: " + ", ".join(related), xalign=0, name="fieldlabel"), False, False, 0)
        self._show_panel()

    def _show_panel(self):
        # The panel is kept out of the window's show_all(); its own
        # show_all() then does nothing, so show what's in it, then it.
        for child in self.panel.get_children():
            child.show_all()
        self.panel.show()

    # Each object's real controls.
    def _fill_performance(self, d):
        combo = Gtk.ComboBoxText()
        for mode, label in (("auto", "Automatic"), ("balanced", "Balanced"), ("low-resource", "Low resource"),
                            ("battery-saver", "Battery saver"), ("performance", "Performance")):
            combo.append(mode, label)
        mode = d.get("mode") if isinstance(d.get("mode"), str) else "auto"
        combo.set_active_id(mode)
        combo.get_accessible().set_name("Resource mode")
        combo.connect("changed", lambda c: self._act(lambda: brain.run({"type": "mode", "mode": c.get_active_id()})))
        self.panel.pack_start(Gtk.Label(label="How much background work NexaOS allows", xalign=0, name="fieldlabel"), False, False, 0)
        self.panel.pack_start(combo, False, False, 0)
        if d.get("budget", {}).get("why"):
            self._text(d["budget"]["why"].capitalize() + ".", "fieldlabel")
        self._button("Why is it slow?", lambda: self._act(lambda: brain.run({"type": "answer", "topic": "slow"})))

    def _fill_storage(self, d):
        if d.get("total"):
            bar = Gtk.LevelBar(min_value=0, max_value=1)
            bar.set_value(1 - d["free"] / d["total"])
            bar.get_accessible().set_name("Storage used")
            self.panel.pack_start(bar, False, False, 0)
        self._button("Open the Trash", lambda: brain.run({"type": "open_trash"}))

    def _fill_power(self, d):
        self._button("Battery health", lambda: self._act(lambda: brain.run({"type": "answer", "topic": "battery"})))
        self._button("Lock the screen", lambda: brain.run({"type": "lock"}))

    def _fill_bluetooth(self, d):
        if "powered" in d:
            from nexa_common import bluetooth as bt
            self._switch("Bluetooth", d["powered"], lambda on: self._act(lambda: bt.set_powered(on), "Bluetooth is on." if on else "Bluetooth is off."))

    def _fill_nexa(self, d):
        self._switch("Nexa Drop for 10 minutes", d.get("drop"), lambda on: self._act(lambda: link.set_drop(10 if on else 0),
                                                                                       "Nexa Drop is on." if on else "Nexa Drop is off."))

    def _fill_appearance(self, d):
        row = Gtk.FlowBox(selection_mode=Gtk.SelectionMode.NONE, max_children_per_line=3)
        for key, t in themes.THEMES.items():
            b = Gtk.Button(label=t["label"])
            b.set_tooltip_text(t.get("tagline", ""))
            if key == d.get("theme"):
                b.set_name("actionbtn")
            b.connect("clicked", lambda _b, k=key: self._act(lambda: (themes.apply(k), self._load_colours())[0] and None,
                                                              f"{themes.THEMES[k]['label']} theme applied."))
            row.add(b)
        self.panel.pack_start(row, False, False, 0)
        self._switch("Glass windows", d.get("glass"), lambda on: self._act(lambda: themes.apply(glass=on) and None))

    def _fill_privacy(self, d):
        self._switch("Nexa Patterns (learns which apps you use when)", d.get("patterns"),
                     lambda on: self._act(lambda: patterns.update_config(enabled=on) and None))
        self._switch("Habit learning (apps you open together)", d.get("habits"),
                     lambda on: self._act(lambda: brain.request("config", feature="habits", on=on) and None))
        self._text("Both stay on this computer. Forget everything in Privacy.", "fieldlabel")

    def _fill_workspaces(self, d):
        for ws in d.get("workspaces", [])[:8]:
            self._button(f"Restore “{ws['name']}”", lambda n=ws["name"]: self._act(
                lambda: "; ".join(brain.restore_workspace(n)) or f"“{n}” restored."))
        entry = Gtk.Entry(placeholder_text="Name for the apps open now")
        entry.get_accessible().set_name("New workspace name")
        self.panel.pack_start(entry, False, False, 0)
        self._button("Save as a workspace", lambda: self._act(
            lambda: f"Saved “{brain.save_workspace(entry.get_text())['workspace']['name']}”."), primary=True)

    def _fill_accounts(self, d):
        from nexa_common import sessions
        for p in d.get("others", [])[:4]:
            label = f"Switch to {p['display']}" + (" (signed in)" if p["signed_in"] else "")
            self._button(label, lambda n=p["name"]: sessions.switch_to(n))
        if d.get("others"):
            self._text("Your desktop locks and stays as it is; theirs asks for their password.", "fieldlabel")

    def _fill_apps(self, d):
        self._button("Open Nexus", lambda: brain.run({"type": "launch", "app": "nexa-nexus"}))

    def _fill_intelligence(self, d):
        feats = d.get("features") or {}
        names = {"futureload": "Pressure forecasts", "pulse": "Resource modes", "search": "Commands in Nexus",
                 "workspaces": "Workspaces", "habits": "Habit learning", "freeze": "Pause background apps"}
        for k, label in names.items():
            if k in feats:
                self._switch(label, feats[k], lambda on, k=k: self._act(lambda: brain.request("config", feature=k, on=on) and None))
        try:
            decisions = brain.request("decisions", limit=4, timeout=1).get("decisions", [])
        except brain.BrainError:
            decisions = []
        for dec in decisions:
            self._text(f"{dec['agent']}: {dec['what']} ({dec['why']})", "fieldlabel")

    def _fill_security(self, d):
        if d.get("tpm"):
            self._text(d["tpm"], "fieldlabel")
        self._text("Firewall, sandboxing and the broker are part of NexaOS and always on.", "fieldlabel")


CSS = b"""
.lf-cell { background: none; border: none; box-shadow: none; padding: 0; }
.lf-cell:focus { outline: 2px solid @nexa_accent; outline-offset: -6px; border-radius: 12px; }
.lf-name { font-weight: bold; color: @nexa_text_strong; }
.lf-sub { font-size: 8.5pt; color: @nexa_muted; }
.lf-cluster { font-size: 8pt; font-weight: bold; letter-spacing: 2px; color: @nexa_muted; }
.lf-shape { font-size: 9pt; }
.lf-ok { color: #5ee6a8; }
.lf-warn { color: #ffb547; }
.lf-off { color: @nexa_muted; }
.lf-unknown { color: @nexa_muted; }
.lf-panel { background-color: alpha(@nexa_surface, 0.6); border-left: 1px solid alpha(@nexa_accent, 0.3); }
"""
