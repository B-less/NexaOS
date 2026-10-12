"""A per-user Tor client for the browser's Tor windows (`nexadark`).

Tor runs as the desktop user, not root, listening only on 127.0.0.1, and
only while someone wants it. No GTK here, so it can be unit-tested.

    python3 -m nexa_browser.tor status|stop
"""
import os
import re
import signal
import subprocess
import sys

SOCKS_PORT = 9150  # Tor Browser's port, so it never clashes with a system tor on 9050
PROXY_URI = f"socks5://127.0.0.1:{SOCKS_PORT}"
_BOOTSTRAP = re.compile(r"Bootstrapped (\d+)%(?: \([\w-]+\))?: (.*)")


class TorError(Exception):
    pass


def runtime_dir():
    base = os.environ.get("XDG_RUNTIME_DIR") or f"/tmp/nexa-tor-{os.getuid()}"
    return os.path.join(base, "nexa-tor")


def data_dir():
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "nexa-tor")


def log_path():
    return os.path.join(runtime_dir(), "tor.log")


def pid_path():
    return os.path.join(runtime_dir(), "tor.pid")


def torrc_path():
    """An empty config file: every setting is on the command line, and this
    keeps any system-wide /etc/tor/torrc out of it (tor won't read /dev/null)."""
    return os.path.join(runtime_dir(), "torrc")


def _pid():
    try:
        with open(pid_path()) as f:
            pid = int(f.read().strip())
        with open(f"/proc/{pid}/comm") as f:
            return pid if f.read().strip() == "tor" else None
    except (OSError, ValueError):
        return None


def running():
    return _pid() is not None


def command():
    return ["tor", "--defaults-torrc", torrc_path(), "-f", torrc_path(),
            "--SocksPort", f"127.0.0.1:{SOCKS_PORT}",
            "--ClientOnly", "1",
            "--AvoidDiskWrites", "1",
            "--DataDirectory", data_dir(),
            "--PidFile", pid_path(),
            "--Log", f"notice file {log_path()}",
            "--RunAsDaemon", "1"]


def start():
    """Start Tor unless it is already running. Returns at once; follow
    progress() for the connection."""
    if running():
        return
    for d in (runtime_dir(), data_dir()):
        os.makedirs(d, mode=0o700, exist_ok=True)
        os.chmod(d, 0o700)  # tor refuses a DataDirectory others can read
    open(log_path(), "w").close()
    open(torrc_path(), "w").close()
    try:
        result = subprocess.run(command(), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, timeout=30)
    except FileNotFoundError:
        raise TorError("Tor is not installed (apk add tor).")
    except subprocess.TimeoutExpired:
        raise TorError("Tor did not start.")
    if result.returncode != 0:
        lines = [l.split("] ", 1)[-1] for l in result.stdout.splitlines() if "[err]" in l or "[warn]" in l]
        raise TorError(" ".join(lines[-3:]) or result.stdout.strip()[-300:] or "Tor failed to start.")


def progress(log=None):
    """(percent, summary) from the newest bootstrap line, (0, '') before any."""
    try:
        with open(log or log_path(), errors="replace") as f:
            text = f.read()
    except OSError:
        return 0, ""
    found = _BOOTSTRAP.findall(text)
    if not found:
        return 0, ""
    pct, summary = found[-1]
    return int(pct), summary.strip()


def stop():
    pid = _pid()
    if pid is not None:
        os.kill(pid, signal.SIGTERM)
    return pid is not None


def main(argv):
    cmd = argv[0] if argv else "status"
    if cmd == "stop":
        print("Tor stopped." if stop() else "Tor was not running.")
        return 0
    if cmd == "status":
        if running():
            pct, summary = progress()
            print(f"Tor is running on {PROXY_URI}: {pct}% {summary}".rstrip())
        else:
            print("Tor is not running.")
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
