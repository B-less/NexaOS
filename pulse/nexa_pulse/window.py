"""Nexa Pulse: running apps and processes, plus live CPU, memory, disk and
network graphs. Replaces Nexa Tasks and Nexa System Monitor."""
import os
import signal
import sys
import threading
from collections import deque

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib, Pango

from nexa_common import brain, compositor, desktopapps, exits, magic, themes
from nexa_common.titlebar import attach_titlebar

from . import stats

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
notebook, notebook > stack { background-image: none; background-color: @nexa_bg; }
notebook header { background-image: none; background-color: @nexa_surface; border: none; }
notebook header tab { padding: 6px 16px; color: @nexa_muted; border: none; }
notebook header tab:checked { color: @nexa_text; box-shadow: inset 0 -2px @nexa_accent; }
#toolbar { background-image: none; background-color: @nexa_bg; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 12px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#toolbtn:disabled { color: @nexa_disabled; }
treeview { background-image: none; background-color: @nexa_bg; color: @nexa_text; }
treeview:selected { background-image: none; background-color: @nexa_selected; color: @nexa_selected_text; }
treeview header button { background-image: none; background-color: @nexa_surface; color: @nexa_muted; }
#card { background-image: none; background-color: @nexa_surface; border-radius: 12px; }
#card-title { color: @nexa_muted; font-size: 9pt; }
#card-value { color: @nexa_text; font-size: 15pt; font-weight: bold; }
#card-detail { color: @nexa_muted; font-size: 9pt; }
#statusbar { color: @nexa_muted; font-size: 9pt; }
#why-proven { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 8px; padding: 1px 8px; font-size: 8pt; font-weight: bold; }
#why-possible { color: @nexa_muted; border: 1px solid @nexa_muted; border-radius: 8px; padding: 0 7px; font-size: 8pt; }
#why-text { color: @nexa_text; }
#why-evidence { color: @nexa_muted; font-size: 8pt; }
#why-row { background-image: none; background-color: @nexa_surface; border-radius: 10px; }
"""

WHY_TOPICS = [("slow", "Slow"), ("crash", "An app closed"), ("memory", "Memory"), ("network", "No internet"),
              ("stopped", "Something stopped")]

INTERVAL_S = 1
HISTORY = 60
COL_PID, COL_NAME, COL_USER, COL_CPU, COL_MEM, COL_CPU_SORT, COL_MEM_SORT, COL_BOLD = range(8)


def _hex_rgb(hex_colour):
    h = hex_colour.lstrip("#")
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))


def _accents():
    t = themes.THEMES[themes.load_settings()["theme"]]
    return _hex_rgb(t["accent"]), _hex_rgb(t["accent2"])


def _app_names():
    """desktop-file id -> visible name, for labelling open windows."""
    return {a["id"]: a["name"] for a in desktopapps.load_apps()}


class Graph(Gtk.DrawingArea):
    """A filled sparkline. Values are 0..100 unless scale is 'auto'."""

    def __init__(self, series=1, auto_scale=False):
        super().__init__()
        self.set_size_request(-1, 90)
        self.series = [deque(maxlen=HISTORY) for _ in range(series)]
        self.auto_scale = auto_scale
        self.connect("draw", self._draw)

    def push(self, *values):
        for s, v in zip(self.series, values):
            s.append(v)
        self.queue_draw()

    def _draw(self, widget, cr):
        w, h = widget.get_allocated_width(), widget.get_allocated_height()
        colours = _accents()
        top = max([1.0] + [v for s in self.series for v in s]) * 1.15 if self.auto_scale else 100.0
        cr.set_line_width(1)
        cr.set_source_rgba(*colours[0], 0.12)
        for i in range(1, 4):
            y = round(h * i / 4) + 0.5
            cr.move_to(0, y)
            cr.line_to(w, y)
        cr.stroke()
        step = w / (HISTORY - 1)
        for n, s in enumerate(self.series):
            if len(s) < 2:
                continue
            rgb = colours[n % 2]
            # newest sample at the right edge; history grows in from there
            points = [(w - (len(s) - 1 - i) * step, h - min(v, top) / top * (h - 2) - 1) for i, v in enumerate(s)]
            cr.move_to(points[0][0], h)
            for x, y in points:
                cr.line_to(x, y)
            cr.line_to(w, h)
            cr.close_path()
            cr.set_source_rgba(*rgb, 0.18)
            cr.fill()
            cr.move_to(*points[0])
            for x, y in points[1:]:
                cr.line_to(x, y)
            cr.set_source_rgba(*rgb, 0.95)
            cr.set_line_width(2)
            cr.stroke()
        return False


class Card(Gtk.Box):
    def __init__(self, title, graph):
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.set_name("card")
        inner = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        for side in ("start", "end", "top", "bottom"):
            getattr(inner, f"set_margin_{side}")(14)
        super().pack_start(inner, True, True, 0)
        self.pack_start = inner.pack_start
        head = Gtk.Box(spacing=8)
        name = Gtk.Label(label=title, xalign=0)
        name.set_name("card-title")
        self.value = Gtk.Label(xalign=1)
        self.value.set_name("card-value")
        head.pack_start(name, True, True, 0)
        head.pack_end(self.value, False, False, 0)
        self.detail = Gtk.Label(xalign=0)
        self.detail.set_name("card-detail")
        self.pack_start(head, False, False, 0)
        self.pack_start(graph, True, True, 0)
        self.pack_start(self.detail, False, False, 0)
        self.graph = graph


class PulseWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa Pulse")
        self.set_icon_name("nexa-pulse")
        attach_titlebar(self, "Nexa Pulse")
        self.set_default_size(820, 580)

        self._users = stats.usernames()
        self._app_names = _app_names()
        self._cpu = stats.cpu_times()
        self._net = stats.net_bytes()
        procs = stats.processes()
        self._jiffies = {p["pid"]: p["jiffies"] for p in procs}
        self._filter = ""

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

        self.notebook = Gtk.Notebook()
        self.notebook.append_page(self._processes_page(), Gtk.Label(label="Processes"))
        self.notebook.append_page(self._performance_page(), Gtk.Label(label="Performance"))
        self.notebook.append_page(self._why_page(), Gtk.Label(label="Why?"))
        self.add(self.notebook)
        self.connect("key-press-event", self._on_key)

        self._tick()
        GLib.timeout_add_seconds(INTERVAL_S, self._tick)

    # ---- Processes -------------------------------------------------------

    def _processes_page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        bar = Gtk.Box(spacing=6)
        bar.set_name("toolbar")
        bar.set_border_width(8)
        self.search = Gtk.SearchEntry(placeholder_text="Search processes")
        self.search.connect("search-changed", self._on_search)
        bar.pack_start(self.search, True, True, 0)
        self.end_btn = self._toolbtn("End task", self._on_end, signal.SIGTERM)
        self.kill_btn = self._toolbtn("Force quit", self._on_end, signal.SIGKILL)
        bar.pack_end(self.kill_btn, False, False, 0)
        bar.pack_end(self.end_btn, False, False, 0)
        page.pack_start(bar, False, False, 0)

        # pid, name, user, cpu text, mem text, cpu sort, mem sort, weight
        self.store = Gtk.TreeStore(int, str, str, str, str, float, int, int)
        self.view = Gtk.TreeView(model=self.store)
        self.view.get_selection().connect("changed", lambda *_: self._update_buttons())
        for title, col, sort, expand, right in (
                ("Name", COL_NAME, COL_NAME, True, False),
                ("PID", COL_PID, COL_PID, False, True),
                ("User", COL_USER, COL_USER, False, False),
                ("CPU", COL_CPU, COL_CPU_SORT, False, True),
                ("Memory", COL_MEM, COL_MEM_SORT, False, True)):
            cell = Gtk.CellRendererText(xalign=1.0 if right else 0.0)
            if col == COL_NAME:
                cell.set_property("ellipsize", Pango.EllipsizeMode.END)
            column = Gtk.TreeViewColumn(title, cell, text=col, weight=COL_BOLD)
            if col == COL_PID:  # group rows carry pid 0; show nothing there
                column.set_cell_data_func(cell, lambda _c, r, m, i, _d: r.set_property(
                    "text", str(m[i][COL_PID]) if m[i][COL_PID] else ""))
            column.set_sort_column_id(sort)
            column.set_expand(expand)
            column.set_resizable(True)
            if not expand:
                column.set_min_width(80)
            self.view.append_column(column)
        self._bg_row = None
        self.scroller = Gtk.ScrolledWindow()
        self.scroller.add(self.view)
        page.pack_start(self.scroller, True, True, 0)

        self.status = Gtk.Label(xalign=0)
        self.status.set_name("statusbar")
        self.status.set_margin_start(10)
        self.status.set_margin_top(4)
        self.status.set_margin_bottom(4)
        page.pack_start(self.status, False, False, 0)
        self._update_buttons()
        return page

    def _toolbtn(self, label, handler, sig):
        btn = Gtk.Button(label=label)
        btn.set_name("toolbtn")
        btn.connect("clicked", handler, sig)
        return btn

    def _fill_processes(self, procs, cpu):
        selected = self._selected_pid()
        scroll = self.scroller.get_vadjustment().get_value()
        bg_open = self.view.row_expanded(self.store.get_path(self._bg_row)) if self._bg_row else True
        window_names = {}
        for w in compositor.windows():
            desktop_id = w["app_id"].replace("_", "-")
            window_names.setdefault(w["pid"], self._app_names.get(desktop_id, w["app_id"]))

        query = self._filter
        rows_apps, rows_bg = [], []
        for p in procs:
            label = window_names.get(p["pid"], p["name"])
            if query and query not in label.lower() and query not in str(p["pid"]):
                continue
            row = [p["pid"], label, self._users.get(p["uid"], str(p["uid"])),
                   f"{cpu.get(p['pid'], 0.0):.1f}%", stats.human_bytes(p["rss_kb"] * 1024),
                   cpu.get(p["pid"], 0.0), p["rss_kb"], 400]
            (rows_apps if p["pid"] in window_names else rows_bg).append(row)

        self.store.clear()
        reselect = None
        parents = []
        for title, rows in ((f"Apps ({len(rows_apps)})", rows_apps),
                            (f"Background processes ({len(rows_bg)})", rows_bg)):
            parent = self.store.append(None, [0, title, "", "", "", -1.0, -1, 700])
            parents.append(parent)
            for row in sorted(rows, key=lambda r: (-r[COL_CPU_SORT], -r[COL_MEM_SORT])):
                it = self.store.append(parent, row)
                if row[COL_PID] == selected:
                    reselect = it
        self._bg_row = parents[1]
        self.view.expand_row(self.store.get_path(parents[0]), False)
        if bg_open or query:
            self.view.expand_row(self.store.get_path(parents[1]), False)
        if reselect is not None:
            self.view.get_selection().select_iter(reselect)
        GLib.idle_add(self.scroller.get_vadjustment().set_value, scroll)
        self.status.set_text(f"{len(procs)} processes · {len(rows_apps)} open apps")

    def _selected_pid(self):
        model, it = self.view.get_selection().get_selected()
        return model[it][COL_PID] or None if it else None

    def _update_buttons(self):
        pid = self._selected_pid()
        self.end_btn.set_sensitive(pid is not None)
        self.kill_btn.set_sensitive(pid is not None)

    def _on_search(self, entry):
        self._filter = entry.get_text().strip().lower()
        self._fill_processes(stats.processes(), {})

    def _on_key(self, _w, event):
        if event.keyval == Gdk.KEY_Delete and self._selected_pid():
            self._on_end(None, signal.SIGTERM)
            return True
        return False

    def _on_end(self, _btn, sig):
        pid = self._selected_pid()
        if pid is None:
            return
        if pid == os.getpid():
            self.destroy()
            return
        model, it = self.view.get_selection().get_selected()
        name = model[it][COL_NAME] if it else str(pid)
        try:
            os.kill(pid, sig)
            exits.note_ended(name, pid, sig)  # so Why knows it was you
        except ProcessLookupError:
            pass
        except PermissionError:
            self._message("That process belongs to the system, so Nexa Pulse can't end it.")
        except OSError as e:
            self._message(str(e))

    def _message(self, text):
        dialog = Gtk.MessageDialog(transient_for=self, modal=True, message_type=Gtk.MessageType.INFO,
                                   buttons=Gtk.ButtonsType.OK, text=text)
        dialog.run()
        dialog.destroy()

    # ---- Why (Magic Layer: tools/nexa-microbrain/src/why.rs) -------------

    def _why_page(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        page.set_border_width(14)
        intro = Gtk.Label(xalign=0, wrap=True)
        intro.set_text("Ask why, and Nexa MicroBrain looks at the evidence. \u201cProven\u201d means it was measured or "
                       "recorded when it happened; \u201cpossible\u201d means it fits, but isn't shown to be the cause.")
        intro.set_name("card-detail")
        page.pack_start(intro, False, False, 0)
        row = Gtk.Box(spacing=6)
        self._why_buttons = {}
        for topic, label in WHY_TOPICS:
            b = Gtk.Button(label=label)
            b.set_name("toolbtn")
            b.connect("clicked", lambda _b, t=topic: self.ask_why(t))
            row.pack_start(b, False, False, 0)
            self._why_buttons[topic] = b
        page.pack_start(row, False, False, 0)
        self._why_status = Gtk.Label(xalign=0)
        self._why_status.set_name("statusbar")
        page.pack_start(self._why_status, False, False, 0)
        self._why_list = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        scroll = Gtk.ScrolledWindow()
        scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroll.add(self._why_list)
        page.pack_start(scroll, True, True, 0)
        return page

    def ask_why(self, topic):
        self.notebook.set_current_page(2)
        self._why_status.set_text("Looking\u2026")
        for b in self._why_buttons.values():
            b.set_sensitive(False)

        def work():
            try:
                reply, error = brain.request("why", timeout=6, topic=topic), None
            except brain.BrainError as e:
                reply, error = None, str(e)
            GLib.idle_add(self._show_why, reply, error)

        threading.Thread(target=work, daemon=True).start()

    def _show_why(self, reply, error):
        for b in self._why_buttons.values():
            b.set_sensitive(True)
        for child in self._why_list.get_children():
            child.destroy()
        if error:
            self._why_status.set_text(error)
            return False
        import time as _time
        self._why_status.set_text(f"{reply.get('title', '')} \u00b7 checked at {_time.strftime('%H:%M:%S')}")
        for f in reply.get("findings", []):
            box = Gtk.Box(spacing=10)
            box.set_name("why-row")
            box.set_border_width(10)
            badge = Gtk.Label(label="Proven" if f.get("certainty") == "proven" else "Possible")
            badge.set_name("why-proven" if f.get("certainty") == "proven" else "why-possible")
            badge.set_valign(Gtk.Align.START)
            box.pack_start(badge, False, False, 0)
            col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            text = Gtk.Label(label=f.get("text", ""), xalign=0, wrap=True)
            text.set_name("why-text")
            ev = Gtk.Label(label=f.get("evidence", ""), xalign=0, wrap=True)
            ev.set_name("why-evidence")
            col.pack_start(text, False, False, 0)
            col.pack_start(ev, False, False, 0)
            box.pack_start(col, True, True, 0)
            self._why_list.pack_start(box, False, False, 0)
        self._why_list.show_all()
        return False

    # ---- Performance -----------------------------------------------------

    def _performance_page(self):
        grid = Gtk.Grid(column_spacing=12, row_spacing=12, column_homogeneous=True, row_homogeneous=True)
        grid.set_border_width(14)
        self.cpu_card = Card("CPU", Graph())
        self.mem_card = Card("Memory", Graph())
        self.disk_card = Card("Disk", Graph())
        self.net_card = Card("Network", Graph(series=2, auto_scale=True))
        grid.attach(self.cpu_card, 0, 0, 1, 1)
        grid.attach(self.mem_card, 1, 0, 1, 1)
        grid.attach(self.disk_card, 0, 1, 1, 1)
        grid.attach(self.net_card, 1, 1, 1, 1)
        return grid

    # ---- sampling --------------------------------------------------------

    def _tick(self):
        cpu_now = stats.cpu_times()
        d_total = cpu_now[1] - self._cpu[1]
        cpu_pct = stats.cpu_percent(self._cpu, cpu_now)
        self._cpu = cpu_now

        procs = stats.processes()
        per_proc = stats.process_cpu(self._jiffies, procs, d_total)
        self._jiffies = {p["pid"]: p["jiffies"] for p in procs}
        self._fill_processes(procs, per_proc)

        self.cpu_card.graph.push(cpu_pct)
        self.cpu_card.value.set_text(f"{cpu_pct:.0f}%")
        magic.level("nexa-pulse", cpu_pct / 100)  # its Energy Pulse follows the load
        self.cpu_card.detail.set_text(f"{os.cpu_count()} cores · {len(procs)} processes")

        mem_pct, used_mb, total_mb = stats.memory()
        self.mem_card.graph.push(mem_pct)
        self.mem_card.value.set_text(f"{mem_pct:.0f}%")
        self.mem_card.detail.set_text(f"{used_mb / 1024:.1f} of {total_mb / 1024:.1f} GB in use")

        disk_pct, used_gb, total_gb = stats.disk("/")
        self.disk_card.graph.push(disk_pct)
        self.disk_card.value.set_text(f"{disk_pct:.0f}%")
        self.disk_card.detail.set_text(f"{used_gb:.1f} of {total_gb:.1f} GB used")

        net_now = stats.net_bytes()
        rx = max(net_now[0] - self._net[0], 0) / INTERVAL_S
        tx = max(net_now[1] - self._net[1], 0) / INTERVAL_S
        self._net = net_now
        self.net_card.graph.push(rx / 1024, tx / 1024)
        self.net_card.value.set_text(f"↓ {stats.human_bytes(rx)}/s")
        self.net_card.detail.set_text(f"Down ↓ {stats.human_bytes(rx)}/s · Up ↑ {stats.human_bytes(tx)}/s")
        return True


def main():
    Gtk.init()
    win = PulseWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    if "--performance" in sys.argv[1:]:
        win.notebook.set_current_page(1)
    if "--why" in sys.argv[1:]:
        i = sys.argv.index("--why")
        topic = sys.argv[i + 1] if i + 1 < len(sys.argv) else "slow"
        win.ask_why(topic if topic in dict(WHY_TOPICS) else "slow")
    Gtk.main()
