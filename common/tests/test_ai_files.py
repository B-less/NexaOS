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
check("every tool the on-device model can get exists", set(tools.LOCAL_TOOLS) <= set(tools.TOOLS))
route = lambda *said: set(tools.local_tools_for(list(said))) - set(tools.LOCAL_CORE)
check("a greeting gets only the small core set", tools.local_tools_for(["hello"]) == tools.LOCAL_CORE)
check("network questions get the network tool (seen on the HP: 'I don't have access to your network')",
      "get_network_info" in route("tell me about my network") and "get_network_info" in route("what's my IP?"))
check("tidying gets the plan/apply tools, and 'downloads' doesn't look like 'load'",
      route("tidy my downloads") == {"organize_folder", "apply_moves", "find_duplicates"}, str(route("tidy my downloads")))
check("topics add up over the conversation, capped",
      {"get_network_info", "set_volume"} <= route("check my wifi", "now turn the volume up")
      and all(len(tools.local_tools_for([q])) <= tools.LOCAL_MAX for q in ["open the app, tidy, wifi, volume, hot, undo, window, copy"]))

import nexa_ai.tools as T
T._wifi = lambda: type("W", (), {"status": staticmethod(lambda: {"connected_to": "HomeNet"})})
hands.call = lambda tool, args=None, timeout=120: {"summary": "Online through wlan0 (wifi), gateway 192.168.1.1", "online": True} if tool == "net_info" else {}
r = tools.run_tool("get_network_info", {}, False)
check("network info: kernel facts from nexa-tools plus the Wi-Fi name from iwd",
      r["wifi_network"] == "HomeNet" and r["summary"].endswith("Wi-Fi network “HomeNet”"), str(r))
T._wifi = lambda: (_ for _ in ()).throw(RuntimeError("no iwd"))
check("network info still works without iwd", "wifi_network" not in tools.run_tool("get_network_info", {}, False))
import importlib; importlib.reload(hands); T.hands = hands; hands.BIN = fake

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

# ---- web, system and helper tools
check("new tools are registered with the right kind",
      all(tools.TOOLS[n]["category"] == tools.READ for n in ("web_search", "read_webpage", "get_weather", "wikipedia_summary",
          "get_datetime", "get_battery", "top_processes", "calculate", "convert_units", "folder_sizes", "recent_files"))
      and tools.TOOLS["download_file"]["category"] == tools.ACTION and "stop_app" in tools.ALWAYS_CONFIRM)
check("routing: weather, maths, time, slowness, web; 'downloads' isn't 'download '",
      route("weather in Kumasi?") == {"get_weather"} and route("what is 15% of 80") == {"calculate"}
      and "get_datetime" in route("what time is it") and "top_processes" in route("why is my laptop so slow")
      and "web_search" in route("search the web for rust news") and "web_search" not in route("tidy my downloads"))
d = actiontext.describe("download_file", {"url": "https://dl.example.org/files/alpine.iso?x=1"})
check("download card: file, site, where, undo", d["title"] == "Download alpine.iso" and d["lines"][0] == "From dl.example.org  →  ~/Downloads", str(d))
check("closing a program is a danger card that says it can't be undone",
      actiontext.describe("stop_app", {"name": "firefox"})["tone"] == "danger")
check("read chips say the useful part", actiontext.read_summary("calculate", {"expression": "15% of 80"}, {"result": "12"}) == "15% of 80 = 12"
      and actiontext.read_summary("read_webpage", {"url": "https://www.bbc.com/news"}, {"title": "Home", "url": "https://www.bbc.com/news"}) == "Read “Home” (bbc.com)"
      and actiontext.read_summary("web_search", {"query": "x"}, {"results": [1, 2]}) == "Searched the web for “x” · 2 results")

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
    os.makedirs(os.path.join(h, "Downloads"))
    r = real_call("list_dir", {"path": "~/downloads"})
    check("real nexa-tools: '~/downloads' finds ~/Downloads", r["ok"] and r["result"]["path"] == "~/Downloads", str(r))
    r = real_call("organize_plan", {"path": "~/Dowloads"})
    check("real nexa-tools: a wrong folder name says what is there",
          not r["ok"] and "doesn't exist" in r["error"] and "Downloads/" in r["error"], str(r))
    c = real_call("calculate", {"expression": "(2+3)*4 - 15% of 80"})
    check("real nexa-tools: exact maths", c["ok"] and c["result"]["result"] == "8", str(c))
    w = real_call("web_read", {"url": "http://127.0.0.1:80/"})
    check("real nexa-tools: the web tools refuse this computer's own addresses", not w["ok"] and "private or local" in w["error"], str(w))
    n = real_call("net_info", {})
    check("real nexa-tools: network facts (sandboxed, read-only)", n["ok"] and "summary" in n["result"], str(n))
    hist = real_call("history", {})
    check("real nexa-tools: history says it was undone", hist["ok"] and hist["result"]["actions"][0]["undone"] is True, str(hist))
else:
    print("note: build/native/nexa-tools not built; skipping the round trip")

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
