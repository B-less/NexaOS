import json
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib, Gdk

from . import agent, client, local_fallback, tools
from .cli import load_config, make_logger
from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#header { background-image: none; background-color: @nexa_surface; color: @nexa_text; }
#status { color: @nexa_muted; font-size: 9pt; }
#scroller { background-image: none; background-color: @nexa_bg; }
#entry { background-image: none; background-color: @nexa_field; color: @nexa_text_strong; border-radius: 8px; padding: 8px; border: 1px solid @nexa_border; }
#send { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 8px; padding: 6px 14px; font-weight: bold; }
#send:hover { background-image: none; background-color: @nexa_accent_hover; }
#control-toggle { border-radius: 12px; padding: 4px 12px; font-weight: bold; font-size: 9pt; }
#control-toggle.control-off { background-image: none; background-color: @nexa_hover; color: @nexa_muted; }
#control-toggle.control-on { background-image: none; background-color: @nexa_crit; color: @nexa_on_accent; }

.bubble { border-radius: 14px; padding: 8px 12px; }
.bubble-user { background-image: none; background-color: @nexa_selected; color: @nexa_selected_text; }
.bubble-ai { background-image: none; background-color: mix(@nexa_surface_hi, @nexa_ok, 0.14); color: @nexa_text_strong; }
.bubble-sys { background-image: none; background-color: transparent; color: @nexa_muted; font-style: italic; }
.bubble-tool { background-image: none; background-color: @nexa_surface; color: @nexa_muted; font-family: monospace; font-size: 9.5pt; border: 1px solid @nexa_border; }
.bubble-tool-error { color: @nexa_crit; }
.bubble-thinking { background-image: none; background-color: transparent; color: @nexa_disabled; font-style: italic; }
"""

ROLE_STYLE = {
    "user": ("bubble-user", Gtk.Align.END, 48),
    "ai": ("bubble-ai", Gtk.Align.START, 56),
    "sys": ("bubble-sys", Gtk.Align.START, 56),
}


class NexaAssistantWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="NexaOS Assistant")
        self.set_icon_name("nexa-ai")
        attach_titlebar(self, "NexaOS Assistant")
        self.set_default_size(640, 720)
        self.cfg = load_config()
        self.history = []
        self.log = make_logger()
        self._pending_tool_labels = []
        self._thinking_row = None

        self._build_ui()
        self._add_bubble("NexaOS Assistant ready. Toggle Control to let it act on the machine.", "sys")
        self.set_focus(self.entry)

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        header.set_name("header")
        header.set_border_width(10)
        title = Gtk.Label(label="NexaOS Assistant")
        title.set_xalign(0)
        header.pack_start(title, True, True, 0)

        self.control_switch = Gtk.ToggleButton()
        self.control_switch.set_name("control-toggle")
        self.control_switch.set_active(bool(self.cfg.get("allow_control")))
        self.control_switch.connect("toggled", self._on_control_toggled)
        self._update_control_label()
        header.pack_start(self.control_switch, False, False, 0)
        root.pack_start(header, False, False, 0)

        self.status = Gtk.Label(label=self._status_text())
        self.status.set_name("status")
        self.status.set_xalign(0)
        self.status.set_margin_start(10)
        root.pack_start(self.status, False, False, 4)

        scroller = Gtk.ScrolledWindow()
        scroller.set_name("scroller")
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.message_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        self.message_box.set_border_width(10)
        scroller.add(self.message_box)
        self._adjustment = scroller.get_vadjustment()
        root.pack_start(scroller, True, True, 0)

        entry_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        entry_row.set_border_width(10)
        self.entry = Gtk.Entry()
        self.entry.set_name("entry")
        self.entry.set_placeholder_text("Ask about this machine, or tell it what to do…")
        self.entry.connect("activate", self._on_send)
        entry_row.pack_start(self.entry, True, True, 0)
        send_btn = Gtk.Button(label="Send")
        send_btn.set_name("send")
        send_btn.connect("clicked", self._on_send)
        entry_row.pack_start(send_btn, False, False, 0)
        root.pack_start(entry_row, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _on_control_toggled(self, _button):
        self._update_control_label()

    def _update_control_label(self):
        on = self.control_switch.get_active()
        self.control_switch.set_label("Control: ON" if on else "Control: OFF")
        ctx = self.control_switch.get_style_context()
        ctx.remove_class("control-on")
        ctx.remove_class("control-off")
        ctx.add_class("control-on" if on else "control-off")

    def _status_text(self):
        mode = "cloud (Claude)" if self.cfg.get("api_key") else "offline — no API key configured"
        return f"backend: {mode}   ·   model: {self.cfg.get('model')}"

    def _add_bubble(self, text, role, extra_class=None):
        css_class, halign, max_chars = ROLE_STYLE[role]
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        row.set_halign(halign)
        label = Gtk.Label(label=text)
        label.set_line_wrap(True)
        label.set_max_width_chars(max_chars)
        label.set_xalign(0)
        label.set_selectable(True)
        ctx = label.get_style_context()
        ctx.add_class("bubble")
        ctx.add_class(css_class)
        if extra_class:
            ctx.add_class(extra_class)
        row.pack_start(label, False, False, 0)
        self.message_box.pack_start(row, False, False, 0)
        row.show_all()
        GLib.idle_add(self._scroll_to_bottom)
        return label

    def _scroll_to_bottom(self):
        self._adjustment.set_value(self._adjustment.get_upper())
        return False

    def _clear_thinking(self):
        if self._thinking_row is not None:
            self._thinking_row.destroy()
            self._thinking_row = None

    def _on_send(self, _widget):
        query = self.entry.get_text().strip()
        if not query:
            return
        self.entry.set_text("")
        self.entry.set_sensitive(False)
        self._add_bubble(query, "user")
        self._pending_tool_labels = []
        thinking = self._add_bubble("thinking…", "sys", extra_class="bubble-thinking")
        self._thinking_row = thinking.get_parent()
        threading.Thread(target=self._worker, args=(query,), daemon=True).start()

    def _on_agent_event(self, kind, payload):
        self._clear_thinking()
        if kind == "text":
            self._add_bubble(payload, "ai")
        elif kind == "tool_call":
            arg_str = ", ".join(f"{k}={v}" for k, v in (payload["args"] or {}).items())
            label = self._add_bubble(f"→ {payload['name']}({arg_str})", "sys", extra_class="bubble-tool")
            self._pending_tool_labels.append(label)
        elif kind == "tool_result":
            if self._pending_tool_labels:
                label = self._pending_tool_labels.pop(0)
                if payload["error"]:
                    label.set_text(label.get_text() + f"\n✗ {payload['error']}")
                    label.get_style_context().add_class("bubble-tool-error")
                else:
                    label.set_text(label.get_text() + f"\n✓ {_short_result(payload['result'])}")
        return False

    def _emit(self, kind, payload):
        GLib.idle_add(self._on_agent_event, kind, payload)

    def _worker(self, query):
        allow_control = self.control_switch.get_active()
        try:
            if not self.cfg.get("api_key"):
                self._emit("text", local_fallback.answer(query))
            else:
                self.history = agent.run(
                    self.cfg["api_key"], self.cfg["model"], query, allow_control,
                    self.cfg["require_confirmation"], self._confirm_blocking, self.log,
                    self._emit, history=self.history,
                )
        except (client.NetworkError, client.ApiError) as e:
            self._emit("text", f"(cloud AI unavailable: {e})")
            self._emit("text", local_fallback.answer(query))
        except Exception as e:  # noqa: BLE001 - surface any unexpected failure to the chat window
            self._emit("text", f"internal error: {e}")

        GLib.idle_add(self._finish_worker)

    def _finish_worker(self):
        self._clear_thinking()
        self.entry.set_sensitive(True)
        self.entry.grab_focus()
        return False

    def _confirm_blocking(self, name, args):
        event = threading.Event()
        answer = {"ok": False}

        def ask():
            arg_str = ", ".join(f"{k}={v}" for k, v in (args or {}).items())
            # MessageType.WARNING pulls in a stock icon through the same broken
            # PNG-loading path as the switch icon above; OTHER renders no icon.
            dialog = Gtk.MessageDialog(
                transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
                buttons=Gtk.ButtonsType.YES_NO,
                text=f"Run '{name}({arg_str})'?",
            )
            dialog.format_secondary_text("This changes real system state.")
            response = dialog.run()
            dialog.destroy()
            answer["ok"] = response == Gtk.ResponseType.YES
            event.set()
            return False

        GLib.idle_add(ask)
        event.wait()
        return answer["ok"]


def _short_result(result, limit=400):
    text = result if isinstance(result, str) else json.dumps(result, indent=2)
    return text if len(text) <= limit else text[:limit] + "…"


def main():
    import sys
    Gtk.init()
    win = NexaAssistantWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    # `nexa-ai-gui --ask "question"` (from Nexus's search) asks straight away.
    if "--ask" in sys.argv[1:]:
        i = sys.argv.index("--ask")
        question = " ".join(sys.argv[i + 1:]).strip()
        if question:
            win.entry.set_text(question)
            GLib.idle_add(lambda: win._on_send(None) and False)
    Gtk.main()


if __name__ == "__main__":
    main()
