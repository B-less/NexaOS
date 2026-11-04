"""Settings > Display, Sound, Mouse & Touchpad, Wi-Fi, Notifications,
Default Apps and Accessibility. Each page is a thin view: the work is done
by nexa-display (C), PipeWire (pactl), the compositor, iwd, mako, GIO and
nexa_common's *conf modules. Slow calls run off the GTK thread."""
import os
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gio, GLib, Gtk, Pango

from nexa_common import accessconf, displays, inputconf, notifyconf, sound, wifi


def _bg(work, done):
    """work() on a thread; done(result, error) back on the GTK thread."""
    def run():
        try:
            result, error = work(), None
        except Exception as e:  # each page shows what went wrong
            result, error = None, str(e) or e.__class__.__name__
        GLib.idle_add(lambda: done(result, error) and False)
    threading.Thread(target=run, daemon=True).start()


def _label(text, dim=False, wrap=True, xalign=0):
    lbl = Gtk.Label(label=text, xalign=xalign)
    lbl.set_line_wrap(wrap)
    lbl.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
    if dim:
        lbl.get_style_context().add_class("dim-label")
    return lbl


def _row(title, widget, subtitle=None):
    """A settings row: words on the left, the control on the right."""
    row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
    words = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
    words.pack_start(_label(title), False, False, 0)
    if subtitle:
        words.pack_start(_label(subtitle, dim=True), False, False, 0)
    row.pack_start(words, True, True, 0)
    widget.set_valign(Gtk.Align.CENTER)
    row.pack_start(widget, False, False, 0)
    return row


def _switch(active, on_change):
    sw = Gtk.Switch()
    sw.set_active(active)
    sw.connect("notify::active", lambda s, _p: on_change(s.get_active()))
    return sw


def _combo(items, active_id, on_change):
    """items: [(id, text)]."""
    c = Gtk.ComboBoxText()
    for i, t in items:
        c.append(str(i), t)
    c.set_active_id(str(active_id))
    c.connect("changed", lambda w: w.get_active_id() is not None and on_change(w.get_active_id()))
    return c


def _slider(lo, hi, value, on_change, step=1, width=220, digits=0):
    s = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, lo, hi, step)
    s.set_value(value)
    s.set_digits(digits)
    s.set_draw_value(False)
    s.set_size_request(width, -1)
    pending = {"id": 0}

    def changed(w):
        # Apply once the slider rests (not for every pixel of a drag).
        if pending["id"]:
            GLib.source_remove(pending["id"])
        pending["id"] = GLib.timeout_add(200, lambda: (pending.update(id=0), on_change(w.get_value()))[1] and False)
    s.connect("value-changed", changed)
    return s


class _Page:
    title = ""

    def __init__(self, win):
        self.win = win
        self.page = win._page(self.title)
        self.status = _label("", dim=True)

    def heading(self, text):
        self.page.pack_start(self.win._field_label(text), False, False, 0)

    def add(self, widget, pad=0):
        self.page.pack_start(widget, False, False, pad)

    def say(self, text, error=False):
        self.win._set_label(self.status, text, error)


# ---------------------------------------------------------------- Display

