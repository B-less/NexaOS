"""Run with: python3 common/tests/test_ai_voice.py

The assistant's link to nexa-voice (nexa-ai/nexa_ai/voice.py) against a fake
nexa-voice that answers each command the way the real one does."""
import json, os, stat, sys, tempfile, threading, time
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(HERE)), "nexa-ai"))
from nexa_ai import voice

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

tmp = tempfile.mkdtemp()
log = os.path.join(tmp, "commands")
fake = os.path.join(tmp, "nexa-voice")
with open(fake, "w") as f:
    f.write(f"""#!/usr/bin/env python3
import json, sys
assert sys.argv[1:] == ["serve"], sys.argv
print(json.dumps({{"event": "ready"}}), flush=True)
for line in sys.stdin:
    cmd = json.loads(line)
    open({log!r}, "a").write(line)
    if "listen" in cmd:
        print(json.dumps({{"event": "heard", "text": "open my mail"}}), flush=True)
    elif "say" in cmd:
        print(json.dumps({{"event": "speaking", "text": cmd["say"]}}), flush=True)
        print(json.dumps({{"event": "done"}}), flush=True)
    print("not json", flush=True)
""")
os.chmod(fake, stat.S_IRWXU)

events, got = [], threading.Event()
def on_event(ev):
    events.append(ev)
    if ev["event"] in ("heard", "done"):
        got.set()

link = voice.VoiceLink(on_event, path=fake)
check("not started until it's needed", not link.running)
check("stop and hide before first use don't start it", link.stop() is False and link.hide() is False and not link.running)
check("listen starts it", link.listen() and link.running)
got.wait(5)
check("what was heard comes back (ready and stray lines skipped)", events == [{"event": "heard", "text": "open my mail"}], events)
got.clear()
check("think", link.think())
check("nothing to say is not sent", link.say("   ") is False)
check("say", link.say("  Opening it now.  "))
got.wait(5)
check("speech events come back", events[1:] == [{"event": "speaking", "text": "Opening it now."}, {"event": "done"}], events)
check("stop once running", link.stop())
link.say("x" * 70000)
time.sleep(0.3)
sent = [json.loads(l) for l in open(log)]
check("commands as sent", sent[:4] == [{"listen": True}, {"think": True}, {"say": "Opening it now."}, {"stop": True}], sent)
check("very long answers are cut to what the voice takes", len(sent[4]["say"]) == 60000)
proc = link.proc
link.close()
check("close ends nexa-voice", proc.poll() is not None and not link.running)

missing = voice.VoiceLink(on_event, path=None)
os.environ["NEXA_VOICE_BIN"] = os.path.join(tmp, "nope")
check("no nexa-voice: unavailable, commands refused quietly",
      (voice.program() is None or os.path.exists(voice.program())) and missing.listen() is False and not missing.running)
os.environ["NEXA_VOICE_BIN"] = fake
check("NEXA_VOICE_BIN points at it", voice.program() == fake)

print(f"\n{'ALL PASSED' if not failures else str(len(failures)) + ' FAILED'}")
sys.exit(1 if failures else 0)
