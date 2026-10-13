"""Run with: python3 common/tests/test_ai_files.py

The assistant's file tools go through nexa-tools (Rust): argument
translation, error handling, and the words on approval cards and the
Activity timeline. A fake nexa-tools records what it was asked; the real
binary is tested by `cargo test` (tools/nexa-tools/tests/actions.rs) and,
when it has been built, by the round trip at the end of this file."""
import json, os, stat, subprocess, sys, tempfile, time
HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, "nexa-ai"))

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

tmp = tempfile.mkdtemp()
fake = os.path.join(tmp, "nexa-tools")
seen = os.path.join(tmp, "seen.json")
with open(fake, "w") as f:
    f.write(f"""#!/bin/sh
cat > {seen}
if grep -q '"trash"' {seen}; then echo '{{"ok": false, "error": "~/x is missing"}}'; exit 1; fi
if grep -q '"crash"' {seen}; then echo 'boom' >&2; exit 3; fi
echo '{{"ok": true, "result": {{"done": "Created ~/a.txt", "undo_id": "1-0"}}}}'
""")
os.chmod(fake, 0o755)
os.environ["NEXA_TOOLS_BIN"] = fake

from nexa_common import actiontext, hands
hands.BIN = fake
from nexa_ai import tools

def last():
    return json.load(open(seen))

r = tools.run_tool("write_text_file", {"path": "~/a.txt", "content": "hi"}, allow_control=False)
check("write goes to nexa-tools as write_text, create by default",
      last() == {"tool": "write_text", "args": {"path": "~/a.txt", "content": "hi", "mode": "create"}}, str(last()))
check("...and its result (with the undo id) comes back", r == {"done": "Created ~/a.txt", "undo_id": "1-0"})
tools.run_tool("move_file", {"src": "a", "dest": "b"}, False)
check("move: src/dest become from/to", last() == {"tool": "move", "args": {"from": "a", "to": "b"}})
tools.run_tool("search_files", {"query": "tax"}, False)
check("search: query becomes pattern, home by default", last() == {"tool": "search", "args": {"pattern": "tax", "path": "~"}})
tools.run_tool("organize_folder", {}, False)
check("tidy plans ~/Downloads by type unless told", last() == {"tool": "organize_plan", "args": {"path": "~/Downloads", "by": "type"}})
try:
    tools.run_tool("delete_file", {"path": "~/x"}, False)
    check("delete is trash, and its refusal is a ToolError", False)
except tools.ToolError as e:
    check("delete is trash, and its refusal is a ToolError", last()["tool"] == "trash" and "missing" in str(e), str(e))
try:
    hands.call("crash")
    check("a crash without JSON reports stderr", False)
except hands.HandsError as e:
    check("a crash without JSON reports stderr", "boom" in str(e), str(e))
hands.BIN = os.path.join(tmp, "missing")
try:
    hands.call("list_dir")
except hands.HandsError as e:
    check("missing binary: clear error", "not installed" in str(e))
hands.BIN = fake

file_tools = {"list_dir", "read_text_file", "write_text_file", "search_files", "move_file", "copy_file",
              "delete_file", "create_dir", "find_duplicates", "organize_folder", "apply_moves", "undo_action",
              "recent_actions"}
src = open(os.path.join(REPO, "nexa-ai/nexa_ai/tools.py")).read()
check("Python no longer touches files itself (no shutil/os.remove in tools.py)",
      "shutil" not in src and "os.remove(" not in src and "os.makedirs(" not in src and "os.scandir(" not in src)
check("every file tool is registered", file_tools <= set(tools.TOOLS))
check("planning is read-only, applying is a confirmable action",
      tools.TOOLS["organize_folder"]["category"] == tools.READ and tools.TOOLS["apply_moves"]["category"] == tools.ACTION)
check("trash still always asks first", "delete_file" in tools.ALWAYS_CONFIRM)
check("local model keeps a short tool list", len(tools.LOCAL_TOOLS) <= 16 and set(tools.LOCAL_TOOLS) <= set(tools.TOOLS))

# ---- card words
home = os.path.expanduser("~")
d = actiontext.describe("write_text_file", {"path": "Notes/todo.txt", "content": "a\nb\nc\n"})
check("write card: title, size line, preview", d["title"] == "Create ~/Notes/todo.txt" and d["lines"][0].startswith("3 lines")
      and d["preview"] == "a\nb\nc" and d["kind"] == "write", str(d))
