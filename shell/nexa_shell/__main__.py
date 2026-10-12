import sys

from . import control, dock, launchpad, start

PANELS = {"start": start.main, "control": control.main, "dock": dock.main, "launchpad": launchpad.main}

if __name__ == "__main__":
    which = sys.argv[1] if len(sys.argv) > 1 else ""
    if which not in PANELS:
        print("usage: python3 -m nexa_shell start|control|dock|launchpad", file=sys.stderr)
        sys.exit(2)
    sys.exit(PANELS[which]())
