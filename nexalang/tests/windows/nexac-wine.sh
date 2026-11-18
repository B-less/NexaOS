#!/bin/sh
# Runs the Windows nexac (dist/nexac.exe, from editors/windows/build.sh)
# under Wine, as if it were a Linux nexac. Test-only (0.9):
#
#   NEXAC_TEST_WINDOWS=1 NEXAC_TEST_EXE=$PWD/tests/windows/nexac-wine.sh cargo test --test tooling
#   python3 tests/windows/differential.py target/release/nexac tests/windows/nexac-wine.sh 1500
#
# UTF-8 file names need a UTF-8 locale (Wine maps names through it). Wine
# 9 crashes ("free(): invalid pointer") when $XDG_RUNTIME_DIR's usual
# folder /run/user/<uid> is missing: create it (mode 700) first.
# (full paths: the extension tests run it with a PATH of one folder)
here=$(/usr/bin/dirname "$(/usr/bin/readlink -f "$0")")
export LANG=C.UTF-8 WINEDEBUG=-all
exec /usr/bin/wine "${NEXAC_EXE:-$here/../../dist/nexac.exe}" "$@"
