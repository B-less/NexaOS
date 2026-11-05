import json
import os
import subprocess
import sys
import tempfile
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GdkPixbuf, GLib

from nexa_common.titlebar import attach_titlebar
from . import packaging
from nexa_common import askpass

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#namelabel { color: @nexa_text_strong; font-size: 18pt; font-weight: bold; }
#metalabel { color: @nexa_muted; font-size: 10pt; }
#desclabel { color: @nexa_text; font-size: 11pt; }
#installbtn { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 6px; padding: 10px 24px; font-weight: bold; font-size: 12pt; }
#installbtn:hover { background-image: none; background-color: @nexa_accent_hover; }
#installbtn:disabled { background-image: none; background-color: @nexa_hover; color: @nexa_disabled; }
#statuslabel { color: @nexa_ok; font-size: 10pt; }
#errorlabel { color: @nexa_crit; font-size: 10pt; }
#trusted { color: @nexa_ok; font-size: 10pt; font-weight: bold; }
#untrusted { color: @nexa_warn; font-size: 10pt; font-weight: bold; }
#detail { color: @nexa_muted; font-size: 9.5pt; }
"""

NEXA_PACK = "/usr/local/bin/nexa-pack"
KEYS_DIR = "/etc/nexa/keys"
PERMISSION_TEXT = {
    "network": "the internet", "files.home": "your home folder", "files.documents": "Documents",
    "files.pictures": "Pictures", "files.music": "Music", "files.videos": "Videos",
    "files.drives": "USB sticks and other drives",
    "camera": "the camera", "microphone": "the microphone", "location": "your location",
    "notifications": "notifications", "background": "running in the background",
}


def inspect(path):
    """nexa-pack's report on a package (verification needs no root)."""
    try:
        r = subprocess.run([NEXA_PACK, "verify", path, "--keys", KEYS_DIR, "--json"],
                           capture_output=True, text=True, timeout=60)
        report = json.loads(r.stdout.strip().splitlines()[-1])
    except FileNotFoundError:
        return {"ok": False, "error": "nexa-pack (the package verifier) is not installed"}
    except (ValueError, IndexError, subprocess.SubprocessError):
        return {"ok": False, "error": "couldn't read this package"}
    return report


# Installing and uninstalling go through the system broker (nexa-brokerd,
# Rust): it reads the package with your permissions, has nexa-pack verify
# it, places only the app's own files, asks your password on an installed
# system, and logs it.
BROKER_ACTIONS = {"install": "install_app", "uninstall": "uninstall_app"}


def _call_helper(action, **args):
    """(ok, message, unverified): unverified means the broker refused only
    because the publisher isn't trusted, so the user may choose to go on."""
    from nexa_common import broker

    def ask_password(what, error):
        return askpass.ask(f"{what or 'Installing an app'} needs your password.")(error is not None)

    try:
        result = broker.run(BROKER_ACTIONS[action], args, ask_password)
    except broker.Unverified as e:
        return False, str(e), True
    except broker.BrokerError as e:
        return False, "Cancelled." if str(e) == "cancelled" else str(e), False
    return True, (result or {}).get("done", "") if isinstance(result, dict) else "", False