d = actiontext.describe("write_text_file", {"path": home + "/essay.txt", "content": "x", "mode": "overwrite"})
check("overwrite card says the old version is kept", d["title"] == "Replace ~/essay.txt" and any("undone" in l for l in d["lines"]), str(d))
d = actiontext.describe("write_text_file", {"path": "big.txt", "content": "\n".join(str(i) for i in range(20))})
check("long previews are cut to 8 lines", d["preview"].count("\n") == 8 and d["preview"].endswith("…"))
d = actiontext.describe("delete_file", {"path": "~/old.pdf"})
check("trash card is the danger tone and says it's recoverable", d["tone"] == "danger" and "restore" in d["lines"][0])
d = actiontext.describe("apply_moves", {"moves": [
    {"from": "~/Downloads/a.jpg", "to": "~/Downloads/Pictures/a.jpg"},
    {"from": "~/Downloads/b.png", "to": "~/Downloads/Pictures/b.png"},
    {"from": "~/Downloads/c.pdf", "to": "~/Downloads/Documents/c.pdf"}]})
check("tidy card groups by destination folder", d["title"] == "Tidy 3 files into 2 folders"
      and d["lines"] == ["Documents  ·  1  ·  c.pdf", "Pictures  ·  2  ·  a.jpg, b.png"], str(d))
d = actiontext.describe("move_file", {"src": "~/a/report.pdf", "dest": "~/b"})
check("move card shows from → to", d["title"] == "Move report.pdf" and d["lines"] == ["~/a/report.pdf  →  ~/b"], str(d))
check("unknown/control tools still get a readable card", actiontext.describe("set_hostname", {"name": "box"})["title"] == "Set hostname")
check("read chip: items counted", actiontext.read_summary("list_dir", {"path": "~/Documents"}, {"entries": [1, 2]}) == "Looked in ~/Documents · 2 items")
check("read chip: duplicates with space to free",
      actiontext.read_summary("find_duplicates", {}, {"groups": [1], "reclaimable_bytes": 2048}) == "Checked your home folder for duplicates · 1 set of copies, 2.0 KB to free")
check("done text notes skipped files", actiontext.done_text({"done": "Moved 2 files", "skipped": ["~/x"]}) == "Moved 2 files (1 skipped)")
check("timeline kinds from summaries", [actiontext.kind_of_summary(s) for s in
      ("Moved ~/a to the Trash", "Moved 5 files", "Moved ~/a to ~/b", "Created folder ~/x", "Replaced ~/e", "Undid: x")]
      == ["trash", "tidy", "move", "create", "write", "undo"])
now = time.mktime((2026, 10, 13, 15, 0, 0, 0, 0, -1))
groups = actiontext.group_by_day([{"time": now - 60}, {"time": now - 3600}, {"time": now - 86400}, {"time": now - 3 * 86400}], now)
check("timeline groups: Today, Yesterday, then dates", [g[0] for g in groups] == ["Today", "Yesterday", "Saturday 10 October"]
      and len(groups[0][1]) == 2, str(groups))

# ---- the real binary, when it has been built
real = os.path.join(REPO, "build/native/nexa-tools")
if os.access(real, os.X_OK):
    h = tempfile.mkdtemp()
    env = dict(os.environ, HOME=h)
    def real_call(tool, args):
        p = subprocess.run([real], input=json.dumps({"tool": tool, "args": args}).encode(), capture_output=True, env=env)
        return json.loads(p.stdout)
    landlock = real_call("list_dir", {"path": "~"})
    if not landlock["ok"] and "Landlock" in landlock.get("error", ""):
        env["NEXA_TOOLS_ALLOW_UNSANDBOXED"] = "1"
        print("note: this kernel has no Landlock; round trip runs unsandboxed")
    r = real_call("write_text", {"path": "~/Notes/hi.txt", "content": "hello"})
    check("real nexa-tools: write", r["ok"] and open(os.path.join(h, "Notes/hi.txt")).read() == "hello", str(r))
    u = real_call("undo", {"id": r["result"]["undo_id"]})
    check("real nexa-tools: undo removes it (into the Trash)", u["ok"] and not os.path.exists(os.path.join(h, "Notes/hi.txt")), str(u))
    hist = real_call("history", {})
    check("real nexa-tools: history says it was undone", hist["ok"] and hist["result"]["actions"][0]["undone"] is True, str(hist))
else:
    print("note: build/native/nexa-tools not built; skipping the round trip")

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
