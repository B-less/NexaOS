#!/bin/sh
# /etc/profile - Nexa OS login environment

export PATH="/bin:/sbin:/usr/bin:/usr/sbin:/apps/gui"
export HOME="${HOME:-/home/nexa}"
export USER="${USER:-nexa}"
export TERM="${TERM:-vt100}"
export HISTFILE="$HOME/.nexa_history"
export HISTSIZE=500

# Nexa OS prompt
export PS1="\[\033[1;36m\]nexa\[\033[0m\]> "

# System paths
APPS="/apps"
PKGDB="/packages/installed.db"
BOOT_LOG="/logs/boot.log"

# --- Helper functions available in every session ---

nx_valid_name() {
    case "$1" in
        ''|*[!a-zA-Z0-9_-]*) return 1 ;;
        *) return 0 ;;
    esac
}

nx_sync_pkgdb() {
    tmp="/packages/installed.tmp"
    : > "$tmp" 2>/dev/null || return 1
    for d in /apps/*/; do
        [ -d "$d" ] || continue
        pkg="${d%/}"; pkg="${pkg##*/}"
        nx_valid_name "$pkg" && [ -f "/apps/$pkg/run" ] && echo "$pkg" >> "$tmp"
    done
    mv "$tmp" "$PKGDB" 2>/dev/null
}

