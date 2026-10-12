import json
import subprocess
import threading

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib

from nexa_common import diskinstall, sysconf
from nexa_common.titlebar import attach_titlebar

CSS = b"""
window { background-image: none; background-color: @nexa_bg; }
#heading { color: @nexa_text_strong; font-size: 18pt; font-weight: bold; }
#subheading { color: @nexa_muted; font-size: 10pt; }
#navbar { background-image: none; background-color: @nexa_surface; }
#navbtn { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text; border-radius: 6px; padding: 8px 18px; }
#navbtn:hover { background-image: none; background-color: @nexa_hover; }
#navbtn:disabled { color: @nexa_disabled; }
#primarybtn { background-image: none; background-color: @nexa_accent; color: @nexa_on_accent; border-radius: 6px; padding: 8px 20px; font-weight: bold; }
#primarybtn:hover { background-image: none; background-color: @nexa_accent_hover; }
#primarybtn:disabled { background-image: none; background-color: @nexa_hover; color: @nexa_disabled; }
#dangerbtn { background-image: none; background-color: @nexa_crit; color: @nexa_on_accent; border-radius: 6px; padding: 8px 20px; font-weight: bold; }
#dangerbtn:hover { background-image: none; background-color: @nexa_crit_hover; }
#dangerbtn:disabled { background-image: none; background-color: @nexa_hover; color: @nexa_disabled; }
#diskrow { background-image: none; background-color: @nexa_surface; border-radius: 8px; padding: 4px; }
#diskrow:selected { background-image: none; background-color: @nexa_selected; }
#diskname { color: @nexa_text; font-size: 11pt; }
#diskmeta { color: @nexa_muted; font-size: 9pt; }
#warning { color: @nexa_crit; font-size: 10pt; }
#entry { background-image: none; background-color: @nexa_field; color: @nexa_text_strong; border-radius: 6px; padding: 6px 10px; border: 1px solid @nexa_border; }
#fieldlabel { color: @nexa_muted; font-size: 9pt; }
#stagelabel { color: @nexa_text; font-size: 11pt; }
#errorlabel { color: @nexa_crit; font-size: 10pt; }
"""

STAGE_LABELS = {
    "partition": "Partitioning the disk...",
    "format": "Formatting partitions...",
    "mount": "Mounting the new system...",
    "kernel": "Installing the kernel...",
    "clone": "Copying system files (this takes a while)...",
    "bootloader": "Installing the bootloader (BIOS + UEFI)...",
    "identity": "Applying your settings...",
    "unmount": "Finishing up...",
}
PAGES = ("welcome", "disk", "confirm", "setup", "progress", "done")


class SetupWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Install NexaOS")
        self.set_icon_name("nexa-setup")
        attach_titlebar(self, "Install NexaOS")
        self.set_default_size(560, 480)
        self.selected_disk = None
        self._build_ui()
        self._go("welcome")

    # ---- shell ----

    def _build_ui(self):
        root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.add(root)

        self.stack = Gtk.Stack()
        self.stack.set_transition_type(Gtk.StackTransitionType.SLIDE_LEFT_RIGHT)
        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        content.set_border_width(24)
        content.pack_start(self.stack, True, True, 0)
        root.pack_start(content, True, True, 0)

        self.stack.add_named(self._build_welcome_page(), "welcome")
        self.stack.add_named(self._build_disk_page(), "disk")
        self.stack.add_named(self._build_confirm_page(), "confirm")
        self.stack.add_named(self._build_setup_page(), "setup")
        self.stack.add_named(self._build_progress_page(), "progress")
        self.stack.add_named(self._build_done_page(), "done")

        nav = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        nav.set_name("navbar")
        nav.set_border_width(14)
        self.back_btn = Gtk.Button(label="Back")
        self.back_btn.set_name("navbtn")
        self._install_failed_state = False
        self.back_btn.connect("clicked", self._on_back)
        nav.pack_start(self.back_btn, False, False, 0)
        nav.pack_start(Gtk.Box(), True, True, 0)
        self.next_btn = Gtk.Button(label="Next")
        self.next_btn.set_name("primarybtn")
        self.next_btn.connect("clicked", self._on_next)
        nav.pack_start(self.next_btn, False, False, 0)
        root.pack_start(nav, False, False, 0)

        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )

    def _page(self, title, subtitle=""):
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        h = Gtk.Label(label=title, xalign=0)
        h.set_name("heading")
        box.pack_start(h, False, False, 0)
        if subtitle:
            s = Gtk.Label(label=subtitle, xalign=0)
            s.set_name("subheading")
            s.set_line_wrap(True)
            box.pack_start(s, False, False, 0)
        return box

    def _prev_page(self):
        i = PAGES.index(self.stack.get_visible_child_name())
        return PAGES[max(0, i - 1)]

    def _go(self, name):
        self.stack.set_visible_child_name(name)
        self.back_btn.set_visible(name not in ("welcome", "progress", "done"))
        self.next_btn.set_visible(name not in ("progress", "done"))
        # Pages that gate Next re-disable it below; without this reset, going
        # Back from a page that had it greyed out left it greyed everywhere.
        self.next_btn.set_sensitive(True)
        if name == "disk":
            self._refresh_disks()
        if name == "confirm":
            self._update_confirm_state()
        if name == "setup":
            self._update_password_state()
        if name == "progress":
            self._start_install()
        self.next_btn.set_label("Install" if name == "confirm" else "Next")
        self.next_btn.set_name("dangerbtn" if name == "confirm" else "primarybtn")

    def _on_back(self, _btn):
        self._go("welcome" if self._install_failed_state else self._prev_page())

    def _on_next(self, _btn):
        name = self.stack.get_visible_child_name()
        order = list(PAGES)
        self._go(order[order.index(name) + 1])

    # ---- welcome ----

    def _build_welcome_page(self):
        page = self._page(
            "Install NexaOS",
            "This installs NexaOS onto a real disk with its own bootloader (BIOS and "
            "UEFI), so the machine boots straight into it without needing a USB stick "
            "or a virtual machine's -kernel flag.",
        )
        warn = Gtk.Label(
            label="This will erase everything on the disk you choose. There is no way to "
                  "undo it once it starts.",
            xalign=0,
        )
        warn.set_name("warning")
        warn.set_line_wrap(True)
        page.pack_start(warn, False, False, 12)
        return page

    # ---- disk selection ----

    def _build_disk_page(self):
        page = self._page("Choose a disk", "Everything on the selected disk will be erased.")
        scroller = Gtk.ScrolledWindow()
        self.disk_list = Gtk.ListBox()
        self.disk_list.set_selection_mode(Gtk.SelectionMode.SINGLE)
        self.disk_list.connect("row-selected", self._on_disk_selected)
        scroller.add(self.disk_list)
        page.pack_start(scroller, True, True, 0)
        self.disk_status = Gtk.Label(label="", xalign=0)
        self.disk_status.set_name("subheading")
        page.pack_start(self.disk_status, False, False, 0)
        return page

    def _refresh_disks(self):
        for child in self.disk_list.get_children():
            self.disk_list.remove(child)
        try:
            disks = diskinstall.list_target_disks()
        except Exception as e:
            self.disk_status.set_text(f"Couldn't list disks: {e}")
            disks = []
        if not disks:
            self.disk_status.set_text(
                "No suitable disk was found (the disk this session is running from is "
                "never shown here)."
            )
        else:
            self.disk_status.set_text("")
        for disk in disks:
            row = Gtk.ListBoxRow()
            row.set_name("diskrow")
            row.disk = disk
            row.set_selectable(disk["large_enough"])
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            box.set_border_width(10)
            gb = disk["size_bytes"] / 1024**3
            name = Gtk.Label(label=f"/dev/{disk['name']}  ({gb:.0f} GB)", xalign=0)
            name.set_name("diskname")
            box.pack_start(name, False, False, 0)
            meta_text = disk["model"] or "Unknown drive"
            if not disk["large_enough"]:
                meta_text += f" -- too small (need at least {diskinstall.MIN_DISK_BYTES // 1024**3} GB)"
            meta = Gtk.Label(label=meta_text, xalign=0)
            meta.set_name("diskmeta")
            box.pack_start(meta, False, False, 0)
            row.add(box)
            self.disk_list.add(row)
        self.disk_list.show_all()
        self._update_next_sensitivity()

    def _on_disk_selected(self, _list, row):
        self.selected_disk = row.disk if row else None
        self._update_next_sensitivity()

    def _update_next_sensitivity(self):  # disk page
        if self.stack.get_visible_child_name() == "disk":
            self.next_btn.set_sensitive(self.selected_disk is not None)

    # ---- confirm ----

    def _build_confirm_page(self):
        page = self._page("Confirm")
        self.confirm_summary = Gtk.Label(label="", xalign=0)
        self.confirm_summary.set_line_wrap(True)
        page.pack_start(self.confirm_summary, False, False, 0)
        warn = Gtk.Label(
            label="Type the disk name below exactly to confirm you want to erase it.",
            xalign=0,
        )
        warn.set_name("warning")
        warn.set_line_wrap(True)
        page.pack_start(warn, False, False, 6)
        self.confirm_entry = Gtk.Entry()
        self.confirm_entry.set_name("entry")
        self.confirm_entry.connect("changed", lambda *_a: self._update_confirm_state())
        page.pack_start(self.confirm_entry, False, False, 0)
        return page

    def _update_confirm_state(self):
        if self.selected_disk is None:
            return
        name = self.selected_disk["name"]
        self.confirm_summary.set_text(
            f"About to erase /dev/{name} and install NexaOS on it. This cannot be undone."
        )
        self.confirm_entry.set_placeholder_text(name)
        if self.stack.get_visible_child_name() == "confirm":
            self.next_btn.set_sensitive(self.confirm_entry.get_text() == name)

    # ---- setup ----

    def _build_setup_page(self):
        page = self._page("A few details")
        page.pack_start(self._field_label("Hostname"), False, False, 0)
        self.hostname_entry = Gtk.Entry()
        self.hostname_entry.set_name("entry")
        self.hostname_entry.set_text("nexa")
        page.pack_start(self.hostname_entry, False, False, 0)

        page.pack_start(self._field_label("Time zone"), False, False, 0)
        self._zone_regions = {}
        for zone in sysconf.list_zones():
            region, _, city = zone.partition("/")
            self._zone_regions.setdefault(region or "Other", []).append((city or zone, zone))
        zone_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        self.region_combo = Gtk.ComboBoxText()
        self.city_combo = Gtk.ComboBoxText()
        for region in sorted(self._zone_regions):
            self.region_combo.append_text(region)
        self.region_combo.connect("changed", self._on_region_changed)
        zone_row.pack_start(self.region_combo, False, False, 0)
        zone_row.pack_start(self.city_combo, True, True, 0)
        page.pack_start(zone_row, False, False, 0)
        self._select_zone(sysconf.current_zone())

        page.pack_start(self._field_label("Keyboard layout"), False, False, 0)
        self._layouts = sysconf.list_layouts()
        self.layout_combo = Gtk.ComboBoxText()
        current = sysconf.read_keyboard_layout()
        for i, (code, desc) in enumerate(self._layouts):
            self.layout_combo.append_text(f"{desc} ({code})")
            if code == current:
                self.layout_combo.set_active(i)
        page.pack_start(self.layout_combo, False, False, 0)

        page.pack_start(self._field_label("Password (for the \"user\" account and administrator access)"),
                        False, False, 0)
        self.password_entry = self._password_entry("Password")
        page.pack_start(self.password_entry, False, False, 0)
        self.password_confirm_entry = self._password_entry("Confirm password")
        page.pack_start(self.password_confirm_entry, False, False, 0)
        self.password_hint = Gtk.Label(label="", xalign=0)
        self.password_hint.set_name("fieldlabel")
        page.pack_start(self.password_hint, False, False, 0)
        return page

    def _password_entry(self, placeholder):
        entry = Gtk.Entry()
        entry.set_name("entry")
        entry.set_visibility(False)
        entry.set_input_purpose(Gtk.InputPurpose.PASSWORD)
        entry.set_placeholder_text(placeholder)
        entry.connect("changed", lambda *_a: self._update_password_state())
        return entry

    def _password_problem(self):
        password = self.password_entry.get_text()
        try:
            diskinstall.validate_password(password)
        except diskinstall.InstallError as e:
            return str(e)
        if password != self.password_confirm_entry.get_text():
            return "The passwords don't match."
        return None

    def _update_password_state(self):
        problem = self._password_problem()
        # Don't nag before anything has been typed.
        typed = self.password_entry.get_text() or self.password_confirm_entry.get_text()
        self.password_hint.set_text(problem if (problem and typed) else "")
        if self.stack.get_visible_child_name() == "setup":
            self.next_btn.set_sensitive(problem is None)

    def _field_label(self, text):
        label = Gtk.Label(label=text, xalign=0)
        label.set_name("fieldlabel")
        return label

    def _select_zone(self, zone):
        region, _, city = zone.partition("/")
        regions = sorted(self._zone_regions)
        if region in regions:
            self.region_combo.set_active(regions.index(region))
            self._city_zones = sorted(self._zone_regions[region])
            for i, (_c, z) in enumerate(self._city_zones):
                if z == zone:
                    self.city_combo.set_active(i)
        elif regions:
            self.region_combo.set_active(0)

    def _on_region_changed(self, combo):
        self.city_combo.remove_all()
        self._city_zones = sorted(self._zone_regions.get(combo.get_active_text(), []))
        for city, _zone in self._city_zones:
            self.city_combo.append_text(city.replace("_", " "))
        if self._city_zones:
            self.city_combo.set_active(0)

    def _chosen_zone(self):
        idx = self.city_combo.get_active()
        if self._city_zones and 0 <= idx < len(self._city_zones):
            return self._city_zones[idx][1]
        return None

    def _chosen_layout(self):
        idx = self.layout_combo.get_active()
        if 0 <= idx < len(self._layouts):
            return self._layouts[idx][0]
        return None

    # ---- progress ----

    def _build_progress_page(self):
        page = self._page("Installing NexaOS")
        self.stage_label = Gtk.Label(label="Starting...", xalign=0)
        self.stage_label.set_name("stagelabel")
        page.pack_start(self.stage_label, False, False, 20)
        self.progress_bar = Gtk.ProgressBar()
        self.progress_bar.set_pulse_step(0.1)
        page.pack_start(self.progress_bar, False, False, 0)
        self.progress_error = Gtk.Label(label="", xalign=0)
        self.progress_error.set_name("errorlabel")
        self.progress_error.set_line_wrap(True)
        page.pack_start(self.progress_error, False, False, 12)
        return page

    def _start_install(self):
        self._install_failed_state = False
        self.back_btn.set_label("Back")
        self.stage_label.set_text("Starting...")
        self.progress_bar.set_fraction(0)
        self.progress_error.set_text("")
        GLib.timeout_add(120, self._pulse)
        threading.Thread(target=self._run_install_worker, daemon=True).start()

    def _pulse(self):
        if self.stack.get_visible_child_name() != "progress":
            return False
        self.progress_bar.pulse()
        return True

    def _run_install_worker(self):
        request = {
            "disk": self.selected_disk["name"],
            "hostname": self.hostname_entry.get_text().strip() or None,
            "zone": self._chosen_zone(),
            "keyboard": self._chosen_layout(),
            "password": self.password_entry.get_text(),
        }
        try:
            proc = subprocess.Popen(
                ["doas", "/usr/local/bin/nexa-setup-helper"],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
        except OSError as e:
            GLib.idle_add(self._install_failed, str(e))
            return
        proc.stdin.write(json.dumps(request))
        proc.stdin.close()
        final = None
        for line in proc.stdout:
            line = line.strip()
            if not line:
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "stage" in event:
                GLib.idle_add(self._install_stage, event["stage"])
            elif "ok" in event:
                final = event
        stderr = proc.stderr.read()
        proc.wait()
        if final is None:
            GLib.idle_add(self._install_failed, stderr.strip() or "The installer helper produced no output.")
        elif final.get("ok"):
            GLib.idle_add(self._install_done)
        else:
            GLib.idle_add(self._install_failed, final.get("error", "Installation failed."))

    def _install_stage(self, stage):
        self.stage_label.set_text(STAGE_LABELS.get(stage, stage))
        return False

    def _install_failed(self, message):
        self.progress_error.set_text(message)
        self.stage_label.set_text("Installation failed.")
        self._install_failed_state = True
        self.back_btn.set_label("Start over")
        self.back_btn.set_visible(True)
        return False

    def _install_done(self):
        self._go("done")
        return False

    # ---- done ----

    def _build_done_page(self):
        page = self._page("Installation complete", "NexaOS is installed. Restart to boot into it.")
        reboot_btn = Gtk.Button(label="Restart now")
        reboot_btn.set_name("primarybtn")
        reboot_btn.set_halign(Gtk.Align.START)
        reboot_btn.connect("clicked", lambda *_a: subprocess.run(["nexa-power"], check=False) or self.close())
        page.pack_start(reboot_btn, False, False, 16)
        return page


def main():
    Gtk.init()
    win = SetupWindow()
    win.connect("destroy", Gtk.main_quit)
    win.show_all()
    Gtk.main()
