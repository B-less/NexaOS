import os
from collections import deque

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#statlabel { color: @nexa_text; font-size: 10pt; font-weight: 600; }
progressbar trough { background-image: none; background-color: @nexa_surface_hi; border-radius: 4px; min-height: 8px; }
progressbar progress { background-image: none; background-color: @nexa_accent; border-radius: 4px; min-height: 8px; }
"""

HISTORY_LEN = 60


def _read_cpu_jiffies():
    with open("/proc/stat") as f:
        parts = f.readline().split()
    values = [int(x) for x in parts[1:]]
    idle = values[3] + values[4]  # idle + iowait
    total = sum(values)
    return idle, total


def _read_mem_percent():
    info = {}
    with open("/proc/meminfo") as f:
        for line in f:
            key, _, rest = line.partition(":")
            info[key] = int(rest.split()[0])
    total = info.get("MemTotal", 1)
    available = info.get("MemAvailable", total)
    used_pct = max(0.0, (total - available) / total * 100)
    return used_pct, total // 1024, (total - available) // 1024


def _read_disk_percent(path="/"):
    st = os.statvfs(path)
    total = st.f_blocks * st.f_frsize
    free = st.f_bavail * st.f_frsize
    used = total - free
    pct = (used / total * 100) if total else 0.0
    return pct, total // (1024 * 1024), used // (1024 * 1024)


class SysMonWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa System Monitor")
        self.set_icon_name("nexa-sysmon")
        attach_titlebar(self, "Nexa System Monitor")
        self.set_default_size(560, 480)

        self._prev_idle, self._prev_total = _read_cpu_jiffies()
        self.cpu_history = deque([0.0] * HISTORY_LEN, maxlen=HISTORY_LEN)

        self._build_ui()
        self._refresh()
        GLib.timeout_add_seconds(1, self._on_timer)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        root.set_border_width(16)
        self.add(root)

        self.cpu_label = Gtk.Label(label="CPU")
        self.cpu_label.set_name("statlabel")
        self.cpu_label.set_xalign(0)
        root.pack_start(self.cpu_label, False, False, 0)

        self.graph = Gtk.DrawingArea()
        self.graph.set_size_request(-1, 140)
        self.graph.connect("draw", self._on_draw_graph)
        root.pack_start(self.graph, False, False, 0)

        self.cpu_bar = Gtk.ProgressBar()
        root.pack_start(self.cpu_bar, False, False, 0)

        self.mem_label = Gtk.Label(label="Memory")
        self.mem_label.set_name("statlabel")
        self.mem_label.set_xalign(0)
        self.mem_label.set_margin_top(12)
        root.pack_start(self.mem_label, False, False, 0)
        self.mem_bar = Gtk.ProgressBar()
        root.pack_start(self.mem_bar, False, False, 0)

        self.disk_label = Gtk.Label(label="Disk (/)")
        self.disk_label.set_name("statlabel")
        self.disk_label.set_xalign(0)
        self.disk_label.set_margin_top(12)
        root.pack_start(self.disk_label, False, False, 0)
        self.disk_bar = Gtk.ProgressBar()
        root.pack_start(self.disk_bar, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _on_draw_graph(self, widget, cr):
        w = widget.get_allocated_width()
        h = widget.get_allocated_height()
        cr.set_source_rgb(0.137, 0.137, 0.137)
        cr.rectangle(0, 0, w, h)
        cr.fill()

        cr.set_source_rgb(0.478, 0.647, 0.969)
        cr.set_line_width(2)
        n = len(self.cpu_history)
        for i, val in enumerate(self.cpu_history):
            x = w * i / max(n - 1, 1)
            y = h - (val / 100) * h
            if i == 0:
                cr.move_to(x, y)
            else:
                cr.line_to(x, y)
        cr.stroke()
        return False

    def _on_timer(self):
        self._refresh()
        return True

    def _refresh(self):
        idle, total = _read_cpu_jiffies()
        d_idle = idle - self._prev_idle
        d_total = max(total - self._prev_total, 1)
        cpu_pct = max(0.0, 100 * (1 - d_idle / d_total))
        self._prev_idle, self._prev_total = idle, total
        self.cpu_history.append(cpu_pct)
        self.cpu_label.set_text(f"CPU: {cpu_pct:.1f}%")
        self.cpu_bar.set_fraction(min(cpu_pct / 100, 1.0))
        self.graph.queue_draw()

        mem_pct, mem_total, mem_used = _read_mem_percent()
        self.mem_label.set_text(f"Memory: {mem_used} / {mem_total} MB ({mem_pct:.1f}%)")
        self.mem_bar.set_fraction(min(mem_pct / 100, 1.0))

        disk_pct, disk_total, disk_used = _read_disk_percent()
        self.disk_label.set_text(f"Disk (/): {disk_used} / {disk_total} MB ({disk_pct:.1f}%)")
        self.disk_bar.set_fraction(min(disk_pct / 100, 1.0))


def main():
    Gtk.init()
    win = SysMonWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
