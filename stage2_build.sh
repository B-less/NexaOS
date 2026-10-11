#!/bin/bash
set -e

BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

echo "Compiling monitor.c..."
cc -Wall -O2 -static -o $BASE/monitor_bin $BASE/monitor.c
echo "OK"

umount $MNT 2>/dev/null || true
mount -o loop $BASE/nexa.img $MNT

# 1. Install Process Monitor App
mkdir -p $MNT/apps/monitor
cp $BASE/monitor_bin $MNT/apps/monitor/run
chmod +x $MNT/apps/monitor/run

# 2. Add Service Manager to /etc/profile
cat >> $MNT/etc/profile << 'EOF'

# Service Manager
service() {
    [ -z "$1" ] && echo "Usage: service <start|stop|restart|status> <daemon>" && return
    cmd="$1"
    daemon="$2"
    case "$cmd" in
        status)
            if pidof "$daemon" >/dev/null 2>&1; then
                echo "[ OK ] $daemon is running"
            else
                echo "[FAIL] $daemon is stopped"
            fi
            ;;
        stop)
            if pidof "$daemon" >/dev/null 2>&1; then
                killall "$daemon" 2>/dev/null || echo "Failed to kill $daemon"
                echo "Stopped $daemon"
            else
                echo "$daemon is not running"
            fi
            ;;
        start)
            if pidof "$daemon" >/dev/null 2>&1; then
                echo "$daemon is already running"
            else
                "/sbin/$daemon" &
                echo "Started $daemon"
            fi
            ;;
        restart)
            service stop "$daemon"
            sleep 1
            service start "$daemon"
            ;;
        *)
            echo "Usage: service <start|stop|restart|status> <daemon>"
            ;;
    esac
}
EOF

# 3. Add User Management Wrappers
cat > $MNT/bin/useradd << 'EOF'
#!/bin/sh
if [ "$1" = "" ]; then
    echo "Usage: useradd <username>"
    exit 1
fi
echo "Creating user $1..."
adduser -D -h "/home/$1" -s /bin/ash "$1"
passwd "$1"
EOF
chmod +x $MNT/bin/useradd

cat > $MNT/bin/userdel << 'EOF'
#!/bin/sh
if [ "$1" = "" ]; then
    echo "Usage: userdel <username>"
    exit 1
fi
echo "Deleting user $1..."
deluser "$1"
rm -rf "/home/$1"
echo "Done."
EOF
chmod +x $MNT/bin/userdel

# 4. Basic Apps (File Manager & Editor)
mkdir -p $MNT/apps/filemgr
cat > $MNT/apps/filemgr/run << 'EOF'
#!/bin/sh
d="$(pwd)"
while true; do
    clear
    echo -e "\033[1;34m=== Nexa File Manager ===\033[0m"
    echo -e "Path: \033[1;32m$d\033[0m\n"
    ls -la "$d"
    echo
    echo "Commands: cd <dir> | rm <file> | cat <file> | cp <s d> | mv <s d> | q (quit)"
    read -p "filemgr> " cmd arg1 arg2
    case "$cmd" in
        q) break ;;
        cd) [ -d "$d/$arg1" ] && d="$(cd "$d/$arg1" && pwd)" || echo "Not a directory" ;;
        rm) rm -r "$d/$arg1" 2>/dev/null ;;
        cat) cat "$d/$arg1" 2>/dev/null ; echo ; read -p "[Press Enter]" _ ;;
        cp) cp -r "$d/$arg1" "$d/$arg2" 2>/dev/null ;;
        mv) mv "$d/$arg1" "$d/$arg2" 2>/dev/null ;;
        *) echo "Unknown command" ;;
    esac
done
EOF
chmod +x $MNT/apps/filemgr/run

mkdir -p $MNT/apps/editor
cat > $MNT/apps/editor/run << 'EOF'
#!/bin/sh
[ -z "$1" ] && echo "Usage: run editor <filename>" && exit 1
vi "$1"
EOF
chmod +x $MNT/apps/editor/run

# Update permissions
chmod 755 $MNT/home/nexa
chown 1000:1000 $MNT/home/nexa

umount $MNT
echo "Stage 2 complete!"