class InstallerWindow(Gtk.Window):
    def __init__(self, nexa_path):
        super().__init__(title="Nexa Installer")
        self.set_icon_name("nexa-installer")
        attach_titlebar(self, "Nexa Installer")
        self.set_default_size(440, 360)
        self.set_resizable(False)

        self.nexa_path = nexa_path
        self.manifest = None
        self._icon_tmp = None

        self._build_ui()
        self._load()

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        root.set_border_width(24)
        self.add(root)

        self.icon_image = Gtk.Image()
        self.icon_image.set_pixel_size(96)
        root.pack_start(self.icon_image, False, False, 0)

        self.name_label = Gtk.Label(label="")
        self.name_label.set_name("namelabel")
        root.pack_start(self.name_label, False, False, 0)

        self.meta_label = Gtk.Label(label="")
        self.meta_label.set_name("metalabel")
        root.pack_start(self.meta_label, False, False, 0)

        self.desc_label = Gtk.Label(label="")
        self.desc_label.set_name("desclabel")
        self.desc_label.set_line_wrap(True)
        self.desc_label.set_justify(Gtk.Justification.CENTER)
        root.pack_start(self.desc_label, False, False, 0)

        self.trust_label = Gtk.Label(label="")
        root.pack_start(self.trust_label, False, False, 0)
        self.detail_label = Gtk.Label(label="")
        self.detail_label.set_name("detail")
        self.detail_label.set_line_wrap(True)
        self.detail_label.set_justify(Gtk.Justification.CENTER)
        root.pack_start(self.detail_label, False, False, 0)

        self.install_btn = Gtk.Button(label="Install")
        self.install_btn.set_name("installbtn")
        self.install_btn.set_sensitive(False)
        self.install_btn.connect("clicked", self._on_install)
        root.pack_start(self.install_btn, False, False, 8)

        self.status_label = Gtk.Label(label="")
        root.pack_start(self.status_label, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _load(self):
        report = inspect(self.nexa_path)
        if not report.get("ok"):
            self._set_status(f"This package can't be installed: {report.get('error')}", error=True)
            return
        self.manifest = report

        self.name_label.set_text(report["name"])
        size_mb = report.get("size", 0) / 1024 / 1024
        self.meta_label.set_text(f"{report['id']}  ·  v{report['version']}  ·  {size_mb:.1f} MB")
        self.desc_label.set_text(report.get("description", ""))
        if report["trust"] == "trusted":
            self.trust_label.set_name("trusted")
            self.trust_label.set_text(f"Verified publisher ({report['publisher']})")
        else:
            self.trust_label.set_name("untrusted")
            self.trust_label.set_text("Not signed by a publisher you trust" if report["trust"] == "unknown-publisher"
                                      else "Not signed: the publisher can't be verified")
        details = []
        if report.get("permissions"):
            details.append("Asks for: " + ", ".join(PERMISSION_TEXT.get(p, p) for p in report["permissions"]))
        if report.get("depends"):
            details.append("Also installs: " + ", ".join(report["depends"]))
        self.detail_label.set_text("\n".join(details))
        self.install_btn.set_sensitive(True)

        fd, tmp_path = tempfile.mkstemp(prefix="nexa-install-icon-")
        os.close(fd)
        self._icon_tmp = tmp_path
        if packaging.extract_icon(self.nexa_path, self.manifest, tmp_path):
            try:
                pixbuf = GdkPixbuf.Pixbuf.new_from_file_at_scale(tmp_path, 96, 96, True)
                self.icon_image.set_from_pixbuf(pixbuf)
            except GLib.Error:
                self.icon_image.set_from_icon_name("package-x-generic", Gtk.IconSize.DIALOG)
        else:
            self.icon_image.set_from_icon_name("package-x-generic", Gtk.IconSize.DIALOG)

    def _on_install(self, _btn):
        if not self._confirm(
            f"Install {self.manifest['name']}?",
            "This copies files into /usr/local/bin and /usr/local/lib as root.",
        ):
            return
        self.install_btn.set_sensitive(False)
        self._set_status("Installing…")

        self._start_install(allow_unverified=False)

    def _start_install(self, allow_unverified):
        def work():
            ok, output, unverified = _call_helper("install", path=os.path.abspath(self.nexa_path),
                                                  allow_unverified=allow_unverified)
            GLib.idle_add(self._on_install_done, ok, output, unverified)

        threading.Thread(target=work, daemon=True).start()

    def _on_install_done(self, ok, output, unverified=False):
        if unverified and self._confirm(
                "Install an unverified app?",
                output + "\n\nApps from unknown publishers can read your files and use the "
                "internet. Install anyway?"):
            self._set_status("Installing…")
            self._start_install(allow_unverified=True)
            return False
        if ok:
            self._set_status(output)
        else:
            self._set_status(output, error=True)
            self.install_btn.set_sensitive(True)
        return False

    def _set_status(self, text, error=False):
        self.status_label.set_name("errorlabel" if error else "statuslabel")
        self.status_label.set_text(text)

    def _confirm(self, text, secondary):
        dialog = Gtk.MessageDialog(
            transient_for=self, flags=0, message_type=Gtk.MessageType.OTHER,
            buttons=Gtk.ButtonsType.YES_NO, text=text,
        )
        dialog.format_secondary_text(secondary)
        response = dialog.run()
        dialog.destroy()
        return response == Gtk.ResponseType.YES

    def cleanup(self):
        if self._icon_tmp and os.path.exists(self._icon_tmp):
            try:
                os.unlink(self._icon_tmp)
            except OSError:
                pass


def main():
    Gtk.init()
    if len(sys.argv) < 2:
        dialog = Gtk.MessageDialog(
            flags=0, message_type=Gtk.MessageType.INFO, buttons=Gtk.ButtonsType.OK,
            text="Nexa Installer",
        )
        dialog.format_secondary_text("Open a .nexa package file to install it.")
        dialog.run()
        dialog.destroy()
        return
    win = InstallerWindow(sys.argv[1])
    win.connect("destroy", lambda *_a: win.cleanup())
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
