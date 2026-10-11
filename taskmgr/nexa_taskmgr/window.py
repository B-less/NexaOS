import os
import signal

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#toolbar { background-image: none; background-color: #252525; }
#toolbtn { background-image: none; background-color: #2f2f2f; color: #d4d4d4; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: #3a3a3a; }
#toolbtn:disabled { color: #5a5a5a; }
treeview { background-image: none; background-color: #1e1e1e; color: #d4d4d4; }
treeview:selected { background-image: none; background-color: #2c3a52; color: #dbe4f5; }
treeview header button { background-image: none; background-color: #232323; color: #9a9a9a; }
#statusbar { background-image: none; background-color: #202020; color: #8a8a8a; font-size: 9pt; }
"""

COL_PID, COL_NAME, COL_USER, COL_CPU, COL_MEM = range(5)


def _read_total_jiffies():
    with open("/proc/stat") as f:
        parts = f.readline().split()
    return sum(int(x) for x in parts[1:])


def _list_processes():
    procs = []
    for name in os.listdir("/proc"):
        if not name.isdigit():
            continue
        pid = int(name)
        try:
            with open(f"/proc/{pid}/stat") as f:
                stat = f.read()
            comm_start = stat.index("(")
            comm_end = stat.rindex(")")
            comm = stat[comm_start + 1:comm_end]
            fields = stat[comm_end + 2:].split()
            utime, stime = int(fields[11]), int(fields[12])
            uid = os.stat(f"/proc/{pid}").st_uid
            vmrss = 0
            with open(f"/proc/{pid}/status") as f:
                for line in f:
                    if line.startswith("VmRSS:"):
                        vmrss = int(line.split()[1])  # kB
                        break
        except (OSError, ValueError, IndexError):
            continue
        procs.append({
            "pid": pid, "name": comm, "uid": uid,
            "jiffies": utime + stime, "rss_kb": vmrss,
        })
    return procs


def _username(uid):
    try:
        with open("/etc/passwd") as f:
            for line in f:
                parts = line.split(":")
                if len(parts) > 2 and parts[2] == str(uid):
                    return parts[0]
    except OSError:
        pass
    return str(uid)


class TaskManagerWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa Tasks")
        self.set_icon_name("nexa-taskmgr")
        attach_titlebar(self, "Nexa Tasks")
        self.set_default_size(700, 520)

        self._prev_total = _read_total_jiffies()
        self._prev_procs = {p["pid"]: p["jiffies"] for p in _list_processes()}
        self._username_cache = {}

        self._build_ui()
        self._refresh()
        GLib.timeout_add_seconds(2, self._on_timer)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        self.kill_btn = self._toolbtn("End Task", self._on_kill)
        self.kill_btn.set_sensitive(False)
        toolbar.pack_start(self.kill_btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.store = Gtk.ListStore(int, str, str, str, str)
        self.view = Gtk.TreeView(model=self.store)
        self.view.set_headers_visible(True)
        self.view.get_selection().connect("changed", self._on_selection_changed)

        pid_col = Gtk.TreeViewColumn("PID", Gtk.CellRendererText(), text=COL_PID)
        pid_col.set_fixed_width(70)
        pid_col.set_sort_column_id(COL_PID)
        self.view.append_column(pid_col)

        name_col = Gtk.TreeViewColumn("Name", Gtk.CellRendererText(), text=COL_NAME)
        name_col.set_expand(True)
        name_col.set_sort_column_id(COL_NAME)
        self.view.append_column(name_col)

        user_col = Gtk.TreeViewColumn("User", Gtk.CellRendererText(), text=COL_USER)
        user_col.set_fixed_width(90)
        self.view.append_column(user_col)

        cpu_renderer = Gtk.CellRendererText()
        cpu_renderer.set_property("xalign", 1.0)
        cpu_col = Gtk.TreeViewColumn("CPU%", cpu_renderer, text=COL_CPU)
        cpu_col.set_fixed_width(80)
        cpu_col.set_sort_column_id(COL_CPU)
        self.view.append_column(cpu_col)

        mem_renderer = Gtk.CellRendererText()
        mem_renderer.set_property("xalign", 1.0)
        mem_col = Gtk.TreeViewColumn("Memory", mem_renderer, text=COL_MEM)
        mem_col.set_fixed_width(90)
        self.view.append_column(mem_col)

        scroller.add(self.view)
        root.pack_start(scroller, True, True, 0)

        self.status = Gtk.Label(label="")
        self.status.set_name("statusbar")
        self.status.set_xalign(0)
        self.status.set_margin_start(10)
        self.status.set_margin_top(4)
        self.status.set_margin_bottom(4)
        root.pack_start(self.status, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _toolbtn(self, label, handler):
        btn = Gtk.Button(label=label)
        btn.set_name("toolbtn")
        btn.connect("clicked", handler)
        return btn

    def _on_timer(self):
        self._refresh()
        return True

    def _refresh(self):
        total = _read_total_jiffies()
        total_delta = max(total - self._prev_total, 1)
        procs = _list_processes()

        selected_pid = self._selected_pid()
        self.store.clear()
        for p in sorted(procs, key=lambda x: x["jiffies"], reverse=True):
            prev = self._prev_procs.get(p["pid"], p["jiffies"])
            cpu_pct = max(0.0, (p["jiffies"] - prev) / total_delta * 100)
            user = self._username_cache.setdefault(p["uid"], _username(p["uid"]))
            it = self.store.append([
                p["pid"], p["name"], user,
                f"{cpu_pct:.1f}", f"{p['rss_kb'] / 1024:.1f} MB",
            ])
            if p["pid"] == selected_pid:
                self.view.get_selection().select_iter(it)

        self._prev_total = total
        self._prev_procs = {p["pid"]: p["jiffies"] for p in procs}
        self.status.set_text(f"{len(procs)} processes")

    def _selected_pid(self):
        model, it = self.view.get_selection().get_selected()
        return model[it][COL_PID] if it else None

    def _on_selection_changed(self, _sel):
        self.kill_btn.set_sensitive(self._selected_pid() is not None)

    def _on_kill(self, _btn):
        pid = self._selected_pid()
        if pid is None:
            return
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError as e:
            self._error(str(e))
        self._refresh()

    def _error(self, text):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
            buttons=Gtk.ButtonsType.OK, text=text,
        )
        dialog.run()
        dialog.destroy()


def main():
    Gtk.init()
    win = TaskManagerWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