class DisplayPage(_Page):
    title = "Display"
    KEEP_SECONDS = 15

    def __init__(self, win):
        super().__init__(win)
        self.cards = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        self.add(self.cards)
        apply_btn = Gtk.Button(label="Apply")
        apply_btn.set_name("actionbtn")
        apply_btn.set_halign(Gtk.Align.START)
        apply_btn.connect("clicked", lambda _b: self.apply())
        self.apply_btn = apply_btn
        self.add(apply_btn)
        self.add(self.status)
        self._night()
        self.screens = []
        self.widgets = {}
        self.reload()

    def reload(self):
        self.say("Looking at the screens…")
        _bg(displays.screens, self._loaded)

    def _loaded(self, screens, error):
        for c in self.cards.get_children():
            self.cards.remove(c)
        if error:
            self.say(f"Screens can't be changed here: {error}", error=True)
            self.apply_btn.set_sensitive(False)
            return
        self.screens = screens
        self.widgets = {}
        for s in screens:
            self.cards.pack_start(self._card(s, screens), False, False, 0)
        self.cards.show_all()
        self.apply_btn.set_sensitive(True)
        self.say("")

    def _card(self, s, all_screens):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        box.pack_start(_label(f"{displays.describe(s)}  ({s['name']})"), False, False, 0)
        w = {}
        if len(all_screens) > 1:
            w["enabled"] = Gtk.Switch(active=s["enabled"])
            box.pack_start(_row("On", w["enabled"]), False, False, 0)
        res = displays.resolutions(s)
        cur = s.get("mode") or {}
        w["res"] = Gtk.ComboBoxText()
        preferred = {(m["width"], m["height"]) for m in s.get("modes", []) if m.get("preferred")}
        for width, height, _r in res:
            tag = "  (recommended)" if (width, height) in preferred else ""
            w["res"].append(f"{width}x{height}", f"{width} × {height}{tag}")
        w["res"].set_active_id(f"{cur.get('width')}x{cur.get('height')}")
        w["rate"] = Gtk.ComboBoxText()

        def fill_rates(*_a):
            w["rate"].remove_all()
            size = w["res"].get_active_id()
            for width, height, rates in res:
                if f"{width}x{height}" == size:
                    for r in rates:
                        w["rate"].append(str(r), f"{r / 1000:.2f} Hz" if r else "Default")
            if not w["rate"].set_active_id(str(cur.get("refresh"))):
                w["rate"].set_active(0)
        w["res"].connect("changed", fill_rates)
        fill_rates()
        box.pack_start(_row("Resolution", w["res"]), False, False, 0)
        box.pack_start(_row("Refresh rate", w["rate"]), False, False, 0)
        scales = sorted(set(displays.SCALES) | {round(s["scale"], 2)})
        w["scale"] = Gtk.ComboBoxText()
        for sc in scales:
            w["scale"].append(f"{sc:g}", f"{round(sc * 100)} %")
        w["scale"].set_active_id(f"{round(s['scale'], 2):g}")
        box.pack_start(_row("Scale", w["scale"], "Makes everything on this screen bigger"), False, False, 0)
        w["rot"] = Gtk.ComboBoxText()
        for t in displays.TRANSFORMS:
            w["rot"].append(t, displays.ROTATION_NAMES[t])
        if not w["rot"].set_active_id(s["transform"]):
            w["rot"].set_active_id("normal")
        box.pack_start(_row("Rotation", w["rot"]), False, False, 0)
        others = [o for o in all_screens if o["name"] != s["name"]]
        if others:
            w["place"] = Gtk.ComboBoxText()
            w["place"].append("keep", "Where it is now")
            for o in others:
                for side, text in (("right", "Right of"), ("left", "Left of"), ("above", "Above"), ("below", "Below")):
                    w["place"].append(f"{side}:{o['name']}", f"{text} {o['name']}")
            w["place"].set_active_id("keep")
            box.pack_start(_row("Position", w["place"]), False, False, 0)
        self.widgets[s["name"]] = w
        frame = Gtk.Frame()
        box.set_border_width(12)
        frame.add(box)
        return frame

    @staticmethod
    def _logical(s, head):
        w, h = head.get("width") or 0, head.get("height") or 0
        if head.get("transform") in ("90", "270"):
            w, h = h, w
        sc = head.get("scale") or 1
        return int(w / sc), int(h / sc)

    def wanted(self):
        heads = {}
        for s in self.screens:
            w = self.widgets.get(s["name"], {})
            head = {"name": s["name"]}
            if "enabled" in w:
                head["enabled"] = w["enabled"].get_active()
            size = (w["res"].get_active_id() or "").split("x")
            if len(size) == 2 and all(x.isdigit() for x in size):
                head["width"], head["height"] = int(size[0]), int(size[1])
            if w["rate"].get_active_id():
                head["refresh"] = int(w["rate"].get_active_id())
            head["scale"] = float(w["scale"].get_active_id() or s["scale"])
            head["transform"] = w["rot"].get_active_id() or "normal"
            heads[s["name"]] = head
        # Positions: next to another screen, by its size on the desktop.
        for s in self.screens:
            w = self.widgets.get(s["name"], {})
            where = w["place"].get_active_id() if "place" in w else "keep"
            if not where or where == "keep":
                continue
            side, other = where.split(":", 1)
            o = next(x for x in self.screens if x["name"] == other)
            ow, oh = self._logical(o, {**displays.snapshot(o), **heads[other]})
            mw, mh = self._logical(s, {**displays.snapshot(s), **heads[s["name"]]})
            ox, oy = heads[other].get("x", o["x"]), heads[other].get("y", o["y"])
            heads[s["name"]]["x"], heads[s["name"]]["y"] = {"right": (ox + ow, oy), "left": (ox - mw, oy),
                                                           "above": (ox, oy - mh), "below": (ox, oy + oh)}[side]
        return list(heads.values())

    def apply(self):
        before = [displays.snapshot(s) for s in self.screens]
        heads = self.wanted()

        def work():
            displays.apply(heads, test=True)
            displays.apply(heads)
            return displays.screens()

        def done(screens, error):
            if error:
                self.say(f"Not changed: {error}", error=True)
                return
            if self.keep_dialog():
                displays.save(screens)
                self.say("Saved. These screens will look like this every time you sign in.")
                self._loaded(screens, None)
            else:
                _bg(lambda: (displays.apply(before), displays.screens())[1], self._loaded)
                self.say("Put back as it was.")
        self.say("Changing…")
        _bg(work, done)

    def keep_dialog(self):
        """Keep these settings? Reverts by itself after 15 s (in case the
        screen shows nothing now)."""
        d = Gtk.Dialog(title="Keep these display settings?", transient_for=self.win, modal=True)
        d.add_button("Put back", Gtk.ResponseType.CANCEL)
        d.add_button("Keep", Gtk.ResponseType.OK)
        label = _label("")
        d.get_content_area().set_border_width(16)
        d.get_content_area().add(label)
        left = {"n": self.KEEP_SECONDS}

        def tick():
            if left["n"] <= 0:
                d.response(Gtk.ResponseType.CANCEL)
                return False
            label.set_text(f"Going back to the previous settings in {left['n']} s.")
            left["n"] -= 1
            return True
        tick()
        timer = GLib.timeout_add(1000, tick)
        d.show_all()
        response = d.run()
        GLib.source_remove(timer) if left["n"] > 0 else None
        d.destroy()
        return response == Gtk.ResponseType.OK

    # Night light
    def _night(self):
        n = displays.load_night()
        self.add(Gtk.Separator(), 6)
        self.heading("Night light")
        self.add(_label("Warmer colours in the evening are easier on the eyes and on sleep.", dim=True))
        self.night_on = Gtk.Switch(active=n["enabled"])
        self.add(_row("Night light", self.night_on))
        self.night_temp = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 2500, 6000, 100)
        self.night_temp.set_inverted(True)  # right = warmer
        self.night_temp.set_value(n["temp"])
        self.night_temp.set_draw_value(False)
        self.night_temp.set_size_request(220, -1)
        self.add(_row("Warmth", self.night_temp))
        times = Gtk.Box(spacing=6)
        self.night_from = Gtk.Entry(text=n["from"], width_chars=6)
        self.night_to = Gtk.Entry(text=n["to"], width_chars=6)
        for w in (_label("From", wrap=False), self.night_from, _label("to", wrap=False), self.night_to):
            times.pack_start(w, False, False, 0)
        self.add(_row("Hours", times))
        save = Gtk.Button(label="Save night light")
        save.set_name("actionbtn")
        save.set_halign(Gtk.Align.START)
        save.connect("clicked", lambda _b: self.save_night())
        self.add(save)

    def save_night(self):
        try:
            displays.save_night(self.night_on.get_active(), self.night_temp.get_value(),
                                self.night_from.get_text().strip(), self.night_to.get_text().strip())
        except (ValueError, OSError) as e:
            self.say(f"Not saved: {e}", error=True)
            return
        self.say("Night light " + ("on." if self.night_on.get_active() else "off."))


