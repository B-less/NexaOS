#!/bin/sh
# /sbin/nexa-admin — Nexa OS login shell
# Thin wrapper that sets up the environment and execs ash

# Source the system profile
. /etc/profile 2>/dev/null

# Run ash as a login shell (gives history, tab completion, job control)
exec /bin/ash --login
