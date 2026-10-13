"""Client for nexa-tools, the Rust program that does the assistant's file work.

Each call runs /usr/local/bin/nexa-tools once, as the user: it checks the
paths (inside home, never keys or Nexa Mind's own records), sandboxes itself
with Landlock to just those folders, does the action, records it for undo and
in ~/.local/share/nexa-mind/activity.jsonl, and answers with one JSON object.
Python never touches the files itself.
"""
import json
import os
import subprocess

BIN = os.environ.get("NEXA_TOOLS_BIN", "/usr/local/bin/nexa-tools")


class HandsError(Exception):
    pass


def call(tool, args=None, timeout=120):
    try:
        p = subprocess.run([BIN], input=json.dumps({"tool": tool, "args": args or {}}).encode(),
                           capture_output=True, timeout=timeout)
    except FileNotFoundError:
        raise HandsError("nexa-tools is not installed")
    except subprocess.TimeoutExpired:
        raise HandsError(f"{tool} took longer than {timeout} s and was stopped")
    try:
        resp = json.loads(p.stdout)
    except (json.JSONDecodeError, UnicodeDecodeError):
        raise HandsError(p.stderr.decode(errors="replace").strip() or f"nexa-tools failed (exit {p.returncode})")
    if not resp.get("ok"):
        raise HandsError(resp.get("error") or "nexa-tools failed")
    return resp.get("result")


def history(limit=50):
    return call("history", {"limit": limit}).get("actions", [])


def undo(action_id):
    return call("undo", {"id": action_id})
