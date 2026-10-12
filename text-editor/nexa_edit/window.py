import os

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("GtkSource", "4")
from gi.repository import Gtk, Gdk, Gio, GtkSource, Pango

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#toolbar { background-image: none; background-color: @nexa_surface; }
#toolbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 4px 10px; }
#toolbtn:hover { background-image: none; background-color: @nexa_hover; }
#statusbar { background-image: none; background-color: @nexa_surface; color: @nexa_muted; font-size: 9pt; }
"""

DARK_SCHEMES = ("oblivion", "cobalt", "solarized-dark", "classic")


class EditorWindow(Gtk.Window):
    def __init__(self, path=None):
        super().__init__(title="Nexa Editor")
        self.set_icon_name("nexa-edit")
        attach_titlebar(self, "Nexa Editor")
        self.set_default_size(860, 620)
        self.path = os.path.abspath(path) if path else None
        self.modified = False

        self._build_ui()
        if self.path and os.path.isfile(self.path):
            self._load(self.path)
        else:
            self._update_title()
        self.connect("delete-event", self._on_delete)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_name("toolbar")
        toolbar.set_border_width(8)
        for label, handler in (
            ("New", self._on_new), ("Open", self._on_open),
            ("Save", self._on_save), ("Save As", self._on_save_as),
        ):
            btn = Gtk.Button(label=label)
            btn.set_name("toolbtn")
            btn.connect("clicked", handler)
            toolbar.pack_start(btn, False, False, 0)
        root.pack_start(toolbar, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        lang_manager = GtkSource.LanguageManager.get_default()
        self.buffer = GtkSource.Buffer()
        scheme_manager = GtkSource.StyleSchemeManager.get_default()
        for name in DARK_SCHEMES:
            scheme = scheme_manager.get_scheme(name)
            if scheme:
                self.buffer.set_style_scheme(scheme)
                break
        self.buffer.connect("changed", self._on_buffer_changed)

        self.view = GtkSource.View(buffer=self.buffer)
        self.view.set_show_line_numbers(True)
        self.view.set_highlight_current_line(True)
        self.view.set_tab_width(4)
        self.view.set_insert_spaces_instead_of_tabs(True)
        self.view.set_monospace(True)
        self.view.override_font(Pango.FontDescription("monospace 11"))
        self.view.get_buffer().connect("notify::cursor-position", self._on_cursor_moved)
        scroller.add(self.view)
        root.pack_start(scroller, True, True, 0)

        self.status = Gtk.Label(label="")
        self.status.set_name("statusbar")
        self.status.set_xalign(0)
        self.status.set_margin_start(10)
        self.status.set_margin_top(4)
        self.status.set_margin_bottom(4)
        root.pack_start(self.status, False, False, 0)

        self._lang_manager = lang_manager

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

        accels = Gtk.AccelGroup()
        self.add_accel_group(accels)
        for key, handler in (
            ("n", self._on_new), ("o", self._on_open),
            ("s", self._on_save),
        ):
            accels.connect(Gdk.keyval_from_name(key), Gdk.ModifierType.CONTROL_MASK, 0,
                            lambda *a, h=handler: h(None))
        accels.connect(Gdk.keyval_from_name("s"), Gdk.ModifierType.CONTROL_MASK | Gdk.ModifierType.SHIFT_MASK, 0,
                        lambda *a: self._on_save_as(None))

    def _on_cursor_moved(self, buf, _pspec):
        it = buf.get_iter_at_mark(buf.get_insert())
        self._set_status(it.get_line() + 1, it.get_line_offset() + 1)

    def _set_status(self, line=1, col=1):
        name = self.path or "untitled"
        mark = " •" if self.modified else ""
        self.status.set_text(f"{name}{mark}    Ln {line}, Col {col}")

    def _update_title(self):
        name = os.path.basename(self.path) if self.path else "untitled"
        mark = "• " if self.modified else ""
        self.set_title(f"{mark}{name} — Nexa Editor")
        self._set_status()

    def _on_buffer_changed(self, _buf):
        if not self.modified:
            self.modified = True
            self._update_title()

    def _guess_language(self, path):
        content_type, _ = Gio.content_type_guess(path, None)
        return self._lang_manager.guess_language(path, content_type)

    def _load(self, path):
        try:
            with open(path, "r", errors="replace") as f:
                text = f.read()
        except OSError as e:
            self._error(f"Couldn't open {path}: {e}")
            return
        self.buffer.begin_not_undoable_action()
        self.buffer.set_text(text)
        self.buffer.end_not_undoable_action()
        self.buffer.set_language(self._guess_language(path))
        self.path = os.path.abspath(path)
        self.modified = False
        self._update_title()

    def _save_to(self, path):
        start, end = self.buffer.get_bounds()
        text = self.buffer.get_text(start, end, True)
        try:
            with open(path, "w") as f:
                f.write(text)
        except OSError as e:
            self._error(f"Couldn't save {path}: {e}")
            return False
        self.path = os.path.abspath(path)
        self.modified = False
        self.buffer.set_language(self._guess_language(self.path))
        self._update_title()
        return True

    def _on_new(self, _btn):
        if not self._confirm_discard():
            return
        self.buffer.set_text("")
        self.buffer.set_language(None)
        self.path = None
        self.modified = False
        self._update_title()

    def _on_open(self, _btn):
        if not self._confirm_discard():
            return
        dialog = Gtk.FileChooserDialog(
            title="Open File", transient_for=self, action=Gtk.FileChooserAction.OPEN,
        )
        dialog.add_buttons(Gtk.STOCK_CANCEL, Gtk.ResponseType.CANCEL, Gtk.STOCK_OPEN, Gtk.ResponseType.OK)
        if self.path:
            dialog.set_current_folder(os.path.dirname(self.path))
        if dialog.run() == Gtk.ResponseType.OK:
            self._load(dialog.get_filename())
        dialog.destroy()

    def _on_save(self, _btn):
        if self.path:
            self._save_to(self.path)
        else:
            self._on_save_as(None)

    def _on_save_as(self, _btn):
        dialog = Gtk.FileChooserDialog(
            title="Save As", transient_for=self, action=Gtk.FileChooserAction.SAVE,
        )
        dialog.add_buttons(Gtk.STOCK_CANCEL, Gtk.ResponseType.CANCEL, Gtk.STOCK_SAVE, Gtk.ResponseType.OK)
        dialog.set_do_overwrite_confirmation(True)
        if self.path:
            dialog.set_current_folder(os.path.dirname(self.path))
            dialog.set_current_name(os.path.basename(self.path))
        else:
            dialog.set_current_name("untitled.txt")
        if dialog.run() == Gtk.ResponseType.OK:
            self._save_to(dialog.get_filename())
        dialog.destroy()

    def _confirm_discard(self):
        if not self.modified:
            return True
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.WARNING,
            buttons=Gtk.ButtonsType.NONE, text="Save changes before continuing?",
        )
        dialog.format_secondary_text(f"{os.path.basename(self.path) if self.path else 'untitled'} has unsaved changes.")
        dialog.add_buttons(
            "Discard", Gtk.ResponseType.NO, "Cancel", Gtk.ResponseType.CANCEL,
            "Save", Gtk.ResponseType.YES,
        )
        response = dialog.run()
        dialog.destroy()
        if response == Gtk.ResponseType.YES:
            self._on_save(None)
            return not self.modified
        return response == Gtk.ResponseType.NO

    def _on_delete(self, _widget, _event):
        return not self._confirm_discard()

    def _error(self, message):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.ERROR,
            buttons=Gtk.ButtonsType.OK, text=message,
        )
        dialog.run()
        dialog.destroy()


def main(argv):
    Gtk.init()
    path = argv[0] if argv else None
    win = EditorWindow(path)
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
