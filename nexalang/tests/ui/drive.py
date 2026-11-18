"""Drive a NexaLang window program the way a person would: real key
presses (xdotool, through the X server's XTEST) on a private headless X
server (Xvfb). Test-only; not part of NexaOS.

    drive.py STEPS -- COMMAND [ARGS...]

STEPS is a file with one step per line:
    file PATH TEXT    before the program starts: a file in the new $HOME
                      (TEXT may use \\xNN escapes for bytes)
    write PATH TEXT   while it runs: write a file in $HOME (same escapes)
    window TITLE      wait until a window with this title is shown, focus it
    gone TITLE        wait until no window has this title (a dialog closed)
    size W H          resize the last window found (W and H in pixels)
    type TEXT         type the text
    key K [K...]      press keys (xdotool names: Return, Tab, ctrl+n, Escape)
    wait SECONDS      let the program catch up
    shot NAME         screenshot the screen into $NX_SHOTS/NAME.png (if set)
    fill PATH N TEXT  before the program starts: a file in $HOME of N bytes,
                      TEXT repeated (hostile-input tests: huge files)
    show PATH         after the program ends: print that file from $HOME
                      (or "(no file)"), with 10-digit numbers (Unix times)
                      shown as T and bytes that aren't UTF-8 as \\xNN, so
                      saved data can be compared exactly
    # ...             a comment

The program's output is passed through (with the new $HOME shown as
/HOME); the exit code is the program's (124 if it hasn't finished
$NX_WINDOW_WAIT seconds, default 15, after the last step). $HOME is a new
empty folder unless NX_KEEP_HOME is set. NEXALANG_UI_TREE is set, so
`key ctrl+shift+F12` prints the window's visible widgets (0.7).
NX_SCREEN sets the screen's size (default 1024x768; a bigger one keeps the
pointer, which starts in the middle, off the window: no tooltips in
screenshots).
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
        xvfb = subprocess.Popen(["Xvfb", f":{n}", "-screen", "0", os.environ.get("NX_SCREEN", "1024x768") + "x24", "-nolisten", "tcp"],
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
    # no session bus here: GTK's file chooser keeps its settings in memory
    env["GSETTINGS_BACKEND"] = "memory"
    # Ctrl+Shift+F12 prints the window's widgets (0.7): steps compare them
    env["NEXALANG_UI_TREE"] = "1"
    shows = []
    for raw in open(steps_file, encoding="utf-8"):
        if raw.startswith("file ") and home:
            put_file(home, raw.rstrip("\n").split(" ", 1)[1])
        elif raw.startswith("fill ") and home:
            rel, n, text = raw.rstrip("\n").split(" ", 3)[1:]
            path = os.path.join(home, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            data = text.encode() * (int(n) // len(text.encode()) + 1)
            with open(path, "wb") as f:
                f.write(data[:int(n)])
        elif raw.startswith("show "):
            shows.append(raw.rstrip("\n").split(" ", 1)[1])
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
            if not line.strip() or line.lstrip().startswith("#") or word_of(line) in ("file", "fill", "show"):
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
            elif word == "gone":
                # wait until no window has this title any more (a dialog closed)
                deadline = time.time() + window_wait
                while prog.poll() is None and time.time() < deadline:
                    r = x("search", "--onlyvisible", "--name", "^" + rest + "$")
                    if r.returncode != 0 or not r.stdout.split():
                        break
                    time.sleep(0.2)
                else:
                    if prog.poll() is None:
                        print(f"drive: the window {rest!r} is still there", file=sys.stderr)
                        break
                time.sleep(0.3)
                continue
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
            import re
            for rel in shows:
                path = os.path.join(home, rel)
                body = open(path, "rb").read() if os.path.exists(path) else b"(no file)\n"
                body = body.decode("utf-8", "backslashreplace").encode()  # bytes that aren't text as \xNN
                text += f"--- {rel}\n".encode() + re.sub(rb"\b\d{10}\b", b"T", body)
        if home:
            text = text.replace(home.encode(), b"/HOME")
        sys.stdout.buffer.write(text)
        sys.stdout.flush()
        if home:
            shutil.rmtree(home, ignore_errors=True)
    sys.exit(code)

main()
