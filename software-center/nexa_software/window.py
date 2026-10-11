import json
import subprocess
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib

from nexa_common.titlebar import attach_titlebar
from nexa_common import apkops

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#toolbar { background-image: none; background-color: #252525; }
#searchentry { background-image: none; background-color: #2a2a2a; color: #ffffff; border-radius: 14px; padding: 6px 12px; border: 1px solid #3c3c3c; }
#toolbtn { background-image: none; background-color: #2f2f2f; color: #d4d4d4; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: #3a3a3a; }
#toolbtn:disabled { color: #5a5a5a; }
#actionbtn { background-image: none; background-color: #7aa2f7; color: #1e1e1e; border-radius: 6px; padding: 6px 14px; font-weight: bold; }
#actionbtn:hover { background-image: none; background-color: #92b6ff; }
#actionbtn:disabled { background-image: none; background-color: #3a3a3a; color: #5a5a5a; }
#removebtn { background-image: none; background-color: #f7768e; color: #1e1e1e; border-radius: 6px; padding: 6px 14px; font-weight: bold; }
#removebtn:hover { background-image: none; background-color: #ff8fa3; }
#removebtn:disabled { background-image: none; background-color: #3a3a3a; color: #5a5a5a; }
treeview { background-image: none; background-color: #1e1e1e; color: #d4d4d4; }
treeview:selected { background-image: none; background-color: #2c3a52; color: #dbe4f5; }
treeview header button { background-image: none; background-color: #232323; color: #9a9a9a; }
#statusbar { background-image: none; background-color: #202020; color: #8a8a8a; font-size: 9pt; }
notebook tab { background-image: none; background-color: #232323; color: #9a9a9a; padding: 6px 14px; }
notebook tab:checked { background-image: none; background-color: #2c3a52; color: #dbe4f5; }
"""

COL_NAME, COL_VERSION, COL_DESC = range(3)


def _call_helper(action, **args):
    request = json.dumps({"action": action, "args": args})
    try:
        result = subprocess.run(
            ["doas", "/usr/local/bin/nexa-software-helper"],
            input=request, capture_output=True, text=True, timeout=150,
        )
    except (subprocess.SubprocessError, OSError) as e:
        return False, str(e)
    try:
        response = json.loads(result.stdout)
    except json.JSONDecodeError:
        return False, (result.stderr or "").strip() or "helper produced no output"
    if response.get("ok"):
        return True, response.get("output", "")
    return False, response.get("error", "unknown error")


class SoftwareWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa Software")
        self.set_icon_name("nexa-software")
        attach_titlebar(self, "Nexa Software")
        self.set_default_size(800, 560)

        self.installed = set()
        self._build_ui()
        self._refresh_installed_set()
        self._on_refresh_installed(None)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        notebook = Gtk.Notebook()
        root.pack_start(notebook, True, True, 0)
        notebook.append_page(self._build_search_page(), Gtk.Label(label="Search"))
        notebook.append_page(self._build_installed_page(), Gtk.Label(label="Installed"))

        self.status = Gtk.Label(label="Ready")
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

    def _build_search_page(self):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        self.search_entry = Gtk.Entry()
        self.search_entry.set_name("searchentry")
        self.search_entry.set_placeholder_text("Search packages…")
        self.search_entry.set_hexpand(True)
        self.search_entry.connect("activate", self._on_search)
        toolbar.pack_start(self.search_entry, True, True, 0)
        toolbar.pack_start(self._toolbtn("Search", self._on_search), False, False, 0)
        toolbar.pack_start(self._toolbtn("Update Index", self._on_update_index), False, False, 0)
        box.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.search_store = Gtk.ListStore(str, str, str)
        self.search_view = Gtk.TreeView(model=self.search_store)
        self._add_columns(self.search_view)
        self.search_view.get_selection().connect("changed", self._on_search_selection_changed)
        scroller.add(self.search_view)
        box.pack_start(scroller, True, True, 0)

        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        actions.set_name("toolbar")
        actions.set_border_width(8)
        self.install_btn = self._toolbtn("Install", self._on_install, "actionbtn")
        self.install_btn.set_sensitive(False)
        actions.pack_start(self.install_btn, False, False, 0)
        box.pack_start(actions, False, False, 0)

        return box

    def _build_installed_page(self):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        toolbar.pack_start(self._toolbtn("Refresh", self._on_refresh_installed), False, False, 0)
        box.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        self.installed_store = Gtk.ListStore(str)
        self.installed_view = Gtk.TreeView(model=self.installed_store)
        renderer = Gtk.CellRendererText()
        col = Gtk.TreeViewColumn("Package", renderer, text=0)
        col.set_expand(True)
        self.installed_view.append_column(col)
        self.installed_view.get_selection().connect("changed", self._on_installed_selection_changed)
        scroller.add(self.installed_view)
        box.pack_start(scroller, True, True, 0)

        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        actions.set_name("toolbar")
        actions.set_border_width(8)
        self.remove_btn = self._toolbtn("Remove", self._on_remove, "removebtn")
        self.remove_btn.set_sensitive(False)
        actions.pack_start(self.remove_btn, False, False, 0)
        box.pack_start(actions, False, False, 0)

        return box

    def _add_columns(self, view):
        name_col = Gtk.TreeViewColumn("Name", Gtk.CellRendererText(), text=COL_NAME)
        name_col.set_fixed_width(220)
        name_col.set_resizable(True)
        view.append_column(name_col)
        version_col = Gtk.TreeViewColumn("Version", Gtk.CellRendererText(), text=COL_VERSION)
        version_col.set_fixed_width(140)
        view.append_column(version_col)
        desc_renderer = Gtk.CellRendererText()
        desc_renderer.set_property("ellipsize", 3)
        desc_col = Gtk.TreeViewColumn("Description", desc_renderer, text=COL_DESC)
        desc_col.set_expand(True)
        view.append_column(desc_col)

    def _toolbtn(self, label, handler, css_name="toolbtn"):
        btn = Gtk.Button(label=label)
        btn.set_name(css_name)
        btn.connect("clicked", handler)
        return btn

    def _refresh_installed_set(self):
        try:
            self.installed = apkops.list_installed()
        except apkops.ApkError as e:
            self.status.set_text(f"Couldn't read installed packages: {e}")
            self.installed = set()

    def _on_search(self, _widget):
        query = self.search_entry.get_text().strip()
        if not query:
            return
        self.status.set_text(f"Searching for '{query}'…")
        self.search_store.clear()
        self.install_btn.set_sensitive(False)

        def work():
            try:
                results, error = apkops.search(query), None
            except apkops.ApkError as e:
                results, error = [], str(e)
            GLib.idle_add(self._on_search_done, results, error)

        threading.Thread(target=work, daemon=True).start()

    def _on_search_done(self, results, error):
        if error:
            self.status.set_text(f"Search failed: {error}")
            return False
        self._refresh_installed_set()
        for pkg in results:
            label = pkg["name"] + ("  (installed)" if pkg["name"] in self.installed else "")
            self.search_store.append([label, pkg["version"], pkg["description"]])
        self.status.set_text(f"{len(results)} result{'s' if len(results) != 1 else ''}")
        return False

    def _selected_search_name(self):
        model, it = self.search_view.get_selection().get_selected()
        if it is None:
            return None
        return model[it][COL_NAME].split("  (installed)")[0]

    def _on_search_selection_changed(self, _sel):
        name = self._selected_search_name()
        already = bool(name) and name in self.installed
        self.install_btn.set_sensitive(bool(name) and not already)

    def _on_install(self, _btn):
        name = self._selected_search_name()
        if not name:
            return
        if not self._confirm(f"Install '{name}'?", "This downloads and installs the package system-wide."):
            return
        self.install_btn.set_sensitive(False)
        self.status.set_text(f"Installing {name}…")

        def work():
            ok, output = _call_helper("install", package=name)
            GLib.idle_add(self._on_action_done, "install", name, ok, output)

        threading.Thread(target=work, daemon=True).start()

    def _selected_installed_name(self):
        model, it = self.installed_view.get_selection().get_selected()
        return model[it][0] if it else None

    def _on_installed_selection_changed(self, _sel):
        self.remove_btn.set_sensitive(self._selected_installed_name() is not None)

    def _on_remove(self, _btn):
        name = self._selected_installed_name()
        if not name:
            return
        if not self._confirm(f"Remove '{name}'?", "This uninstalls the package from the system."):
            return
        self.remove_btn.set_sensitive(False)
        self.status.set_text(f"Removing {name}…")

        def work():
            ok, output = _call_helper("remove", package=name)
            GLib.idle_add(self._on_action_done, "remove", name, ok, output)

        threading.Thread(target=work, daemon=True).start()

    def _on_action_done(self, action, name, ok, output):
        verb = "Installed" if action == "install" else "Removed"
        self.status.set_text(f"{verb} {name}" if ok else f"Failed to {action} {name}: {output}")
        self._refresh_installed_set()
        self._on_refresh_installed(None)
        return False

    def _on_update_index(self, _btn):
        self.status.set_text("Updating package index…")

        def work():
            ok, output = _call_helper("update_index")
            GLib.idle_add(self._on_update_index_done, ok, output)

        threading.Thread(target=work, daemon=True).start()

    def _on_update_index_done(self, ok, output):
        self.status.set_text("Package index updated" if ok else f"Update failed: {output}")
        return False

    def _on_refresh_installed(self, _btn):
        self._refresh_installed_set()
        self.installed_store.clear()
        for name in sorted(self.installed):
            self.installed_store.append([name])

    def _confirm(self, text, secondary):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
            buttons=Gtk.ButtonsType.YES_NO, text=text,
        )
        dialog.format_secondary_text(secondary)
        response = dialog.run()
        dialog.destroy()
        return response == Gtk.ResponseType.YES


def main():
    Gtk.init()
    win = SoftwareWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
