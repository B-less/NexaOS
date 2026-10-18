import sys

from . import control, desktop, dock, nexus

# "start" and "launchpad" are the old names (bar button, Super+D, saved
# configs); both open Nexus now.
PANELS = {"nexus": nexus.main, "start": nexus.main, "launchpad": nexus.main,
          "control": control.main, "dock": dock.main, "desktop": desktop.main}

if __name__ == "__main__":
    which = sys.argv[1] if len(sys.argv) > 1 else ""
    if which not in PANELS:
        print("usage: python3 -m nexa_shell nexus|control|dock|desktop", file=sys.stderr)
        sys.exit(2)
    sys.exit(PANELS[which]())