# ------------------------------------------------------------------ Sound

class SoundPage(_Page):
    title = "Sound"

    def __init__(self, win):
        super().__init__(win)
        self.body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        self.add(self.body)
        refresh = Gtk.Button(label="Refresh")
        refresh.set_halign(Gtk.Align.START)
        refresh.connect("clicked", lambda _b: self.reload())
        self.add(refresh)
        self.add(self.status)
        self.reload()

    def reload(self):
        _bg(lambda: (sound.outputs(), sound.inputs(), sound.apps()), self._loaded)

    def _loaded(self, data, error):
        for c in self.body.get_children():
            self.body.remove(c)
        if error:
            self.say(f"Sound isn't available: {error}", error=True)
            return
        outs, ins, apps = data
        for kind, title, devices in (("output", "Output (speakers, headphones)", outs), ("input", "Input (microphones)", ins)):
            self.body.pack_start(self.win._field_label(title), False, False, 0)
            if not devices:
                self.body.pack_start(_label("None found", dim=True), False, False, 0)
                continue
            default = next((d for d in devices if d["default"]), devices[0])
            pick = _combo([(d["name"], d["description"]) for d in devices], default["name"],
                          lambda name, k=kind: self._call(lambda: sound.set_default(k, name), reload=True))
            self.body.pack_start(_row("Device", pick), False, False, 0)
            vol = _slider(0, 150, default["volume"], lambda v, k=kind, n=default["name"]: self._call(lambda: sound.set_volume(k, n, v)))
            self.body.pack_start(_row("Volume", vol), False, False, 0)
            mute = _switch(default["mute"], lambda m, k=kind, n=default["name"]: self._call(lambda: sound.set_mute(k, n, m)))
            self.body.pack_start(_row("Muted", mute), False, False, 0)
        self.body.pack_start(self.win._field_label("Apps playing sound"), False, False, 0)
        if not apps:
            self.body.pack_start(_label("No app is playing sound right now.", dim=True), False, False, 0)
        for a in apps:
            box = Gtk.Box(spacing=8)
            box.pack_start(_slider(0, 150, a["volume"], lambda v, i=a["index"]: self._call(lambda: sound.set_app_volume(i, v))),
                           False, False, 0)
            box.pack_start(_switch(not a["mute"], lambda on, i=a["index"]: self._call(lambda: sound.set_app_mute(i, not on))),
                           False, False, 0)
            self.body.pack_start(_row(a["app"], box), False, False, 0)
        self.body.show_all()
        self.say("")

    def _call(self, fn, reload=False):
        def done(_r, error):
            if error:
                self.say(error, error=True)
            elif reload:
                self.reload()
        _bg(fn, done)


