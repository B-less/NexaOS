import sys

from . import desktop, drives, linknotify, nexus, parts

# "start" and "launchpad" are the old names (bar button, Super+D, saved
# configs); both open Nexus now.
PANELS = {"nexus": nexus.main, "start": nexus.main, "launchpad": nexus.main,
          "foryou": desktop.main,
          "drives": drives.main, "linknotify": linknotify.main, "parts": parts.main}

if __name__ == "__main__":
    which = sys.argv[1] if len(sys.argv) > 1 else ""
    if which not in PANELS:
        print("usage: python3 -m nexa_shell nexus|foryou|drives|linknotify|parts", file=sys.stderr)
        sys.exit(2)
    sys.exit(PANELS[which]())
