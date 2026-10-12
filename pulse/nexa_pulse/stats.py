"""System and process readings for Nexa Pulse, straight from /proc. No GTK,
so it can be unit-tested; PROC can point at a fake tree."""
import os

PROC = "/proc"


def _read(path):
    with open(path) as f:
        return f.read()


def cpu_times():
    """(idle, total) jiffies across all CPUs."""
    values = [int(x) for x in _read(os.path.join(PROC, "stat")).splitlines()[0].split()[1:]]
    return values[3] + values[4], sum(values)


def cpu_percent(prev, now):
    d_total = now[1] - prev[1]
    if d_total <= 0:
        return 0.0
    return max(0.0, min(100.0, 100.0 * (1 - (now[0] - prev[0]) / d_total)))


def memory():
    """(used_percent, used_mb, total_mb)."""
    info = {}
    for line in _read(os.path.join(PROC, "meminfo")).splitlines():
        key, _, rest = line.partition(":")
        if rest.split():
            info[key] = int(rest.split()[0])
    total = info.get("MemTotal", 1)
    used = total - info.get("MemAvailable", total)
    return used / total * 100, used // 1024, total // 1024


def disk(path="/"):
    """(used_percent, used_gb, total_gb)."""
    st = os.statvfs(path)
    total = st.f_blocks * st.f_frsize
    used = total - st.f_bavail * st.f_frsize
    return (used / total * 100 if total else 0.0), used / 1024 ** 3, total / 1024 ** 3


def net_bytes():
    """(received, sent) bytes over every interface except loopback."""
    rx = tx = 0
    for line in _read(os.path.join(PROC, "net", "dev")).splitlines()[2:]:
        name, _, rest = line.partition(":")
        fields = rest.split()
        if name.strip() == "lo" or len(fields) < 9:
            continue
        rx += int(fields[0])
        tx += int(fields[8])
    return rx, tx


def processes():
    """[{pid, name, uid, jiffies, rss_kb}] for every readable process."""
    out = []
    for entry in os.listdir(PROC):
        if not entry.isdigit():
            continue
        base = os.path.join(PROC, entry)
        try:
            stat = _read(os.path.join(base, "stat"))
            comm = stat[stat.index("(") + 1:stat.rindex(")")]
            fields = stat[stat.rindex(")") + 2:].split()
            rss = 0
            for line in _read(os.path.join(base, "status")).splitlines():
                if line.startswith("VmRSS:"):
                    rss = int(line.split()[1])
                    break
            uid = os.stat(base).st_uid
        except (OSError, ValueError, IndexError):
            continue  # exited while we looked, or not ours to read
        out.append({"pid": int(entry), "name": comm, "uid": uid,
                    "jiffies": int(fields[11]) + int(fields[12]), "rss_kb": rss})
    return out


def process_cpu(prev_jiffies, procs, d_total_jiffies):
    """Per-process CPU % since the last sample, as a share of the whole
    machine (all cores = 100%, like Windows Task Manager). d_total_jiffies is
    the change in cpu_times()[1], which already sums every core."""
    out = {}
    for p in procs:
        before = prev_jiffies.get(p["pid"])
        d = p["jiffies"] - before if before is not None else 0
        out[p["pid"]] = max(0.0, 100.0 * d / max(d_total_jiffies, 1))
    return out


def usernames(path="/etc/passwd"):
    names = {}
    try:
        for line in _read(path).splitlines():
            parts = line.split(":")
            if len(parts) > 2 and parts[2].isdigit():
                names[int(parts[2])] = parts[0]
    except OSError:
        pass
    return names


def human_bytes(n):
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024:
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024
    return f"{n:.1f} TB"
