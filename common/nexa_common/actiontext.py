"""Words for the assistant's actions: what an approval card says before a
tool runs, what it says after, and the day groups of the Activity timeline.
No GTK here (actionui.py draws it), so it is unit-tested directly.
"""
import os
import time

# kind -> the glyph drawn inside the card's hexagon
GLYPHS = {"create": "+", "write": "✎", "move": "→", "copy": "⧉", "trash": "✕", "tidy": "≡",
          "undo": "↶", "look": "◇", "system": "⚙", "app": "▸"}

READ_VERBS = {
    "list_dir": "Looked in {path}", "read_text_file": "Read {path}", "search_files": "Searched for “{query}”",
    "find_duplicates": "Checked {path} for duplicates", "organize_folder": "Planned tidying {path}",
    "recent_actions": "Checked recent actions",
}


def _home_rel(path):
    if not path:
        return "your home folder"
    path = str(path)
    home = os.path.expanduser("~")
    if path == home:
        return "~"
    if path.startswith(home + "/"):
        return "~/" + path[len(home) + 1:]
    if not path.startswith(("/", "~")):
        return "~/" + path
    return path


def _human_size(n):
    for unit in ("bytes", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n} {unit}" if unit == "bytes" else f"{n:.1f} {unit}"
        n /= 1024


def describe(name, args):
    """Before running: {kind, title, lines, preview, tone}. tone is
    "change", "danger" or "system"; preview is text shown in a code box."""
    a = args or {}
    p = _home_rel(a.get("path"))
    if name == "write_text_file":
        content = a.get("content", "")
        mode = a.get("mode", "create")
        title = {"overwrite": f"Replace {p}", "append": f"Add to {p}"}.get(mode, f"Create {p}")
        lines = [f"{content.count(chr(10)) + (0 if content.endswith(chr(10)) or not content else 1)} lines · "
                 f"{_human_size(len(content.encode()))}"]
        if mode != "create":
            lines.append("The current version is kept, so this can be undone.")
        shown = content.splitlines()[:8]
        preview = "\n".join(shown) + ("\n…" if len(content.splitlines()) > 8 else "")
        return {"kind": "write", "title": title, "lines": lines, "preview": preview, "tone": "change"}
    if name in ("move_file", "copy_file"):
        verb = "Move" if name == "move_file" else "Copy"
        return {"kind": verb.lower(), "title": f"{verb} {os.path.basename(str(a.get('src', '')).rstrip('/')) or 'a file'}",
                "lines": [f"{_home_rel(a.get('src'))}  →  {_home_rel(a.get('dest'))}"], "preview": "", "tone": "change"}
    if name == "create_dir":
        return {"kind": "create", "title": f"Create folder {p}", "lines": [], "preview": "", "tone": "change"}
    if name == "delete_file":
        return {"kind": "trash", "title": f"Move {p} to the Trash",
                "lines": ["You can restore it from the Trash, or with Undo."], "preview": "", "tone": "danger"}
    if name == "apply_moves":
        moves = a.get("moves") or []
        groups = {}
        for m in moves:
            to = str(m.get("to", ""))
            groups.setdefault(os.path.basename(os.path.dirname(to)) or to, []).append(os.path.basename(str(m.get("from", ""))))
        lines = []
        for folder, names in sorted(groups.items()):
            shown = ", ".join(names[:3]) + (f" and {len(names) - 3} more" if len(names) > 3 else "")
            lines.append(f"{folder}  ·  {len(names)}  ·  {shown}")
        return {"kind": "tidy", "title": f"Tidy {len(moves)} file{'s' if len(moves) != 1 else ''} into {len(groups)} folder{'s' if len(groups) != 1 else ''}",
                "lines": lines, "preview": "", "tone": "change"}
    if name == "undo_action":
        return {"kind": "undo", "title": "Undo an earlier action", "lines": [], "preview": "", "tone": "change"}
    if name == "launch_app":
        return {"kind": "app", "title": f"Open {a.get('id') or 'an app'}", "lines": [], "preview": "", "tone": "change"}
    arg_str = ", ".join(f"{k}={v}" for k, v in a.items())
    return {"kind": "system", "title": name.replace("_", " ").capitalize(), "lines": [arg_str] if arg_str else [],
            "preview": "", "tone": "system"}


def read_summary(name, args, result):
    """One line for a read-only tool's chip, e.g. "Looked in ~/Documents · 12 items"."""
    a = args or {}
    verb = READ_VERBS.get(name)
    if verb is None:
        return name.replace("_", " ").capitalize()
    text = verb.format(path=_home_rel(a.get("path") or a.get("root")), query=a.get("query", ""))
    r = result if isinstance(result, dict) else {}
    if "entries" in r:
        text += f" · {len(r['entries'])} items"
    elif "matches" in r:
        text += f" · {len(r['matches'])} found"
    elif "groups" in r:
        n = len(r["groups"])
        text += f" · {n} set{'s' if n != 1 else ''} of copies" + (
            f", {_human_size(r.get('reclaimable_bytes', 0))} to free" if n else "")
    elif "moves" in r:
        text += f" · {len(r['moves'])} files to move"
    return text


def done_text(result):
    """What a finished change says, from nexa-tools' {"done": ...}."""
    if isinstance(result, dict):
        text = result.get("done") or "Done"
        if result.get("skipped"):
            text += f" ({len(result['skipped'])} skipped)"
        return text
    return "Done"


def kind_of_summary(summary):
    s = summary or ""
    for prefix, kind in (("Undid", "undo"), ("Created folder", "create"), ("Created", "create"),
                         ("Replaced", "write"), ("Added to", "write"), ("Moved ", "move"), ("Copied", "copy")):
        if s.startswith(prefix):
            return "trash" if "to the Trash" in s else ("tidy" if kind == "move" and s.endswith(" files") else kind)
    return "system"


def day_label(ts, now=None):
    now = time.time() if now is None else now
    d, today = time.localtime(ts), time.localtime(now)
    if (d.tm_year, d.tm_yday) == (today.tm_year, today.tm_yday):
        return "Today"
    y = time.localtime(now - 86400)
    if (d.tm_year, d.tm_yday) == (y.tm_year, y.tm_yday):
        return "Yesterday"
    return time.strftime("%A %-d %B" if d.tm_year == today.tm_year else "%-d %B %Y", d)


def group_by_day(actions, now=None):
    """[(label, [action, ...]), ...] newest first, from nexa-tools history."""
    groups = []
    for act in actions:
        label = day_label(act.get("time", 0), now)
        if not groups or groups[-1][0] != label:
            groups.append((label, []))
        groups[-1][1].append(act)
    return groups
