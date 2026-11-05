"""Settings > Layers: Strata's five layers (workspaces 1 to 5) and each one's
mood (TODOnexa_identity.md phase 5): a name, which notifications pop up,
the background, how dark it is, and power. A thin view over
nexa_common.strataconf; Orbit, Living and Nexa Signals follow the file
live, so nothing needs restarting."""
import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, GLib, Gtk

from nexa_common import magic, notifyconf, strataconf

from .device_pages import _Page, _combo, _label, _row, _slider, _switch

CSS = b"""
#layercard { background-color: @nexa_surface; border-radius: 16px; padding: 16px; }
#layernum { color: @nexa_accent; font-weight: 700; font-size: 12pt; }
"""


class LayersPage(_Page):
    title = "Layers"

    def __init__(self, win):
        super().__init__(win)
        self.conf = strataconf.load()
        self.add(_label("Your workspaces are five layers, one behind the other: Super+scroll or Super+Page Down "
                        "goes deeper, with Shift the window in front comes along. Give a layer a mood and it "
                        "applies whenever that layer is on screen.", dim=True))
        self.add(_row("Show the layers when you move", _switch(self.conf["show"], lambda on: self.set(show=on)),
                      "A stack of glass sheets for about a second"))
        self.add(_row("Colour of each layer", _slider(0, strataconf.TINT_MAX * 100, self.conf["tint"] * 100,
                                                      lambda v: self.set(tint=round(v) / 100)),
                      "How strongly the background takes the layer's colour"))
        self.cards = []
        self.apps = sorted(set(notifyconf.recent_apps()) | {a for m in self.conf["layers"] for a in m["mute"]})
        for i in range(strataconf.LAYERS):
            card = self._card(i)
            self.cards.append(card)
            self.add(card, 4)
        self.add(self.status)
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    # ---- one layer ----

    def _card(self, i):
        m = self.conf["layers"][i]
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        box.set_name("layercard")
        head = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        num = Gtk.Label(label=f"⬡ {i + 1}")
        num.set_name("layernum")
        head.pack_start(num, False, False, 0)
        name = Gtk.Entry()
        name.set_max_length(30)
        name.set_placeholder_text("A name, like Work or Night")
        name.set_text(m["name"])
        name.set_hexpand(True)
        pending = {"id": 0}

        def typed(_e):
            if pending["id"]:
                GLib.source_remove(pending["id"])
            pending["id"] = GLib.timeout_add(400, lambda: (pending.update(id=0), self.mood(i, name=name.get_text()))[1] and False)
        name.connect("changed", typed)
        head.pack_start(name, True, True, 0)
        for key, label in (("focus", "Work"), ("night", "Night"), ("play", "Play")):
            b = Gtk.Button(label=label)
            b.set_tooltip_text(f"Start from the {label} mood")
            b.connect("clicked", lambda _b, k=key: self.preset(i, k))
            head.pack_start(b, False, False, 0)
        box.pack_start(head, False, False, 0)
        w = {"name": name}
        w["quiet"] = _combo([("all", "All of them"), ("quiet", "Only important ones")], "quiet" if m["quiet"] else "all",
                            lambda v: self.mood(i, quiet=v == "quiet"))
        box.pack_start(_row("Notifications that pop up", w["quiet"], "The rest wait in Deck's Signals card"), False, False, 0)
        styles = [("", "As in Magic Layer"), ("none", "Wallpaper only")] + list(magic.STYLES)
        w["style"] = _combo(styles, m["style"], lambda v: self.mood(i, style=v))
        box.pack_start(_row("Background", w["style"]), False, False, 0)
        w["dim"] = _slider(0, strataconf.DIM_MAX * 100, m["dim"] * 100, lambda v: self.mood(i, dim=round(v) / 100))
        box.pack_start(_row("Darker background", w["dim"], "For evenings, or to make windows stand out"), False, False, 0)
        w["power"] = _combo(strataconf.POWER, m["power"], lambda v: self.mood(i, power=v))
        box.pack_start(_row("Power", w["power"], "Back to what it was when you leave this layer"), False, False, 0)
        mutes = Gtk.FlowBox()
        mutes.set_selection_mode(Gtk.SelectionMode.NONE)
        mutes.set_max_children_per_line(4)
        w["mute"] = {}
        for app in self.apps:
            cb = Gtk.CheckButton(label=app)
            cb.set_active(app in m["mute"])
            cb.connect("toggled", lambda c, a=app: self.mute(i, a, c.get_active()))
            w["mute"][app] = cb
            mutes.add(cb)
        box.pack_start(_label("Keep these apps quiet on this layer:" if self.apps else
                              "Apps that send notifications show up here, to keep them quiet on this layer.", dim=True),
                       False, False, 0)
        if self.apps:
            box.pack_start(mutes, False, False, 0)
        box.widgets = w
        return box

    # ---- changes ----

    def mood(self, i, **changes):
        self.conf["layers"][i].update(changes)
        self._save(f"Layer {i + 1}: saved.")

    def mute(self, i, app, on):
        apps = set(self.conf["layers"][i]["mute"])
        (apps.add if on else apps.discard)(app)
        self.mood(i, mute=sorted(apps))

    def preset(self, i, key):
        m = dict(strataconf.PRESETS[key], mute=list(self.conf["layers"][i]["mute"]))
        self.conf["layers"][i] = m
        w = self.cards[i].widgets
        w["name"].set_text(m["name"])
        w["quiet"].set_active_id("quiet" if m["quiet"] else "all")
        w["style"].set_active_id(m["style"])
        w["dim"].set_value(m["dim"] * 100)
        w["power"].set_active_id(m["power"])
        self._save(f"Layer {i + 1} is now {m['name']}.")

    def set(self, **changes):
        self.conf.update(changes)
        self._save("Saved.")

    def _save(self, ok):
        try:
            self.conf = strataconf.save(self.conf)
        except OSError as e:
            self.say(f"Couldn't save: {e}", error=True)
            return
        self.say(ok)
