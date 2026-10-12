"""Run with: python3 common/tests/test_ai_agent.py

Nexa AI's agent loop and request shape against a fake API (no network)."""
import json, os, sys
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(HERE)), "nexa-ai"))
from nexa_ai import agent, client, tools

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

# ---- request shape (urlopen faked)
sent = {}
class FakeResp:
    def __init__(self, body): self.body = body
    def read(self): return json.dumps(self.body).encode()
    def __enter__(self): return self
    def __exit__(self, *a): return False
def fake_urlopen(req, timeout):
    sent["headers"] = {k.lower(): v for k, v in req.header_items()}
    sent["body"] = json.loads(req.data)
    sent["timeout"] = timeout
    return FakeResp({"content": [{"type": "text", "text": "hi"}], "stop_reason": "end_turn"})
client.urllib.request.urlopen = fake_urlopen
client.create_message("k", "claude-opus-5-5", "sys", [{"role": "user", "content": "x"}], [])
check("request: room for thinking + answer (max_tokens 16000)", sent["body"]["max_tokens"] == 16000)
check("request: server-side refusal fallback, default routing",
      sent["body"].get("fallbacks") == "default" and sent["headers"].get("anthropic-beta") == "server-side-fallback-2026-07-01", str(sent))
check("request: longer timeout for thinking", sent["timeout"] >= 120)
from nexa_ai import cli
check("default model is the current Opus", cli.DEFAULT_MODEL == "claude-opus-5-5")

# ---- agent loop (create_message faked)
script = []
def fake_create(api_key, model, system, messages, tool_specs, **kw):
    return script.pop(0)
agent.client.create_message = fake_create
tools.TOOLS["fake_read"] = {"category": tools.READ, "func": lambda a: {"ok": 1}, "description": "", "input_schema": {}}
events = []
on_event = lambda kind, payload: events.append((kind, payload))

thinking = {"type": "thinking", "thinking": "", "signature": "sig"}
script[:] = [
    {"content": [thinking, {"type": "tool_use", "id": "t1", "name": "fake_read", "input": {}}], "stop_reason": "tool_use"},
    {"content": [{"type": "text", "text": "Done."}], "stop_reason": "end_turn"},
]
msgs = agent.run("k", "m", "q", False, False, lambda *a: True, lambda *a, **k: None, on_event)
check("tool loop: runs the tool and returns its result to Claude",
      msgs[2]["content"][0]["type"] == "tool_result" and events[-1] == ("text", "Done."), str(msgs))
check("tool loop: assistant turns are passed back unchanged (thinking blocks kept)", msgs[1]["content"][0] == thinking)

events.clear()
script[:] = [{"content": [], "stop_reason": "refusal", "stop_details": {"type": "refusal", "explanation": "policy"}}]
msgs = agent.run("k", "m", "q", False, False, lambda *a: True, lambda *a, **k: None, on_event)
check("refusal: the user is told, and no empty assistant turn enters the history",
      any("declined" in p for k, p in events) and msgs[-1]["role"] == "user", str(msgs))

events.clear()
script[:] = [{"content": [{"type": "text", "text": "partial"}], "stop_reason": "max_tokens"}]
agent.run("k", "m", "q", False, False, lambda *a: True, lambda *a, **k: None, on_event)
check("cut-off reply is flagged", any("cut off" in p for k, p in events), str(events))

print(f"\n{len(failures)} failure(s)"); sys.exit(1 if failures else 0)
