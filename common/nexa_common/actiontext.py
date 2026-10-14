"""Words for the assistant's actions: what an approval card says before a
tool runs, what it says after, and the day groups of the Activity timeline.
No GTK here (actionui.py draws it), so it is unit-tested directly.
"""
import os
import time

# kind -> the glyph drawn inside the card's hexagon
GLYPHS = {"create": "+", "write": "✎", "move": "→", "copy": "⧉", "trash": "✕", "tidy": "≡",
          "undo": "↶", "look": "◇", "system": "⚙", "app": "▸", "web": "◎", "download": "↓", "stop": "■"}

READ_VERBS = {
    "list_dir": "Looked in {path}", "read_text_file": "Read {path}", "search_files": "Searched for “{query}”",
    "find_duplicates": "Checked {path} for duplicates", "organize_folder": "Planned tidying {path}",
    "recent_actions": "Checked recent actions",
    "web_search": "Searched the web for “{query}”", "read_webpage": "Read {site}",
    "get_weather": "Checked the weather{place}", "wikipedia_summary": "Looked up “{topic}” on Wikipedia",
    "get_datetime": "Checked the date and time", "get_battery": "Checked the battery",
    "top_processes": "Checked what's running", "calculate": "Calculated {expression}",
    "convert_units": "Converted units", "folder_sizes": "Measured {path}", "recent_files": "Listed recent files in {path}",
    "get_network_info": "Checked the network",
}

WEB_TOOLS = {"web_search", "read_webpage", "get_weather", "wikipedia_summary"}


def _site(url):
    u = str(url or "")
    host = u.split("://", 1)[-1].split("/", 1)[0]
    return host[4:] if host.startswith("www.") else (host or "a page")


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
    if name == "download_file":
        url = str(a.get("url", ""))
        file = url.split("?")[0].rstrip("/").rsplit("/", 1)[-1] or "a file"
        return {"kind": "download", "title": f"Download {file}",
                "lines": [f"From {_site(url)}  →  {_home_rel(a.get('folder') or '~/Downloads')}",
                          "Undo moves it to the Trash."], "preview": "", "tone": "change"}
    if name == "stop_app":
        what = a.get("name") or (f"process {a['pid']}" if a.get("pid") else "a program")
        return {"kind": "stop", "title": f"Close {what}",
                "lines": ["Unsaved work in it may be lost. This can't be undone."], "preview": "", "tone": "danger"}
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
    text = verb.format(path=_home_rel(a.get("path") or a.get("root")), query=a.get("query", ""),
                       site=_site(a.get("url")), topic=a.get("topic", ""),
                       place=f" in {a['place']}" if a.get("place") else "", expression=a.get("expression", ""))
    r = result if isinstance(result, dict) else {}
    if name == "calculate" and "result" in r:
        return f"{a.get('expression', '')} = {r['result']}"
    if name in ("convert_units", "get_weather", "get_battery", "top_processes", "get_network_info") and r.get("summary" if name != "convert_units" else "result"):
        return r["summary" if name != "convert_units" else "result"]
    if name == "get_datetime" and r.get("date"):
        return f"{r['date']}, {r.get('time', '')}"
    if name == "web_search" and "results" in r:
        return text + f" · {len(r['results'])} results"
    if name == "read_webpage" and r.get("title"):
        return f"Read “{r['title']}” ({_site(r.get('url') or a.get('url'))})"
    if name == "folder_sizes" and r.get("total"):
        return text + f" · {r['total']}"
    if name == "recent_files" and "files" in r:
        return text + f" · {len(r['files'])} files"
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
    for prefix, kind in (("Undid", "undo"), ("Downloaded", "download"), ("Created folder", "create"), ("Created", "create"),
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
