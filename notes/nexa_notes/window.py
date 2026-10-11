import os
import time

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: #1e1e1e; }
#sidebar { background-image: none; background-color: #232323; }
#notebtn { background-image: none; background-color: transparent; color: #d4d4d4; border-radius: 6px; padding: 8px 10px; }
#notebtn:hover { background-image: none; background-color: #2f2f2f; }
#notebtn.active { background-image: none; background-color: #2c3a52; color: #dbe4f5; }
#toolbar { background-image: none; background-color: #252525; }
#toolbtn { background-image: none; background-color: #2f2f2f; color: #d4d4d4; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: #3a3a3a; }
textview { background-image: none; background-color: #1e1e1e; color: #e0e0e0; font-size: 11pt; }
textview text { background-image: none; background-color: #1e1e1e; }
"""

NOTES_DIR = os.path.join(os.environ.get("HOME", "/home/user"), ".local", "share", "nexa-notes")


class NotesWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Nexa Notes")
        self.set_icon_name("nexa-notes")
        attach_titlebar(self, "Nexa Notes")
        self.set_default_size(760, 540)

        os.makedirs(NOTES_DIR, exist_ok=True)
        self.current_file = None
        self.note_buttons = {}

        self._build_ui()
        self._refresh_list()

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=0)
        self.add(root)

        sidebar = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        sidebar.set_name("sidebar")
        sidebar.set_size_request(200, -1)
        sidebar.set_border_width(8)

        new_btn = Gtk.Button(label="+ New Note")
        new_btn.set_name("toolbtn")
        new_btn.connect("clicked", self._on_new)
        sidebar.pack_start(new_btn, False, False, 4)

        list_scroller = Gtk.ScrolledWindow()
        self.list_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        list_scroller.add(self.list_box)
        sidebar.pack_start(list_scroller, True, True, 0)
        root.pack_start(sidebar, False, False, 0)

        editor_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        save_btn = Gtk.Button(label="Save")
        save_btn.set_name("toolbtn")
        save_btn.connect("clicked", self._on_save)
        delete_btn = Gtk.Button(label="Delete")
        delete_btn.set_name("toolbtn")
        delete_btn.connect("clicked", self._on_delete)
        toolbar.pack_start(save_btn, False, False, 0)
        toolbar.pack_start(delete_btn, False, False, 0)
        editor_box.pack_start(toolbar, False, False, 0)

        text_scroller = Gtk.ScrolledWindow()
        self.textview = Gtk.TextView()
        self.textview.set_wrap_mode(Gtk.WrapMode.WORD)
        self.textview.set_border_width(12)
        self.textbuffer = self.textview.get_buffer()
        text_scroller.add(self.textview)
        editor_box.pack_start(text_scroller, True, True, 0)
        root.pack_start(editor_box, True, True, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _refresh_list(self):
        for child in self.list_box.get_children():
            self.list_box.remove(child)
        self.note_buttons.clear()

        names = sorted(
            n for n in os.listdir(NOTES_DIR) if n.endswith(".txt")
        ) if os.path.isdir(NOTES_DIR) else []
        for name in names:
            path = os.path.join(NOTES_DIR, name)
            title = self._first_line(path) or name[:-4]
            btn = Gtk.Button(label=title)
            btn.set_name("notebtn")
            btn.get_child().set_xalign(0)
            btn.get_child().set_ellipsize(3)
            btn.connect("clicked", self._on_select, path)
            self.list_box.pack_start(btn, False, False, 0)
            self.note_buttons[path] = btn
        self.list_box.show_all()

        if names and not self.current_file:
            self._on_select(None, os.path.join(NOTES_DIR, names[0]))
        elif not names:
            self.current_file = None
            self.textbuffer.set_text("")
        else:
            self._mark_active(self.current_file)

    def _first_line(self, path):
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                return f.readline().strip()
        except OSError:
            return None

    def _mark_active(self, path):
        for p, btn in self.note_buttons.items():
            ctx = btn.get_style_context()
            if p == path:
                ctx.add_class("active")
            else:
                ctx.remove_class("active")

    def _on_select(self, _btn, path):
        self._save_current()
        self.current_file = path
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                content = f.read()
        except OSError:
            content = ""
        self.textbuffer.set_text(content)
        self._mark_active(path)

    def _save_current(self):
        if not self.current_file:
            return
        start, end = self.textbuffer.get_bounds()
        text = self.textbuffer.get_text(start, end, True)
        try:
            with open(self.current_file, "w", encoding="utf-8") as f:
                f.write(text)
        except OSError:
            pass

    def _on_save(self, _btn):
        self._save_current()
        self._refresh_list()

    def _on_new(self, _btn):
        self._save_current()
        name = f"note-{int(time.time())}.txt"
        path = os.path.join(NOTES_DIR, name)
        with open(path, "w", encoding="utf-8") as f:
            f.write("")
        self.current_file = path
        self.textbuffer.set_text("")
        self._refresh_list()
        self.textview.grab_focus()

    def _on_delete(self, _btn):
        if not self.current_file:
            return
        try:
            os.remove(self.current_file)
        except OSError:
            pass
        self.current_file = None
        self._refresh_list()


def main():
    Gtk.init()
    win = NotesWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
