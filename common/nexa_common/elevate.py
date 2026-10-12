"""Run a Nexa root helper through doas, asking for the user's password only
when doas needs one.

The live USB permits every helper without a password; an installed system
requires it for helpers that can change the system (app and package
installs, the assistant's control tools). Callers try without a password
first and catch PasswordRequired to ask for one.

doas reads passwords from the controlling terminal only, so a password is
handed over through a private pseudo-terminal -- never argv, never the
helper's stdin (which carries the JSON request).
"""
import fcntl
import os
import re
import select
import subprocess
import termios
import time

DOAS = "doas"
_NEEDS_PASSWORD = re.compile(r"authenticat\w* required|authoriz\w* required|password is required", re.I)
_BAD_PASSWORD = re.compile(r"authentication failed|incorrect password|sorry", re.I)
MAX_ATTEMPTS = 3


class PasswordRequired(Exception):
    pass


class WrongPassword(Exception):
    pass


def run(helper, request, password=None, timeout=60):
    """Run `doas helper` with request (str) on stdin; returns the
    CompletedProcess. Raises PasswordRequired / WrongPassword."""
    if password is None:
        p = subprocess.run([DOAS, "-n", helper], input=request, capture_output=True,
                           text=True, timeout=timeout)
        if p.returncode != 0 and _NEEDS_PASSWORD.search(p.stderr or ""):
            raise PasswordRequired()
        return p
    return _run_with_password(helper, request, password, timeout)


def _run_with_password(helper, request, password, timeout):
    master, slave = os.openpty()

    def give_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    # The parent keeps its own end of the terminal open throughout: until
    # doas opens /dev/tty, reading the master would fail (EIO) and a password
    # written then would be thrown away, leaving doas waiting forever.
    try:
        p = subprocess.Popen([DOAS, helper], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, preexec_fn=give_terminal,
                             pass_fds=(slave,))
    except OSError:
        os.close(slave)
        os.close(master)
        raise
    try:
        seen, deadline = b"", time.monotonic() + 10
        while b"assword" not in seen and time.monotonic() < deadline and p.poll() is None:
            ready, _, _ = select.select([master], [], [], 0.2)
            if ready:
                try:
                    seen += os.read(master, 1024)
                except OSError:
                    break
        if p.poll() is None:
            os.write(master, password.encode() + b"\n")
        out, err = p.communicate(request, timeout=timeout)
    finally:
        os.close(master)
        os.close(slave)
    if p.returncode != 0 and _BAD_PASSWORD.search(err or ""):
        raise WrongPassword()
    return subprocess.CompletedProcess(p.args, p.returncode, out, err)


def run_interactive(helper, request, ask, timeout=60):
    """Try without a password; if one is needed, call ask(retry) for it
    (None means the user cancelled) and allow a few attempts. Returns the
    CompletedProcess, or None if the user cancelled."""
    try:
        return run(helper, request, timeout=timeout)
    except PasswordRequired:
        pass
    for attempt in range(MAX_ATTEMPTS):
        password = ask(attempt > 0)
        if password is None:
            return None
        try:
            return run(helper, request, password=password, timeout=timeout)
        except WrongPassword:
            continue
    raise WrongPassword()