# ------------------------------------------------------- Mouse & Touchpad

class InputPage(_Page):
    title = "Mouse & Touchpad"

    def __init__(self, win):
        super().__init__(win)
        self.p = inputconf.load()
        self.heading("Mouse")
        self.add(_row("Pointer speed", _slider(-1, 1, self.p["mouse_speed"], lambda v: self.set(mouse_speed=round(v, 2)), step=0.05, digits=2)))
        self.add(_row("Natural scrolling", _switch(self.p["mouse_natural_scroll"], lambda v: self.set(mouse_natural_scroll=v)),
                      "The page follows your fingers, like on a phone"))
        self.add(_row("Left-handed", _switch(self.p["left_handed"], lambda v: self.set(left_handed=v)),
                      "Swaps the left and right buttons"))
        self.add(Gtk.Separator(), 6)
        self.heading("Touchpad")
        self.add(_row("Pointer speed", _slider(-1, 1, self.p["touchpad_speed"], lambda v: self.set(touchpad_speed=round(v, 2)), step=0.05, digits=2)))
        self.add(_row("Tap to click", _switch(self.p["tap_to_click"], lambda v: self.set(tap_to_click=v))))
        self.add(_row("Natural scrolling", _switch(self.p["touchpad_natural_scroll"], lambda v: self.set(touchpad_natural_scroll=v))))
        self.add(_row("Ignore the touchpad while typing", _switch(self.p["disable_while_typing"], lambda v: self.set(disable_while_typing=v))))
        self.add(self.status)

    def set(self, **changes):
        self.p.update(changes)
        try:
            self.p = inputconf.save(self.p)
        except OSError as e:
            self.say(f"Couldn't save: {e}", error=True)
            return
        self.say("Saved and applied.")


# ----------------------------------------------------------------- Wi-Fi

