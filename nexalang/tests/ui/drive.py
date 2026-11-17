"""Drive a NexaLang window program the way a person would: real key
presses (xdotool, through the X server's XTEST) on a private headless X
server (Xvfb). Test-only; not part of NexaOS.

    drive.py STEPS -- COMMAND [ARGS...]

STEPS is a file with one step per line:
    file PATH TEXT    before the program starts: a file in the new $HOME
                      (TEXT may use \\xNN escapes for bytes)
    write PATH TEXT   while it runs: write a file in $HOME (same escapes)
    window TITLE      wait until a window with this title is shown, focus it
    size W H          resize the last window found (W and H in pixels)
    type TEXT         type the text
    key K [K...]      press keys (xdotool names: Return, Tab, ctrl+n, Escape)
    wait SECONDS      let the program catch up
    shot NAME         screenshot the screen into $NX_SHOTS/NAME.png (if set)
    # ...             a comment

The program's output is passed through (with the new $HOME shown as
/HOME); the exit code is the program's (124 if it hasn't finished
$NX_WINDOW_WAIT seconds, default 15, after the last step). $HOME is a new
empty folder unless NX_KEEP_HOME is set.
"""
import os, shutil, subprocess, sys, tempfile, time

def put_file(home, spec):
    rel, _, text = spec.partition(" ")
    path = os.path.join(home, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(text.encode("utf-8").decode("unicode_escape").encode("latin-1"))
    os.replace(tmp, path)  # the way a theme switcher replaces it

def xdotool(env, args):
    """Run xdotool; again if it couldn't connect (a busy X server can
    refuse a connection for a moment)."""
    for _ in range(20):
        r = subprocess.run(["xdotool", *args], env=env, check=False, capture_output=True, timeout=20)
        if r.returncode == 0 or b"open display" not in r.stderr:
            return r
        time.sleep(0.2)
    return r

def word_of(line):
    return line.partition(" ")[0]

def main():
    sep = sys.argv.index("--")
    steps_file, cmd = sys.argv[1], sys.argv[sep + 1:]
    shots = os.environ.get("NX_SHOTS")
    # Our own display number, claimed with the X lock file (O_EXCL), then
    # wait until the server takes connections. (Xvfb's -displayfd sometimes
    # handed two servers started at the same moment a number a program
    # then couldn't connect to.)
    xvfb, num = None, None
    for n in range(200 + os.getpid() % 300, 1000):
        lock = f"/tmp/.X{n}-lock"
        if os.path.exists(lock) or os.path.exists(f"/tmp/.X11-unix/X{n}"):
            continue
        xvfb = subprocess.Popen(["Xvfb", f":{n}", "-screen", "0", "1024x768x24", "-nolisten", "tcp"],
                                stderr=subprocess.DEVNULL)
        ok = False
        for _ in range(100):
            if xvfb.poll() is not None:
                break
            if subprocess.run(["xdotool", "getmouselocation"], env=dict(os.environ, DISPLAY=f":{n}"),
                              capture_output=True).returncode == 0:
                ok = True
                break
            time.sleep(0.1)
        if ok:
            num = str(n)
            break
        xvfb.kill()
        xvfb.wait()
    if num is None:
        sys.exit("drive: couldn't start Xvfb")
    env = dict(os.environ)
    env["DISPLAY"] = ":" + num
    env.pop("WAYLAND_DISPLAY", None)
    home = None
    if "NX_KEEP_HOME" not in env:
        home = tempfile.mkdtemp(prefix="nexa-ui-home-")
        env["HOME"] = home
        for k in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
            env.pop(k, None)
    env["GDK_BACKEND"] = "x11"
    env["NO_AT_BRIDGE"] = "1"  # no accessibility bus in a headless test
    for raw in open(steps_file, encoding="utf-8"):
        if raw.startswith("file ") and home:
            put_file(home, raw.rstrip("\n").split(" ", 1)[1])
    window_wait = float(env.get("NX_WINDOW_WAIT", "15"))
    last_window = ""
    prog = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE)
    out = []
    import threading
    reader = threading.Thread(target=lambda: out.append(prog.stdout.read()))
    reader.start()
    code = None
    try:
        for raw in open(steps_file, encoding="utf-8"):
            line = raw.rstrip("\n")
            if not line.strip() or line.lstrip().startswith("#") or word_of(line) == "file":
                continue
            word, _, rest = line.partition(" ")
            x = lambda *a: xdotool(env, a)
            if word == "window":
                # one connection that waits (--sync), not a new connection
                # every moment: under load, quick reconnects filled the X
                # server's queue and the program's own connect got EAGAIN
                deadline = time.time() + window_wait
                wid = ""
                while not wid and prog.poll() is None and time.time() < deadline:
                    search = subprocess.Popen(["xdotool", "search", "--sync", "--onlyvisible", "--name", "^" + rest + "$"],
                                              env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
                    while search.poll() is None and prog.poll() is None and time.time() < deadline:
                        time.sleep(0.05)
                    if search.poll() is None:
                        search.kill()
                    found = search.communicate()[0].split()
                    if found:
                        wid = found[-1].decode()
                    else:
                        time.sleep(0.3)  # xdotool couldn't connect: try again
                if not wid:
                    print(f"drive: no window called {rest!r} (display {env['DISPLAY']})", file=sys.stderr)
                    break
                last_window = wid
                x("windowfocus", "--sync", wid)
                time.sleep(0.3)
            elif word == "size":
                w, h = rest.split()
                x("windowsize", "--sync", last_window, w, h)
                time.sleep(0.3)
            elif word == "type":
                x("type", "--delay", "15", rest)
            elif word == "key":
                x("key", "--delay", "40", *rest.split())
            elif word == "wait":
                time.sleep(float(rest))
                continue
            elif word == "write":
                put_file(home, rest)
                time.sleep(0.5)
                continue
            elif word == "shot":
                time.sleep(0.4)  # let transitions (160 ms) finish
                if shots:
                    os.makedirs(shots, exist_ok=True)
                    subprocess.run(["import", "-window", "root", os.path.join(shots, rest + ".png")], env=env, check=False)
                continue
            else:
                sys.exit(f"drive: unknown step {line!r}")
            time.sleep(0.25)
            if prog.poll() is not None:
                break
        try:
            code = prog.wait(timeout=window_wait)
        except subprocess.TimeoutExpired:
            prog.kill()
            prog.wait()
            code = 124
    finally:
        if prog.poll() is None:
            prog.kill()
        xvfb.terminate()
        xvfb.wait()
        reader.join()
        text = b"".join(out)
        if home:
            text = text.replace(home.encode(), b"/HOME")
        sys.stdout.buffer.write(text)
        sys.stdout.flush()
        if home:
            shutil.rmtree(home, ignore_errors=True)
    sys.exit(code)

main()
