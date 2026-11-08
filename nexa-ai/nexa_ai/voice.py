"""The assistant's voice and ears: a thin link to `nexa-voice serve` (C,
tools-c/nexa-voice), which shows the helix, records the microphone, has
whisper hear it and Kokoro speak, both in nexa-mindd. This module only
sends its commands and hands back its events; no audio is touched here."""
import json
import os
import shutil
import subprocess
import threading

CANDIDATES = ("/usr/local/bin/nexa-voice", "/usr/bin/nexa-voice")


def program():
    """Path of nexa-voice, or None on a system without it."""
    for path in (os.environ.get("NEXA_VOICE_BIN"), *CANDIDATES, shutil.which("nexa-voice")):
        if path and os.path.isfile(path) and os.access(path, os.X_OK):
            return path
    return None


class VoiceLink:
    """One `nexa-voice serve` for the window's lifetime, started on first
    use. on_event(dict) is called on the reader thread with each event
    ({"event": "heard", "text": ...} and so on); callers hop to their own
    main loop themselves."""

    def __init__(self, on_event, path=None):
        self.on_event = on_event
        self.path = path or program()
        self.proc = None
        self.lock = threading.Lock()

    @property
    def available(self):
        return self.path is not None

    def _start(self):
        if self.proc and self.proc.poll() is None:
            return True
        if not self.path:
            return False
        try:
            self.proc = subprocess.Popen([self.path, "serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=subprocess.DEVNULL, text=True, bufsize=1)
        except OSError:
            self.proc = None
            return False
        threading.Thread(target=self._read, args=(self.proc,), daemon=True).start()
        return True

    def _read(self, proc):
        for line in proc.stdout:
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            if isinstance(ev, dict) and ev.get("event") != "ready":
                self.on_event(ev)

    def _send(self, cmd):
        with self.lock:
            if not self._start():
                return False
            try:
                self.proc.stdin.write(json.dumps(cmd) + "\n")
                self.proc.stdin.flush()
                return True
            except (BrokenPipeError, OSError):
                self.proc = None
                return False

    def listen(self):
        return self._send({"listen": True})

    def think(self):
        return self._send({"think": True})

    def say(self, text):
        text = (text or "").strip()
        return bool(text) and self._send({"say": text[:60000]})

    @property
    def running(self):
        return self.proc is not None and self.proc.poll() is None

    def stop(self):
        """Stop talking, or finish listening (what was said is heard)."""
        return self.running and self._send({"stop": True})

    def hide(self):
        return self.running and self._send({"hide": True})

    def close(self):
        with self.lock:
            if self.proc and self.proc.poll() is None:
                try:
                    self.proc.stdin.close()
                    self.proc.wait(timeout=2)
                except (OSError, subprocess.TimeoutExpired):
                    self.proc.kill()
            self.proc = None