class WifiPage(_Page):
    title = "Wi-Fi"

    def __init__(self, win):
        super().__init__(win)
        self.current = _label("", wrap=True)
        self.add(self.current)
        bar = Gtk.Box(spacing=8)
        self.scan_btn = Gtk.Button(label="Look for networks")
        self.scan_btn.connect("clicked", lambda _b: self.reload(scan=True))
        self.disconnect_btn = Gtk.Button(label="Disconnect")
        self.disconnect_btn.connect("clicked", lambda _b: self._run(wifi.disconnect, "Disconnected."))
        bar.pack_start(self.scan_btn, False, False, 0)
        bar.pack_start(self.disconnect_btn, False, False, 0)
        self.add(bar)
        self.list = Gtk.ListBox()
        self.list.set_selection_mode(Gtk.SelectionMode.NONE)
        self.add(self.list)
        self.add(self.status)
        self.reload()

    def reload(self, scan=False):
        self.scan_btn.set_sensitive(False)
        self.say("Looking…" if scan else "")
        _bg(lambda: (wifi.status(), wifi.list_networks(do_scan=scan), wifi.list_known()), self._loaded)

    def _loaded(self, data, error):
        self.scan_btn.set_sensitive(True)
        for c in self.list.get_children():
            self.list.remove(c)
        if error:
            self.current.set_text("")
            self.say(f"Wi-Fi isn't available: {error}", error=True)
            return
        st, nets, known = data
        here = st.get("connected_to")
        self.current.set_text(f"Connected to {here}" if here else "Not connected to Wi-Fi")
        self.disconnect_btn.set_sensitive(bool(here))
        known_names = {k.get("ssid") if isinstance(k, dict) else k for k in known}
        for n in nets:
            ssid = n.get("ssid") or "(hidden network)"
            row = Gtk.Box(spacing=10)
            row.set_border_width(6)
            bars = "▂▄▆█"[: max(1, round((n.get("signal") or 0) / 25))]
            lock = "🔒 " if n.get("security") not in (None, "", "open") else ""
            row.pack_start(_label(f"{bars}  {lock}{ssid}", wrap=False), True, True, 0)
            if ssid == here:
                row.pack_start(_label("Connected", dim=True, wrap=False), False, False, 0)
            else:
                b = Gtk.Button(label="Connect")
                b.connect("clicked", lambda _b, s=ssid, sec=n.get("security"), k=ssid in known_names: self.connect(s, sec, k))
                row.pack_start(b, False, False, 0)
            if ssid in known_names:
                f = Gtk.Button(label="Forget")
                f.connect("clicked", lambda _b, s=ssid: self._run(lambda: wifi.forget(s), f"Forgot {s}."))
                row.pack_start(f, False, False, 0)
            self.list.add(row)
        self.list.show_all()
        if not nets:
            self.say("No networks found. Look again, or check that Wi-Fi is switched on.")

    def connect(self, ssid, security, known):
        passphrase = None
        if security not in (None, "", "open") and not known:
            passphrase = self._ask(ssid)
            if passphrase is None:
                return
        self._run(lambda: wifi.connect(ssid, passphrase), f"Connected to {ssid}.")

    def _ask(self, ssid):
        d = Gtk.Dialog(title=f"Join {ssid}", transient_for=self.win, modal=True)
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Connect", Gtk.ResponseType.OK)
        d.set_default_response(Gtk.ResponseType.OK)
        e = Gtk.Entry(visibility=False, activates_default=True)
        e.set_placeholder_text("Wi-Fi password")
        box = d.get_content_area()
        box.set_border_width(14)
        box.set_spacing(8)
        box.add(_label(f"The password for {ssid}:"))
        box.add(e)
        d.show_all()
        ok = d.run() == Gtk.ResponseType.OK
        text = e.get_text()
        d.destroy()
        return text if ok else None

    def _run(self, fn, ok_text):
        self.say("Working…")

        def done(_r, error):
            self.say(error if error else ok_text, error=bool(error))
            self.reload()
        _bg(fn, done)


# --------------------------------------------------------- Notifications

class NotificationsPage(_Page):
    title = "Notifications"

    def __init__(self, win):
        super().__init__(win)
        self.p = notifyconf.load()
        self.dnd = _switch(notifyconf.dnd_on(), self._dnd)
        self.add(_row("Do not disturb", self.dnd, "Pop-ups are kept quiet; they're still in Deck's Signals card"))
        times = [(0, "Until I close them"), (3, "3 seconds"), (6, "6 seconds"), (10, "10 seconds"), (20, "20 seconds")]
        if self.p["timeout"] not in dict(times):
            times.append((self.p["timeout"], f"{self.p['timeout']} seconds"))
        self.add(_row("Pop-ups stay", _combo(times, self.p["timeout"], lambda v: self.set(timeout=int(v)))))
        self.add(_row("Where pop-ups appear", _combo(list(notifyconf.POSITIONS.items()), self.p["position"],
                                                     lambda v: self.set(position=v))))
        self.add(Gtk.Separator(), 6)
        self.heading("Apps")
        self.add(_label("Switch an app off to hide its pop-ups.", dim=True))
        self.apps = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.add(self.apps)
        self.add(self.status)
        self.fill_apps()

    def fill_apps(self):
        for c in self.apps.get_children():
            self.apps.remove(c)
        names = sorted(set(notifyconf.recent_apps()) | set(self.p["muted_apps"]))
        if not names:
            self.apps.pack_start(_label("Apps that send notifications show up here.", dim=True), False, False, 0)
        for name in names:
            self.apps.pack_start(_row(name, _switch(name not in self.p["muted_apps"], lambda on, n=name: self.mute(n, not on))),
                                 False, False, 0)
        self.apps.show_all()

    def _dnd(self, on):
        try:
            notifyconf.set_dnd(on)
            self.say("Do not disturb is " + ("on." if on else "off."))
        except Exception as e:
            self.say(str(e), error=True)

    def mute(self, name, muted):
        apps = set(self.p["muted_apps"])
        (apps.add if muted else apps.discard)(name)
        self.set(muted_apps=sorted(apps))

    def set(self, **changes):
        self.p.update(changes)
        try:
            self.p = notifyconf.save(self.p)
        except OSError as e:
            self.say(f"Couldn't save: {e}", error=True)
            return
        self.say("Saved.")


