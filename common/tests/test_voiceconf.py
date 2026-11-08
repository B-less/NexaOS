"""Run with: python3 common/tests/test_voiceconf.py

Nexa's voice settings file (common/nexa_common/voiceconf.py), which
nexa-voice reads."""
import os, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
os.environ["XDG_CONFIG_HOME"] = tempfile.mkdtemp()
from nexa_common import voiceconf

failures = []
def check(label, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + label + (f"  [{detail}]" if detail and not cond else ""))
    if not cond: failures.append(label)

check("defaults without a file", voiceconf.load() == {"voice": "af_heart", "speed": 1.0, "colours": "voice"})
conf = voiceconf.save(voice="bf_emma", speed="1.25")
check("saved and read back", voiceconf.load() == conf == {"voice": "bf_emma", "speed": 1.25, "colours": "voice"}, conf)
text = open(voiceconf.path()).read()
check("in the form nexa-voice reads", "voice=bf_emma\nspeed=1.25\ncolours=voice\n" in text, text)
voiceconf.save(voice="../../etc/passwd", speed=9, colours="neon", other="x")
check("bad values are ignored, not written", voiceconf.load()["voice"] == "bf_emma" and voiceconf.load()["speed"] == 1.25
      and "passwd" not in open(voiceconf.path()).read())
with open(voiceconf.path(), "w") as f:
    f.write("voice = am_michael\nspeed=abc\n# colours=ember\ncolours=calm\ngarbage\n")
check("hand-edited files are read carefully", voiceconf.load() == {"voice": "am_michael", "speed": 1.0, "colours": "calm"})
check("every voice offered is one Kokoro has", [v for v, _ in voiceconf.VOICES] == ["af_heart", "af_bella", "am_michael", "bf_emma"])

print(f"\n{'ALL PASSED' if not failures else str(len(failures)) + ' FAILED'}")
sys.exit(1 if failures else 0)