# pkg wrapper
pkg() {
    case "$1" in
        list)
            nx_sync_pkgdb
            echo; echo "Installed Packages:"; cat "$PKGDB" 2>/dev/null || echo "(none)"; echo ;;
        available)
            echo; echo "Available in /repo:"
            ls /repo 2>/dev/null || echo "(none)"; echo ;;
        info)
            echo; echo "Package: $2"
            [ -d "/apps/$2" ] && echo "Status: installed" || echo "Status: not installed"
            echo ;;
        install)
            [ -z "$2" ] && echo "Usage: pkg install <name>" && return
            nx_valid_name "$2" || { echo "Invalid name"; return; }
            if [ -d "/apps/$2" ]; then
                echo "Already installed: $2"
            elif [ -d "/repo/$2" ]; then
                cp -r "/repo/$2" "/apps/$2" && chmod +x "/apps/$2/run"
                nx_sync_pkgdb; echo "Installed: $2"
            else
                echo "Fetching from package server..."
                tmpdir="/tmp/pkg_$2_$$"
                mkdir -p "$tmpdir"
                if wget -q -O "/tmp/$2.tar.gz" "http://10.0.2.2:8000/$2.tar.gz" 2>/dev/null; then
                    tar -xzf "/tmp/$2.tar.gz" -C "$tmpdir" 2>/dev/null
                    [ -f "$tmpdir/run" ] || { [ -d "$tmpdir/$2" ] && mv "$tmpdir/$2"/* "$tmpdir/"; }
                    if [ -f "$tmpdir/run" ]; then
                        mv "$tmpdir" "/apps/$2"
                        chmod +x "/apps/$2/run"
                        nx_sync_pkgdb; echo "Installed: $2"
                    else
                        echo "Package missing run file"; rm -rf "$tmpdir"
                    fi
                    rm -f "/tmp/$2.tar.gz"
                else
                    echo "Not found: $2"; rm -rf "$tmpdir"
                fi
            fi
            ;;
        remove)
            [ -z "$2" ] && echo "Usage: pkg remove <name>" && return
            case " about calculator desktop-gui editor filemgr menu monitor todo " in
                *" $2 "*) echo "Cannot remove core package"; return ;;
            esac
            [ -d "/apps/$2" ] || { echo "Not installed: $2"; return; }
            rm -rf "/apps/$2"; nx_sync_pkgdb; echo "Removed: $2" ;;
        search)
            echo; echo "Searching for: $2"
            ls /repo 2>/dev/null | grep "$2"
            wget -q -O - "http://10.0.2.2:8000/index.txt" 2>/dev/null | grep "$2" || true
            echo ;;
        *)
            echo "pkg: list | available | info <n> | install <n> | remove <n> | search <n>" ;;
    esac
}

# System info
system() {
    echo
    echo "=== Nexa OS System ==="
    echo "Kernel:   $(uname -r)"
    echo "Arch:     $(uname -m)"
    echo "Uptime:   $(cut -d' ' -f1 /proc/uptime)s"
    echo "Hostname: $(hostname)"
    nx_sync_pkgdb
    echo "Packages: $(wc -l < "$PKGDB" 2>/dev/null || echo 0)"
    echo
}

# Network info
netinfo() {
    echo
    ifconfig 2>/dev/null
    echo "DNS: $(cat /etc/resolv.conf 2>/dev/null)"
    echo
}

# Disk info
disk() {
    echo; df -h /; echo
}

# Process list
procs() {
    echo; ps aux 2>/dev/null || ps; echo
}

# Logs
logs() {
    case "$1" in
        ''|boot) cat "$BOOT_LOG" 2>/dev/null || echo "No logs." ;;
        kernel)  dmesg | tail -30 ;;
        sys)     cat /var/log/messages 2>/dev/null | tail -30 || echo "No syslog yet." ;;
        clear)   : > "$BOOT_LOG"; echo "Cleared." ;;
        *) echo "logs [boot|kernel|sys|clear]" ;;
    esac
}

# Launch GUI
gui() {
    if [ -x "/apps/desktop-gui/run" ]; then
        /apps/desktop-gui/run
    else
        echo "GUI not found. Run 'pkg list' to check installed apps."
    fi
}

# Run an app
run() {
    [ -z "$1" ] && echo "Usage: run <app>" && return
    nx_valid_name "$1" || { echo "Invalid app name"; return; }
    if [ -x "/apps/$1/run" ]; then
        "/apps/$1/run"
    else
        echo "App not found: $1"
        echo "Installed apps: $(ls /apps 2>/dev/null | tr '\n' ' ')"
    fi
}

# Apps list
apps() {
    echo; echo "Installed apps:"; ls /apps 2>/dev/null; echo
}

# Nexa help
help() {
    cat << 'EOF'

  Nexa OS - Command Reference
  ============================
  SYSTEM
    system          System info
    disk            Disk usage
    procs           Process list
    netinfo         Network info
    logs [type]     Show logs (boot|kernel|sys|clear)
    diag            Full diagnostics

  APPS
    apps            List installed apps
    run <app>       Launch an app
    gui             Launch graphical desktop

  PACKAGES
    pkg list        Installed packages
    pkg available   Packages in local repo
    pkg search <n>  Search for packages
    pkg install <n> Install a package
    pkg remove <n>  Remove a package

  USERS
    passwd          Change your password
    whoami          Show current user
    who             Show logged-in users
    su <user>       Switch user

  NETWORK
    ping <host>     Ping a host
    wget <url>      Download a file
    netstat         Show network connections
    nslookup <h>    DNS lookup

  TOOLS
    vi <file>       Text editor
    cat/ls/cp/mv    Standard file tools
    tar/gzip        Archive tools
    sh              Drop to raw shell
    exit            Log out

EOF
}

# Diagnostics
diag() {
    echo
    echo "=== Nexa OS Diagnostics ==="
    echo "Kernel:  $(uname -r)"
    echo "Uptime:  $(cut -d' ' -f1 /proc/uptime)s"
    echo
    echo "--- Memory ---"
    grep 'MemTotal\|MemFree\|MemAvailable' /proc/meminfo
    echo
    echo "--- Disk ---"
    df -h /
    echo
    echo "--- Network ---"
    ifconfig eth0 2>/dev/null
    echo
    echo "--- Services ---"
    echo "httpd:   $(pidof httpd  > /dev/null 2>&1 && echo RUNNING || echo STOPPED)"
    echo "syslogd: $(pidof syslogd > /dev/null 2>&1 && echo RUNNING || echo STOPPED)"
    echo "crond:   $(pidof crond  > /dev/null 2>&1 && echo RUNNING || echo STOPPED)"
    echo
}

# Source any extra profile drops
for f in /etc/profile.d/*.sh; do
    [ -f "$f" ] && . "$f"
done

# MOTD
[ -f /etc/motd ] && cat /etc/motd

export -f pkg system netinfo disk procs logs gui run apps help diag nx_valid_name nx_sync_pkgdb 2>/dev/null || true
