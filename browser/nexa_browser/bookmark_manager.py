"""The bookmark manager: folders, editing, moving, deleting, search, and
import/export of other browsers' HTML bookmark files (nexa-webdata)."""
import os

from gi.repository import Gdk, Gtk, Pango

from . import webdata
from .bookmarks import clean_folder

CSS = b"""
#bm-window { background-color: @nexa_bg; }
#bm-side { background-color: @nexa_surface; }
#bm-side treeview { background-color: @nexa_surface; color: @nexa_text; }
#bm-side treeview:selected { background-color: @nexa_selected; color: @nexa_selected_text; }
#bm-row { padding: 6px 8px; }
#bm-url, #bm-status, #bm-folder { color: @nexa_muted; font-size: 9pt; }
#bm-title { color: @nexa_text; }
"""
ALL, NONE = "\x00all", ""


class BookmarkManager(Gtk.Window):
    def __init__(self, bookmarks, parent=None, open_url=None, changed=None):
        super().__init__(title="Bookmarks")
        self.set_name("bm-window")
        self.set_default_size(880, 580)
        if parent:
            self.set_transient_for(parent)
        self.bm = bookmarks
        self.open_url = open_url or (lambda url: None)
        self.changed = changed or (lambda: None)
        self.folder = ALL
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.add(root)
        bar = Gtk.Box(spacing=8)
        bar.set_border_width(8)
        self.search = Gtk.SearchEntry(placeholder_text="Search bookmarks")
        self.search.set_width_chars(28)
        self.search.connect("search-changed", lambda *_: self.fill())
        bar.pack_start(self.search, False, False, 0)
        for label, tip, fn in (("Export…", "Save all bookmarks as a file other browsers can import", self.export),
                               ("Import…", "Bring in bookmarks exported from another browser (HTML)", self.import_file),
                               ("New Folder", "Make a folder in the selected one", self.new_folder)):
            b = Gtk.Button(label=label)
            b.set_name("toolbtn")
            b.set_tooltip_text(tip)
            b.connect("clicked", lambda *_a, f=fn: f())
            bar.pack_end(b, False, False, 0)
        root.pack_start(bar, False, False, 0)
        body = Gtk.Paned()
        root.pack_start(body, True, True, 0)

        self.tree_store = Gtk.TreeStore(str, str)  # label, folder path
        self.tree = Gtk.TreeView(model=self.tree_store)
        self.tree.set_headers_visible(False)
        self.tree.append_column(Gtk.TreeViewColumn("", Gtk.CellRendererText(ellipsize=Pango.EllipsizeMode.END), text=0))
        self.tree.get_selection().connect("changed", self._on_folder_selected)
        self.tree.connect("button-press-event", self._on_folder_click)
        side = Gtk.ScrolledWindow()
        side.set_name("bm-side")
        side.set_size_request(220, -1)
        side.add(self.tree)
        body.pack1(side, False, False)

        self.list = Gtk.ListBox()
        self.list.connect("row-activated", lambda _l, row: self.open_url(row.url))
        right = Gtk.ScrolledWindow()
        right.add(self.list)
        body.pack2(right, True, False)
        self.status = Gtk.Label(xalign=0)
        self.status.set_name("bm-status")
        self.status.set_margin_start(10)
        self.status.set_margin_top(4)
        self.status.set_margin_bottom(4)
        root.pack_start(self.status, False, False, 0)
        self.connect("key-press-event", self._on_key)
        self.fill_folders()

    # ---------- folders ----------

    def fill_folders(self, select=None):
        self.tree_store.clear()
        rows = {}
        rows[ALL] = self.tree_store.append(None, ["All bookmarks", ALL])
        rows[NONE] = self.tree_store.append(None, ["Not in a folder", NONE])
        for f in self.bm.folders():
            parent = rows.get(f.rsplit("/", 1)[0] if "/" in f else None)
            rows[f] = self.tree_store.append(parent, [f.rsplit("/", 1)[-1], f])
        self.tree.expand_all()
        want = select if select is not None else self.folder
        it = rows.get(want, rows[ALL])
        self.tree.get_selection().select_iter(it)
        self.folder = self.tree_store[it][1]
        self.fill()

    def _on_folder_selected(self, sel):
        model, it = sel.get_selected()
        if it is not None:
            self.folder = model[it][1]
            self.fill()

    def _on_folder_click(self, tree, ev):
        if ev.button != 3:
            return False
        hit = tree.get_path_at_pos(int(ev.x), int(ev.y))
        if not hit:
            return False
        folder = self.tree_store[hit[0]][1]
        if folder in (ALL, NONE):
            return True
        tree.get_selection().select_path(hit[0])
        m = Gtk.Menu()
        for label, fn in (("Rename…", lambda: self.rename_folder(folder)), ("Delete…", lambda: self.delete_folder(folder))):
            it = Gtk.MenuItem(label=label)
            it.connect("activate", lambda *_a, f=fn: f())
            m.append(it)
        m.show_all()
        m.popup_at_pointer(ev)
        return True

    def new_folder(self):
        parent = "" if self.folder in (ALL, NONE) else self.folder
        name = self.ask("New Folder", "Name", "New folder")
        if name:
            f = self.bm.add_folder(name, parent)
            self.fill_folders(select=f)

    def rename_folder(self, folder):
        parent, _, leaf = folder.rpartition("/")
        name = self.ask("Rename Folder", "Name", leaf)
        if name and name != leaf:
            new = clean_folder(f"{parent}/{name}" if parent else name)
            self.bm.rename_folder(folder, new)
            self.changed()
            self.fill_folders(select=new)

    def delete_folder(self, folder):
        n = len([b for b in self.bm.items if b.get("folder", "") == folder or b.get("folder", "").startswith(folder + "/")])
        if n and not self.confirm(f"Delete the folder {folder.rsplit('/', 1)[-1]}?", f"The {n} bookmark{'s' if n != 1 else ''} in it go too."):
            return
        self.bm.delete_folder(folder)
        self.changed()
        self.fill_folders(select=ALL)

    # ---------- bookmarks ----------

    def fill(self):
        for c in self.list.get_children():
            c.destroy()
        q = self.search.get_text()
        rows = self.bm.in_folder(None if self.folder == ALL or q.strip() else self.folder, q)
        for b in rows:
            row = Gtk.ListBoxRow()
            row.url = b["url"]
            box = Gtk.Box(spacing=8)
            box.set_name("bm-row")
            col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            t = Gtk.Label(label=b["title"], xalign=0, ellipsize=Pango.EllipsizeMode.END)
            t.set_name("bm-title")
            u = Gtk.Label(label=b["url"], xalign=0, ellipsize=Pango.EllipsizeMode.END)
            u.set_name("bm-url")
            col.pack_start(t, False, False, 0)
            col.pack_start(u, False, False, 0)
            box.pack_start(col, True, True, 0)
            if self.folder == ALL or q.strip():
                f = Gtk.Label(label=b.get("folder") or "")
                f.set_name("bm-folder")
                box.pack_start(f, False, False, 0)
            for label, tip, fn in (("Edit", "Change the name, address or folder", lambda u=b["url"]: self.edit(u)),
                                   ("✕", "Delete", lambda u=b["url"]: self.delete(u))):
                btn = Gtk.Button(label=label)
                btn.set_name("toolbtn" if label == "Edit" else "tabclose")
                btn.set_tooltip_text(tip)
                btn.connect("clicked", lambda *_a, f=fn: f())
                box.pack_end(btn, False, False, 0)
            row.add(box)
            self.list.add(row)
        if not rows:
            empty = Gtk.Label(label="No matches." if q.strip() else "Nothing here yet. ☆ or Ctrl+D in the browser bookmarks a page; Import brings them from another browser.")
            empty.set_name("bm-url")
            empty.set_line_wrap(True)
            empty.set_margin_top(24)
            self.list.add(empty)
        self.list.show_all()
        self.status.set_text(f"{len(self.bm.items)} bookmarks in {len(self.bm.folders())} folders")

    def edit(self, url):
        b = self.bm.get(url)
        if b is None:
            return
        d = Gtk.Dialog(title="Edit Bookmark", transient_for=self, modal=True)
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Save", Gtk.ResponseType.OK)
        d.set_default_response(Gtk.ResponseType.OK)
        grid = Gtk.Grid(row_spacing=8, column_spacing=12)
        grid.set_border_width(16)
        title, address = Gtk.Entry(text=b["title"], width_chars=44), Gtk.Entry(text=b["url"])
        folder = Gtk.ComboBoxText.new_with_entry()
        folder.append_text("")
        for f in self.bm.folders():
            folder.append_text(f)
        folder.get_child().set_text(b.get("folder", ""))
        error = Gtk.Label(xalign=0)
        error.set_name("bm-url")
        for i, (label, w) in enumerate((("Name", title), ("Address", address), ("Folder", folder))):
            grid.attach(Gtk.Label(label=label, xalign=1), 0, i, 1, 1)
            grid.attach(w, 1, i, 1, 1)
            if isinstance(w, Gtk.Entry):
                w.set_activates_default(True)
        grid.attach(error, 1, 3, 1, 1)
        d.get_content_area().pack_start(grid, True, True, 0)
        d.show_all()
        while d.run() == Gtk.ResponseType.OK:
            new_url = address.get_text().strip()
            if not new_url.startswith(("https://", "http://")):
                error.set_text("The address must start with https:// or http://")
                continue
            if not self.bm.edit(url, title.get_text(), new_url, folder.get_child().get_text()):
                error.set_text("Another bookmark already has that address.")
                continue
            self.changed()
            break
        d.destroy()
        self.fill_folders()

    def delete(self, url):
        self.bm.remove(url)
        self.changed()
        self.fill()

    # ---------- import and export ----------

    def import_file(self, path=None):
        if path is None:
            d = Gtk.FileChooserDialog(title="Import Bookmarks", transient_for=self, action=Gtk.FileChooserAction.OPEN)
            d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Import", Gtk.ResponseType.OK)
            f = Gtk.FileFilter()
            f.set_name("Bookmark files (HTML)")
            for p in ("*.html", "*.htm", "*.HTML"):
                f.add_pattern(p)
            d.add_filter(f)
            path = d.get_filename() if d.run() == Gtk.ResponseType.OK else None
            d.destroy()
        if not path:
            return None
        try:
            r = webdata.run("import_bookmarks", path=path)
        except webdata.EngineError as e:
            self.status.set_text(str(e))
            return None
        before = len(r["items"])
        added = self.bm.add_many(r["items"])
        msg = f"Imported {added} bookmark{'s' if added != 1 else ''}"
        if before - added:
            msg += f"; {before - added} were already here"
        if r.get("skipped"):
            msg += f"; {r['skipped']} couldn't be used (scripts or local places)"
        self.changed()
        self.fill_folders()
        self.status.set_text(msg + ".")
        return added

    def export(self, path=None):
        if path is None:
            d = Gtk.FileChooserDialog(title="Export Bookmarks", transient_for=self, action=Gtk.FileChooserAction.SAVE)
            d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Export", Gtk.ResponseType.OK)
            d.set_do_overwrite_confirmation(True)
            d.set_current_name("bookmarks.html")
            docs = os.path.join(os.path.expanduser("~"), "Documents")
            if os.path.isdir(docs):
                d.set_current_folder(docs)
            path = d.get_filename() if d.run() == Gtk.ResponseType.OK else None
            d.destroy()
        if not path:
            return False
        try:
            r = webdata.run("export_bookmarks", path=path, items=self.bm.items)
        except webdata.EngineError as e:
            self.status.set_text(str(e))
            return False
        self.status.set_text(f"Saved {r['count']} bookmarks in {os.path.basename(path)}.")
        return True

    # ---------- small dialogs ----------

    def ask(self, title, label, default=""):
        d = Gtk.Dialog(title=title, transient_for=self, modal=True)
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "OK", Gtk.ResponseType.OK)
        d.set_default_response(Gtk.ResponseType.OK)
        e = Gtk.Entry(text=default, activates_default=True)
        box = d.get_content_area()
        box.set_border_width(16)
        box.set_spacing(8)
        box.pack_start(Gtk.Label(label=label, xalign=0), False, False, 0)
        box.pack_start(e, False, False, 0)
        d.show_all()
        r = d.run()
        text = e.get_text().strip().replace("/", "-")
        d.destroy()
        return text if r == Gtk.ResponseType.OK and text else None

    def confirm(self, text, secondary):
        d = Gtk.MessageDialog(transient_for=self, flags=0, message_type=Gtk.MessageType.QUESTION, buttons=Gtk.ButtonsType.NONE, text=text)
        d.format_secondary_text(secondary)
        d.add_buttons("Cancel", Gtk.ResponseType.CANCEL, "Delete", Gtk.ResponseType.OK)
        ok = d.run() == Gtk.ResponseType.OK
        d.destroy()
        return ok

    def _on_key(self, _w, ev):
        if ev.keyval == Gdk.KEY_Escape:
            self.close()
            return True
        if ev.state & Gdk.ModifierType.CONTROL_MASK and ev.keyval in (Gdk.KEY_f, Gdk.KEY_F):
            self.search.grab_focus()
            return True
        return False
