"""Nexa Mind's voice and ears, for real: nexa-mindd serving Kokoro and
whisper from the real models, nexa-voice in a headless Sway with the helix
on screen. Kokoro's words are heard back through whisper. Run by
scripts/test-voice.sh; screenshots go to $OUT."""
import json
import os
import re
import struct
import subprocess
import time

import gi
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf

OUT = os.environ.get("OUT", "/tmp")
SOCK = "/tmp/mind-run/mind.sock"
STATE = "/tmp/voice.state"
MIND_ENV = {"NEXA_MIND_STATE": "/tmp/mind-state", "NEXA_MIND_RUN": "/tmp/mind-run"}
SINK = "fakesink sync=true"          # no sound card here: the same clock, no sound
SAID = "Hello, I'm Nexa. Your mail is up to date, and Kofi sent two files to your phone. Do you want me to open them?"
W, H = 1280, 800
fails = 0


def check(ok, what, detail=""):
    global fails
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f"  ({detail})"), flush=True)
    fails += not ok


def info(what):
    print("INFO " + what, flush=True)


def wait(cond, secs=5.0):
    end = time.time() + secs
    while time.time() < end:
        try:
            if cond():
                return True
        except (KeyError, TypeError, IndexError, ValueError):
            pass
        time.sleep(0.05)
    return False


def state():
    try:
        return json.load(open(STATE))
    except (OSError, ValueError):
        return {}


def post(path, body, ctype="application/json", sock=SOCK, stream=False):
    """HTTP over the service's socket: (status, body bytes, seconds to the
    first byte of the body)."""
    import socket
    s = socket.socket(socket.AF_UNIX)
    s.connect(sock)
    data = body if isinstance(body, bytes) else json.dumps(body).encode()
    t0 = time.time()
    s.sendall(f"POST {path} HTTP/1.1\r\nHost: x\r\nContent-Type: {ctype}\r\nContent-Length: {len(data)}\r\nConnection: close\r\n\r\n".encode() + data)
    buf, first = b"", None
    while True:
        chunk = s.recv(65536)
        if not chunk:
            break
        buf += chunk
        if first is None and b"\r\n\r\n" in buf and len(buf) > buf.index(b"\r\n\r\n") + 4:
            first = time.time() - t0
    s.close()
    head, _, rest = buf.partition(b"\r\n\r\n")
    return int(head.split()[1]), rest, first


def frames(body):
    """nexa-speak frames: [(rate, text, samples)], and how it ended."""
    out, i = [], 0
    while i < len(body):
        nl = body.index(b"\n", i)
        line = body[i:nl].decode()
        i = nl + 1
        if line == "END" or line.startswith("ERR "):
            return out, line
        _, rate, n, t = line.split()
        n, t = int(n), int(t)
        text = body[i:i + t].decode()
        pcm = body[i + t:i + t + 2 * n]
        i += t + 2 * n
        out.append((int(rate), text, pcm))
    return out, "cut"


def wav(path, rate, pcm):
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 1, rate, rate * 2, 2, 16)
                + b"data" + struct.pack("<I", len(pcm)) + pcm)


def resample16(pcm, rate):
    s = struct.unpack(f"<{len(pcm) // 2}h", pcm)
    n = int(len(s) * 16000 / rate)
    out = [s[min(len(s) - 1, int(i * rate / 16000))] for i in range(n)]
    return struct.pack(f"<{n}h", *out)


def words(t):
    return re.findall(r"[a-z0-9']+", t.lower().replace("’", "'"))


def similar(a, b):
    wa, wb = words(a), words(b)
    hit = sum(1 for w in wa if w in wb)
    return hit / max(1, len(wa))


def voice(*args, env=None, stdin=None):
    e = dict(os.environ, NEXA_MIND_SOCKET=SOCK, NEXA_VOICE_STATE=STATE, NEXA_VOICE_SINK=SINK)
    e.update(env or {})
    return subprocess.Popen(["nexa-voice", *args], env=e, stdout=subprocess.PIPE, stdin=stdin,
                            stderr=open("/tmp/voice.err", "a"), text=True, bufsize=1)


