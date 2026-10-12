"""Download bookkeeping for the browser: where files go, and live speed.
No GTK here, so it can be unit-tested."""
import os
import time
from collections import deque


def downloads_dir():
    path = os.path.join(os.path.expanduser("~"), "Downloads")
    os.makedirs(path, exist_ok=True)
    return path


def safe_name(suggested):
    """A plain file name: no directories, no hidden-file dot, never empty."""
    name = os.path.basename((suggested or "").replace("\\", "/")).strip()
    name = name.lstrip(".").replace("\x00", "")
    return name or "download"


def unique_path(directory, name):
    """directory/name, or 'name (2).ext', 'name (3).ext', ... if taken."""
    stem, ext = os.path.splitext(name)
    if stem.endswith(".tar"):  # keep "x.tar.gz" together
        stem, ext = stem[:-4], ".tar" + ext
    candidate, n = os.path.join(directory, name), 2
    while os.path.exists(candidate):
        candidate = os.path.join(directory, f"{stem} ({n}){ext}")
        n += 1
    return candidate


class SpeedMeter:
    """Bytes per second over the last few seconds, so the number is steady
    rather than jumping with every network packet."""

    def __init__(self, window=3.0, clock=time.monotonic):
        self.window = window
        self.clock = clock
        self.samples = deque()

    def update(self, total_bytes):
        now = self.clock()
        self.samples.append((now, total_bytes))
        while len(self.samples) > 2 and now - self.samples[0][0] > self.window:
            self.samples.popleft()

    def rate(self):
        if len(self.samples) < 2:
            return 0.0
        (t0, b0), (t1, b1) = self.samples[0], self.samples[-1]
        return (b1 - b0) / (t1 - t0) if t1 > t0 else 0.0


def human_bytes(n):
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024:
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024
    return f"{n:.1f} TB"


def human_time(seconds):
    seconds = int(seconds)
    if seconds < 60:
        return f"{seconds} s"
    if seconds < 3600:
        return f"{seconds // 60} min {seconds % 60} s"
    return f"{seconds // 3600} h {seconds % 3600 // 60} min"


def status_line(received, total, rate):
    """'3.2 MB of 10.0 MB · 1.4 MB/s · 5 s left' (parts drop out when unknown)."""
    parts = [f"{human_bytes(received)} of {human_bytes(total)}" if total else human_bytes(received)]
    if rate > 0:
        parts.append(f"{human_bytes(rate)}/s")
        if total and total > received:
            parts.append(f"{human_time((total - received) / rate)} left")
    return " · ".join(parts)