# ---------------------------------------------------------- Default Apps

# What each choice sets: the first type picks the list of apps offered.
DEFAULT_KINDS = [
    ("Web browser", ["x-scheme-handler/http", "x-scheme-handler/https", "text/html"]),
    ("Email", ["x-scheme-handler/mailto"]),
    ("Files and folders", ["inode/directory"]),
    ("Music", ["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav", "audio/mp4"]),
    ("Video", ["video/mp4", "video/x-matroska", "video/webm", "video/quicktime"]),
    ("Pictures", ["image/png", "image/jpeg", "image/gif", "image/webp", "image/svg+xml"]),
    ("PDF documents", ["application/pdf"]),
    ("Text files", ["text/plain", "text/markdown"]),
]


class DefaultAppsPage(_Page):
    title = "Default Apps"

    def __init__(self, win):
        super().__init__(win)
        self.add(_label("Which app opens each kind of file or link.", dim=True))
        self.combos = {}
        for title, types in DEFAULT_KINDS:
            apps = Gio.AppInfo.get_all_for_type(types[0])
            current = Gio.AppInfo.get_default_for_type(types[0], False)
            if not apps:
                self.add(_row(title, _label("No app for this yet", dim=True, wrap=False)))
                continue
            items = [(a.get_id(), a.get_display_name()) for a in apps]
            combo = _combo(items, current.get_id() if current else items[0][0],
                           lambda app_id, t=types, name=title: self.choose(name, t, app_id))
            self.combos[title] = combo
            self.add(_row(title, combo))
        self.add(self.status)

    def choose(self, title, types, app_id):
        app = next((a for a in Gio.AppInfo.get_all_for_type(types[0]) if a.get_id() == app_id), None)
        if not app:
            return
        try:
            for t in types:
                app.set_as_default_for_type(t)
        except GLib.Error as e:
            self.say(f"Couldn't set it: {e.message}", error=True)
            return
        self.say(f"{title} now open in {app.get_display_name()}.")


# --------------------------------------------------------- Accessibility

class AccessibilityPage(_Page):
    title = "Accessibility"

    def __init__(self, win):
        super().__init__(win)
        a = accessconf.load()
        self.add(_row("Text size", _combo([(f"{s:g}", f"{round(s * 100)} %") for s in accessconf.TEXT_SCALES], f"{a['text_scale']:g}",
                                          lambda v: self.set(text_scale=float(v))), "In every app"))
        sizes = dict(zip(accessconf.CURSOR_SIZES, ("Normal", "Large", "Larger", "Largest")))
        self.add(_row("Pointer size", _combo(list(sizes.items()), a["cursor_size"], lambda v: self.set(cursor_size=int(v)))))
        self.add(_row("High contrast", _switch(a["high_contrast"], lambda v: self.set(high_contrast=v)),
                      "White on black with yellow highlights"))
        self.add(_row("Reduce motion", _switch(a["reduce_motion"], lambda v: self.set(reduce_motion=v)),
                      "No animations; living backgrounds stand still"))
        self.add(_label("Apps already open take the new text size when they open again.", dim=True))
        self.add(self.status)

    def set(self, **changes):
        self.say("Applying…")
        _bg(lambda: accessconf.save(changes), lambda _p, e: self.say(f"Couldn't apply: {e}" if e else "Applied.", error=bool(e)))