def events(p, until=("done", "stopped", "heard", "error"), secs=60):
    got, end = [], time.time() + secs
    import select
    while time.time() < end:
        r, _, _ = select.select([p.stdout], [], [], 0.2)
        if not r:
            if p.poll() is not None:
                break
            continue
        line = p.stdout.readline()
        if not line:
            break
        ev = json.loads(line)
        got.append(ev)
        if ev["event"] in until:
            break
    return got


def shot(name):
    path = f"{OUT}/{name}.png"
    subprocess.run(["grim", path], check=False)
    return path


def overlay_pixels(path):
    """How much of the bottom middle (where the helix shows) is coloured."""
    pb = GdkPixbuf.Pixbuf.new_from_file(path)
    px, st, nch = pb.get_pixels(), pb.get_rowstride(), pb.get_n_channels()
    x0, x1, y0, y1 = W // 2 - 260, W // 2 + 260, H - 96 - 176, H - 96
    coloured = 0
    for y in range(y0, y1, 2):
        for x in range(x0, x1, 2):
            r, g, b = px[y * st + x * nch:y * st + x * nch + 3]
            if max(r, g, b) - min(r, g, b) > 60:
                coloured += 1
    return coloured / (((x1 - x0) // 2) * ((y1 - y0) // 2))


def start_mind(env_extra=None, log="/tmp/mind.log"):
    e = dict(os.environ, **MIND_ENV)
    e.update(env_extra or {})
    sock = e["NEXA_MIND_RUN"] + "/mind.sock"
    if os.path.exists(sock):
        os.unlink(sock)   # an old service's: wait for the new one's
    p = subprocess.Popen(["nexa-mindd", "serve"], env=e, stderr=open(log, "w"))

    def answers():
        import socket
        s = socket.socket(socket.AF_UNIX)
        try:
            s.connect(sock)
            return True
        except OSError:
            return False
        finally:
            s.close()
    wait(answers, 10)
    return p


# ---------------------------------------------------------------------------
mind = start_mind()
check(os.path.exists(SOCK), "nexa-mindd is listening")
print(open("/tmp/mind.log").read().strip())

import socket as _s


def get(path):
    s = _s.socket(_s.AF_UNIX)
    s.connect(SOCK)
    s.sendall(f"GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".encode())
    buf = b""
    while True:
        c = s.recv(65536)
        if not c:
            break
        buf += c
    return json.loads(buf.partition(b"\r\n\r\n")[2])


models = {m["name"]: m for m in get("/v1/models")}
check(models.get("kokoro-82m", {}).get("installed") and models["kokoro-82m"]["role"] == "voice", "Kokoro is installed, as the voice")
check(models.get("whisper-base.en", {}).get("installed") and models["whisper-base.en"]["role"] == "listen", "whisper is installed, for hearing")
check(get("/v1/status").get("voice_loaded") is False, "the voice isn't loaded until it's needed")

# --- speaking, straight from the service ---
t0 = time.time()
code, body, first = post("/v1/audio/speech", {"text": SAID})
took = time.time() - t0
fr, end = frames(body)
pcm = b"".join(f[2] for f in fr)
secs = len(pcm) / 2 / 24000
check(code == 200 and end == "END", "the service speaks", f"{code} {end} {body[:200]!r}")
check(len(fr) >= 3 and all(f[0] == 24000 for f in fr), "sentence by sentence, at 24 kHz", f"{[(f[0], f[1]) for f in fr]}")
check(" ".join(f[1] for f in fr).replace(" ,", ",") .startswith("Hello,"), "each frame says which words it is", f"{[f[1] for f in fr]}")
check(2.5 < secs < 15, f"about the right length ({secs:.1f} s of speech)")
peak = max(abs(v) for v in struct.unpack(f"<{len(pcm) // 2}h", pcm))
check(peak > 3000, "and it isn't silence", f"peak {peak}")
info(f"cold start: first sentence after {first:.1f} s, all {secs:.1f} s of speech made in {took:.1f} s")
t0 = time.time()
code2, body2, first2 = post("/v1/audio/speech", {"text": "Okay, opening it now."})
info(f"warm: first sentence after {first2:.1f} s")
check(code2 == 200 and get("/v1/status").get("voice_loaded") is True, "the voice stays loaded while you talk")
wav("/tmp/said.wav", 24000, pcm)
wav(f"{OUT}/kokoro-said.wav", 24000, pcm)

for bad, why in [({"text": ""}, "empty text"), ({"text": "hi", "voice": "../../etc"}, "a voice outside the list"),
                 ({"text": "hi", "speed": 9}, "a silly speed"), ({"text": "x" * 70000}, "too much text")]:
    code, body, _ = post("/v1/audio/speech", bad)
    check(code == 400, f"refused: {why}", f"{code} {body[:100]!r}")
code, body, _ = post("/v1/audio/speech", {"text": "Emma here.", "voice": "bf_emma"})
check(code == 200 and frames(body)[1] == "END", "a British voice speaks too")

# --- hearing, straight from the service ---
pcm16 = resample16(pcm, 24000)
wav("/tmp/said16.wav", 16000, pcm16 + b"\0" * 64000)   # and two seconds of quiet
t0 = time.time()
code, body, _ = post("/v1/audio/transcriptions", open("/tmp/said16.wav", "rb").read(), "audio/wav")
heard = json.loads(body).get("text", "") if code == 200 else ""
info(f"whisper heard in {time.time() - t0:.1f} s: {heard!r}")
check(code == 200 and similar(SAID, heard) >= 0.85, "whisper hears Kokoro's words", f"{code} {heard!r}")
code, body, _ = post("/v1/audio/transcriptions", b"not a wav", "audio/wav")
check(code == 400, "a file that isn't a recording is refused", f"{code}")
wav("/tmp/stereo.wav", 44100, b"\0" * 44100)
code, body, _ = post("/v1/audio/transcriptions", open("/tmp/stereo.wav", "rb").read(), "audio/wav")
check(code == 400 and b"16 kHz" in body, "only 16 kHz mono is taken")
check(not [f for f in os.listdir("/tmp/mind-run/private") if f.endswith(".wav")], "recordings are deleted after hearing")

# --- the voice loads while you are still talking ---
code, body, _ = post("/v1/audio/warm", {})
check(code == 202, "warm-up is accepted")
mind.terminate()
mind.wait(5)
mind = start_mind()
t0 = time.time()
code, _, _ = post("/v1/audio/warm", {})


def speak_log():
    try:
        return open("/tmp/mind-state/nexa-speak.log").read()
    except OSError:
        return ""
check(code == 202 and wait(lambda: "ready" in speak_log(), 60), "warm-up loads the voice")
info(f"loading the voice took {time.time() - t0:.1f} s here")
t0 = time.time()
code, body, first = post("/v1/audio/speech", {"text": "Sure, here it is."})
info(f"after warm-up: first sentence after {first:.1f} s")
check(code == 200 and first < 3, "after warm-up, Nexa starts speaking quickly", f"{first:.1f} s")

# --- nexa-voice say: the helix on screen, in step with the sound ---
subprocess.run(["grim", "/tmp/before.png"])
before = overlay_pixels("/tmp/before.png")
t0 = time.time()
p = voice("say", SAID)
evs = []
import threading
th = threading.Thread(target=lambda: evs.extend(events(p)))
th.start()
check(wait(lambda: state()["mode"] == "speaking" and state()["shown"], 10), "the helix shows while Nexa speaks")
wait(lambda: state()["played_frames"] > 150, 15)
sp = shot("voice-speaking")
mid = state()
th.join(60)
p.wait(10)
took = time.time() - t0
check(p.returncode == 0 and evs and evs[-1]["event"] == "done", "say: done", f"{evs}")
spoken = [e["text"] for e in evs if e["event"] == "speaking"]
check(len(spoken) >= 3 and spoken[0].startswith("Hello"), "each sentence is announced as it starts", f"{spoken}")
check(took >= secs * 0.9, f"it takes as long as the speech lasts ({took:.1f} s for {secs:.1f} s)")
info(f"pauses to wait for speech: {state().get('underruns')}")
st = state()
check(st.get("source") == "kokoro" and st.get("rate") == 24000, "Kokoro's voice", f"{st.get('source')} {st.get('rate')}")
check(st.get("pops", 0) >= secs * 2, f"the helix pops with the syllables ({st.get('pops')} in {secs:.1f} s)")
check(mid.get("fps", 0) >= 30, f"and moves smoothly while the next sentences are made ({mid.get('fps')} frames a second)")
check(st.get("env_peak", 0) > 0.5, f"and swells with the loudness ({st.get('env_peak')})")
check(abs(st.get("played_frames", 0) - st.get("frames", 1)) <= 30, "every frame of sound reached the helix",
      f"{st.get('played_frames')} of {st.get('frames')}")
check(mid.get("caption", "") in spoken, "the sentence being said is shown under it", f"{mid.get('caption')!r}")
cov = overlay_pixels(sp)
check(cov > before + 0.03, f"the helix is on screen ({cov:.0%} coloured, {before:.0%} before)")

# --- stop mid-sentence ---
long_text = " ".join([SAID] * 4)
p = voice("say", long_text)
first = events(p, until=("speaking",), secs=30)
time.sleep(0.8)
t0 = time.time()
rc = subprocess.run(["nexa-voice", "stop"]).returncode
rest = events(p, secs=5)
p.wait(5)
check(rc == 0 and first and rest and rest[-1]["event"] == "stopped", "nexa-voice stop: stops talking", f"{rc} {rest}")
check(time.time() - t0 < 2 and p.returncode == 0, "at once")
code, body, _ = post("/v1/audio/speech", {"text": "Still here."})
check(code == 200 and frames(body)[1] == "END", "and the service speaks again afterwards")

# --- listening: Kokoro's recording played into the microphone ---
src = "filesrc location=/tmp/said16.wav ! wavparse ! identity sync=true"
p = voice("listen", env={"NEXA_VOICE_SOURCE": src})
check(wait(lambda: state()["mode"] == "listening" and state()["shown"], 5), "the helix listens")
wait(lambda: state()["voiced_ms"] > 1500, 10)
shot("voice-listening")
check(state().get("caption") == "Listening…", "it says it's listening")
check(wait(lambda: state()["mode"] == "thinking", 20), "a pause after speech ends listening by itself")
shot("voice-hearing")
evs = events(p, secs=60)
p.wait(10)
heard = evs[-1].get("text", "") if evs else ""
check(evs and evs[-1]["event"] == "heard" and similar(SAID, heard) >= 0.85, "and whisper hears it", f"{evs}")
info(f"heard: {heard!r}")

p = voice("listen", "--max", "3", env={"NEXA_VOICE_SOURCE": "audiotestsrc wave=silence is-live=true"})
evs = events(p, secs=10)
p.wait(5)
check(evs and evs[-1] == {"event": "heard", "text": ""}, "silence: nothing heard, nothing sent", f"{evs}")

p = voice("listen", env={"NEXA_VOICE_SOURCE": "audiotestsrc wave=silence is-live=true"})
wait(lambda: state()["mode"] == "listening", 5)
time.sleep(0.5)
subprocess.run(["nexa-voice", "stop"])
evs = events(p, secs=10)
p.wait(5)
check(evs and evs[-1]["event"] == "heard", "push-to-talk: stop ends listening", f"{evs}")

# --- serve: what the assistant window drives ---
p = voice("serve", stdin=subprocess.PIPE, env={"NEXA_VOICE_SOURCE": src})
ready = events(p, until=("ready",), secs=5)
check(ready == [{"event": "ready"}], "serve: ready")


def send(cmd):
    p.stdin.write(json.dumps(cmd) + "\n")
    p.stdin.flush()


send({"listen": True})
evs = events(p, secs=60)
check(evs and evs[-1]["event"] == "heard" and similar(SAID, evs[-1]["text"]) >= 0.85, "listen through serve", f"{evs}")
send({"think": True})
check(wait(lambda: state()["mode"] == "thinking" and state()["shown"], 5), "think: the helix spins")
shot("voice-thinking")
send({"say": "Sure. Opening your mail now.", "voice": "am_michael"})
evs = events(p, secs=30)
check([e["event"] for e in evs][-1:] == ["done"] and any(e["event"] == "speaking" for e in evs), "say through serve", f"{evs}")
check(state().get("voice") == "af_heart", "a voice asked for once doesn't change the setting")
check(wait(lambda: not state()["shown"], 3), "it fades away when done")
send({"bogus": 1})
evs = events(p, until=("error",), secs=3)
check(evs and evs[-1]["error"] == "unknown command", "an unknown command is an error, not a crash")
send({"say": long_text})
events(p, until=("speaking",), secs=30)
send({"stop": True})
evs = events(p, secs=5)
check(evs and evs[-1]["event"] == "stopped", "stop through serve")
p.stdin.close()
p.wait(5)
check(p.returncode == 0, "serve ends when the assistant closes")

# --- without Kokoro: espeak-ng, then silence ---
p = voice("say", "Fallback voice here.", env={"NEXA_MIND_SOCKET": "/tmp/nowhere.sock"})
evs = events(p, secs=20)
p.wait(5)
st = state()
check(evs and evs[-1]["event"] == "done" and st.get("source") == "espeak", "no Nexa Mind: espeak-ng speaks", f"{evs} {st.get('source')}")
p = voice("say", "Nobody can talk.", env={"NEXA_MIND_SOCKET": "/tmp/nowhere.sock", "NEXA_VOICE_ESPEAK": "none"})
evs = events(p, secs=20)
p.wait(5)
st = state()
check(evs and evs[-1]["event"] == "done" and st.get("source") == "silent" and st.get("pops", 0) > 0,
      "no voice at all: the helix still moves", f"{evs} {st.get('source')} {st.get('pops')}")

empty = start_mind({"NEXA_MIND_STATE": "/tmp/mind-empty", "NEXA_MIND_RUN": "/tmp/mind-empty-run"}, "/tmp/mind-empty.log")
p = voice("say", "Not downloaded yet.", env={"NEXA_MIND_SOCKET": "/tmp/mind-empty-run/mind.sock"})
evs = events(p, secs=20)
p.wait(5)
check(evs and evs[-1]["event"] == "done" and state().get("source") == "espeak", "Kokoro not downloaded: espeak-ng speaks")
p = voice("listen", env={"NEXA_MIND_SOCKET": "/tmp/mind-empty-run/mind.sock", "NEXA_VOICE_SOURCE": src})
evs = events(p, secs=30)
p.wait(5)
check(evs and evs[-1]["event"] == "error" and "Settings" in evs[-1].get("hint", ""),
      "whisper not downloaded: says where to get it", f"{evs}")
empty.terminate()

# --- the theme and settings ---
cfg = os.path.expanduser("~/.config/nexa")
os.makedirs(cfg, exist_ok=True)
open(f"{cfg}/lock.conf", "w").write("accent=#ff8800\n")
open(f"{cfg}/voice.conf", "w").write("voice=am_michael\nspeed=1.2\ncolours=ember\n")
# Reduce animations, as Settings > Accessibility sets it
subprocess.run(["gsettings", "set", "org.gnome.desktop.interface", "enable-animations", "false"])
p = voice("say", "Calm now.")
wait(lambda: state()["mode"] == "speaking", 10)
st = state()
events(p, secs=20)
p.wait(5)
check(abs(st.get("accent_hue", 0) - 32) < 3, "colours start from the theme's accent", f"{st.get('accent_hue')}")
check(st.get("voice") == "am_michael" and st.get("speed") == 1.2 and st.get("palette") == 3, "voice.conf picks the voice, speed and colours")
check(st.get("animations") == 0, "reduce animations is respected")

mind.terminate()
mind.wait(5)
print(f"{'ALL PASSED' if not fails else str(fails) + ' FAILED'}")
raise SystemExit(1 if fails else 0)
