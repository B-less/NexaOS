import math
import operator

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk

from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#display { background-image: none; background-color: @nexa_bg; color: @nexa_text_strong; font-size: 36pt; font-weight: 300; }
#opdisplay { background-image: none; background-color: @nexa_bg; color: @nexa_muted; font-size: 11pt; }
#calcbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; font-size: 16pt; border-radius: 8px; }
#calcbtn:hover { background-image: none; background-color: @nexa_hover; }
#opbtn { background-image: none; background-color: @nexa_hover; color: @nexa_accent; font-size: 18pt; font-weight: bold; border-radius: 8px; }
#opbtn:hover { background-image: none; background-color: @nexa_hover; }
#eqbtn { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; font-size: 18pt; font-weight: bold; border-radius: 8px; }
#eqbtn:hover { background-image: none; background-color: @nexa_accent_hover; }
#clearbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_crit; font-size: 14pt; border-radius: 8px; }
#clearbtn:hover { background-image: none; background-color: @nexa_hover; }
"""

OPS = {
    "+": operator.add, "−": operator.sub, "×": operator.mul, "÷": operator.truediv,
}


class CalcWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Calculator")
        self.set_icon_name("nexa-calc")
        attach_titlebar(self, "Calculator")
        self.set_default_size(320, 480)
        self.set_resizable(False)

        self.display_text = "0"
        self.accumulator = None
        self.pending_op = None
        self.reset_on_digit = False
        self.error = False

        self._build_ui()
        self.connect("key-press-event", self._on_key)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        display_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        display_box.set_border_width(16)
        self.op_label = Gtk.Label(label="")
        self.op_label.set_name("opdisplay")
        self.op_label.set_xalign(1)
        display_box.pack_start(self.op_label, False, False, 0)

        self.display = Gtk.Label(label="0")
        self.display.set_name("display")
        self.display.set_xalign(1)
        self.display.set_ellipsize(3)
        display_box.pack_start(self.display, False, False, 0)
        root.pack_start(display_box, False, False, 0)

        grid = Gtk.Grid(row_spacing=8, column_spacing=8)
        grid.set_border_width(12)
        grid.set_row_homogeneous(True)
        grid.set_column_homogeneous(True)
        root.pack_start(grid, True, True, 0)

        rows = [
            [("C", "clearbtn", self._on_clear), ("CE", "clearbtn", self._on_clear_entry),
             ("⌫", "clearbtn", self._on_backspace), ("÷", "opbtn", lambda *_: self._on_operator("÷"))],
            [("7", "calcbtn", lambda *_: self._on_digit("7")), ("8", "calcbtn", lambda *_: self._on_digit("8")),
             ("9", "calcbtn", lambda *_: self._on_digit("9")), ("×", "opbtn", lambda *_: self._on_operator("×"))],
            [("4", "calcbtn", lambda *_: self._on_digit("4")), ("5", "calcbtn", lambda *_: self._on_digit("5")),
             ("6", "calcbtn", lambda *_: self._on_digit("6")), ("−", "opbtn", lambda *_: self._on_operator("−"))],
            [("1", "calcbtn", lambda *_: self._on_digit("1")), ("2", "calcbtn", lambda *_: self._on_digit("2")),
             ("3", "calcbtn", lambda *_: self._on_digit("3")), ("+", "opbtn", lambda *_: self._on_operator("+"))],
            [("±", "calcbtn", self._on_sign), ("0", "calcbtn", lambda *_: self._on_digit("0")),
             (".", "calcbtn", self._on_decimal), ("=", "eqbtn", self._on_equals)],
        ]
        for r, row in enumerate(rows):
            for c, (label, css_name, handler) in enumerate(row):
                btn = Gtk.Button(label=label)
                btn.set_name(css_name)
                btn.connect("clicked", handler)
                grid.attach(btn, c, r, 1, 1)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _render(self):
        self.display.set_text(self.display_text)
        if self.pending_op and not self.error:
            self.op_label.set_text(f"{_fmt(self.accumulator)} {self.pending_op}")
        else:
            self.op_label.set_text("")

    def _on_digit(self, d):
        if self.error:
            self._on_clear()
        if self.reset_on_digit or self.display_text == "0":
            self.display_text = d
            self.reset_on_digit = False
        elif self.display_text == "-0":  # ± pressed before the number
            self.display_text = "-" + d
        else:
            self.display_text += d
        self._render()

    def _on_decimal(self, _btn):
        if self.error:
            self._on_clear()
        if self.reset_on_digit:
            self.display_text = "0"
            self.reset_on_digit = False
        if "." not in self.display_text:
            self.display_text += "."
        self._render()

    def _on_sign(self, _btn):
        if self.error:
            return
        if self.reset_on_digit:
            # Right after an operator: the next number is negative.
            self.display_text = "-0"
            self.reset_on_digit = False
        elif self.display_text.startswith("-"):
            self.display_text = self.display_text[1:]
        elif self.display_text != "0":
            self.display_text = "-" + self.display_text
        self._render()

    def _on_operator(self, op):
        if self.error:
            return
        if self.pending_op and not self.reset_on_digit:
            self._compute()
            if self.error:
                return
        else:
            self.accumulator = float(self.display_text)
        self.pending_op = op
        self.reset_on_digit = True
        self._render()

    def _on_equals(self, _btn):
        if self.error or not self.pending_op:
            return
        self._compute()
        self.pending_op = None
        self.reset_on_digit = True
        self._render()

    def _compute(self):
        try:
            result = OPS[self.pending_op](self.accumulator, float(self.display_text))
        except ZeroDivisionError:
            self.display_text = "Cannot divide by zero"
            self.error = True
            self.accumulator = None
            self.pending_op = None
            self._render()
            return
        if not math.isfinite(result):
            self.display_text = "Number too large"
            self.error = True
            self.accumulator = None
            self.pending_op = None
            self._render()
            return
        self.accumulator = result
        self.display_text = _fmt(result)

    def _on_clear(self, _btn=None):
        self.display_text = "0"
        self.accumulator = None
        self.pending_op = None
        self.reset_on_digit = False
        self.error = False
        self._render()

    def _on_clear_entry(self, _btn):
        if self.error:
            self._on_clear()
            return
        self.display_text = "0"
        self._render()

    def _on_backspace(self, _btn):
        if self.error:
            self._on_clear()
            return
        rest = self.display_text[:-1]
        # "-5" minus a digit is just "-", which isn't a number.
        self.display_text = rest if rest not in ("", "-") else "0"
        self._render()

    def _on_key(self, _widget, event):
        key = Gdk.keyval_name(event.keyval)
        digit_keys = {f"KP_{i}": str(i) for i in range(10)}
        digit_keys.update({str(i): str(i) for i in range(10)})
        if key in digit_keys:
            self._on_digit(digit_keys[key])
        elif key in ("plus", "KP_Add"):
            self._on_operator("+")
        elif key in ("minus", "KP_Subtract"):
            self._on_operator("−")
        elif key in ("asterisk", "KP_Multiply"):
            self._on_operator("×")
        elif key in ("slash", "KP_Divide"):
            self._on_operator("÷")
        elif key in ("Return", "KP_Enter", "equal"):
            self._on_equals(None)
        elif key in ("period", "KP_Decimal"):
            self._on_decimal(None)
        elif key == "BackSpace":
            self._on_backspace(None)
        elif key in ("Escape", "Delete"):
            self._on_clear()
        return True


def _fmt(value):
    if value == int(value) and abs(value) < 1e15:
        return str(int(value))
    return f"{value:.10g}"


def main():
    Gtk.init()
    win = CalcWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
