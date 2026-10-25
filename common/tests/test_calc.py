"""Calculator logic, pressed key by key without a screen (the window's
display and operator line are stand-ins)."""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "calculator"))
sys.path.insert(0, os.path.join(HERE, ".."))
try:
    import gi
    gi.require_version("Gtk", "3.0")
    from gi.repository import Gtk  # noqa: F401
except (ImportError, ValueError):
    # No GTK on this machine: the logic doesn't need it, so stand-ins do.
    import types
    gi = types.ModuleType("gi")
    gi.require_version = lambda *_a: None
    repo = types.ModuleType("gi.repository")
    repo.Gtk = types.SimpleNamespace(Window=object)
    repo.Gdk = types.SimpleNamespace()
    sys.modules.update({"gi": gi, "gi.repository": repo})
    sys.modules["nexa_common.titlebar"] = types.SimpleNamespace(attach_titlebar=lambda *_a: None)
from nexa_calc.window import CalcWindow  # noqa: E402

failures = []


def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond:
        failures.append(label)


class Label:
    def __init__(self):
        self.text = ""

    def set_text(self, t):
        self.text = t


def calc():
    c = object.__new__(CalcWindow)
    c.display, c.op_label = Label(), Label()
    c._on_clear()
    return c


def press(c, keys):
    for k in keys.split():
        if k.isdigit():
            for d in k:
                c._on_digit(d)
        elif k == ".":
            c._on_decimal(None)
        elif k == "±":
            c._on_sign(None)
        elif k == "=":
            c._on_equals(None)
        elif k == "⌫":
            c._on_backspace(None)
        else:
            c._on_operator(k)
    return c.display.text


check("12 + 30 = 42", press(calc(), "12 + 30 =") == "42")
check("chained: 2 + 3 × 4 = 20 (left to right)", press(calc(), "2 + 3 × 4 =") == "20")
check("1 ÷ 0 says so, and the next digit starts over", press(calc(), "1 ÷ 0 =") == "Cannot divide by zero")
c = calc(); press(c, "1 ÷ 0 ="); check("... then 7 shows 7", press(c, "7") == "7")
check("0.1 + 0.2 = 0.3", press(calc(), "0 . 1 + 0 . 2 =") == "0.3")
c = calc()
try:
    out = press(c, "5 ± ⌫ + 2 =")
    check("backspace on -5 leaves 0, not a lone minus", out == "2", out)
except ValueError as e:
    check("backspace on -5 leaves 0, not a lone minus", False, str(e))
c = calc()
try:
    out = press(c, "1" + "0" * 200 + " × " + "1" + "0" * 200 + " =")
    check("an overflowing result is an error, not a crash", out == "Number too large", out)
except OverflowError as e:
    check("an overflowing result is an error, not a crash", False, str(e))
check("± right after an operator: 5 + ± 3 = 2", press(calc(), "5 + ± 3 =") == "2")

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
