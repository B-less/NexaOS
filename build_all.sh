#!/bin/bash
set -e
BASE="/home/nexa/NexaOS"
MNT="$BASE/mnt"

echo "=============================="
echo "  NEXA OS FULL FEATURE BUILD"
echo "=============================="

# --- Build kernel module ---
echo "[1/6] Building /proc/nexa kernel module..."
cd "$BASE/nexa_proc"
make -s 2>/dev/null && echo "  OK: nexa_proc.ko built" || echo "  WARN: kernel module build failed (non-fatal)"
cd "$BASE"

echo "[2/6] Mounting disk..."
umount "$MNT" 2>/dev/null || true
mount -o loop "$BASE/nexa.img" "$MNT"

# --- Install profile ---
echo "[3/6] Installing shell environment..."
cp "$BASE/profile" "$MNT/etc/profile"
chmod 644 "$MNT/etc/profile"

mkdir -p "$MNT/etc/profile.d"

# --- Install nexa-admin ---
cp "$BASE/nexa-admin.sh" "$MNT/sbin/nexa-admin"
chmod +x "$MNT/sbin/nexa-admin"

# --- Install MOTD ---
cp "$BASE/motd" "$MNT/etc/motd"

# --- Install kernel module ---
if [ -f "$BASE/nexa_proc/nexa_proc.ko" ]; then
    mkdir -p "$MNT/lib/modules"
    cp "$BASE/nexa_proc/nexa_proc.ko" "$MNT/lib/modules/"
    echo "  OK: nexa_proc.ko installed to /lib/modules/"
fi

# --- Add syslogd, klogd, crond to inittab ---
echo "[4/6] Configuring system daemons..."
cat > "$MNT/etc/inittab" << 'EOF'
tty0:1:respawn:/sbin/getty -L tty0 115200 vt100
ttyS0:1:respawn:/sbin/getty -L ttyS0 115200 vt100
httpd:1:respawn:/sbin/httpd
syslogd:1:once:/bin/syslogd -L -O /var/log/messages
klogd:1:once:/bin/klogd
crond:1:once:/bin/crond -l 8
EOF

# --- Upgrade rcS ---
cat > "$MNT/etc/init.d/rcS" << 'EOF'
#!/bin/sh
mount -a 2>/dev/null
hostname nexa
mkdir -p /var/log /var/run /var/spool/cron/crontabs /tmp
chmod 1777 /tmp
echo "nameserver 8.8.8.8" > /etc/resolv.conf
echo "nameserver 8.8.4.4" >> /etc/resolv.conf
ifconfig lo 127.0.0.1 up
ifconfig eth0 up
udhcpc -i eth0 -q -t 3 >/dev/null 2>&1 &
# Load nexa kernel module if present
[ -f /lib/modules/nexa_proc.ko ] && insmod /lib/modules/nexa_proc.ko 2>/dev/null || true
# Start syslogd
syslogd -L -O /var/log/messages 2>/dev/null &
klogd 2>/dev/null &
crond 2>/dev/null &
echo "Nexa OS ready."
EOF
chmod +x "$MNT/etc/init.d/rcS"

# --- Upgrade httpd to serve /repo as package server ---
echo "[5/6] Upgrading httpd to package server..."
cat > "$BASE/httpd2.c" << 'CSRC'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <dirent.h>

#define PORT 8000
#define ROOT "/repo"

void send_404(int fd) {
    const char *r = "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\n\r\nNot found\n";
    write(fd, r, strlen(r));
}

void serve_index(int fd) {
    char buf[65536];
    int len = 0;
    len += snprintf(buf+len, sizeof(buf)-len,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n"
        "<html><head><title>Nexa Package Server</title>"
        "<style>body{font-family:monospace;background:#111;color:#0f0;padding:20px}"
        "h1{color:#0af}a{color:#0f0}table{width:100%%}tr:hover{background:#222}</style>"
        "</head><body><h1>Nexa OS Package Server</h1><table>");
    DIR *d = opendir(ROOT);
    if (d) {
        struct dirent *e;
        while ((e = readdir(d))) {
            if (e->d_name[0] == '.') continue;
            char path[512];
            snprintf(path, sizeof(path), ROOT"/%s/%s.tar.gz", e->d_name, e->d_name);
            struct stat st;
            if (stat(path, &st) == 0) {
                len += snprintf(buf+len, sizeof(buf)-len,
                    "<tr><td><a href='/%s.tar.gz'>%s</a></td><td>%lld bytes</td></tr>",
                    e->d_name, e->d_name, (long long)st.st_size);
            }
        }
        closedir(d);
    }
    len += snprintf(buf+len, sizeof(buf)-len, "</table></body></html>");
    write(fd, buf, len);
}

void serve_file(int fd, const char *name) {
    char path[512];
    /* Strip .tar.gz to get package name */
    char pkg[256];
    strncpy(pkg, name, sizeof(pkg)-1);
    char *dot = strstr(pkg, ".tar.gz");
    if (dot) *dot = 0;

    snprintf(path, sizeof(path), ROOT"/%s/%s.tar.gz", pkg, pkg);
    int f = open(path, O_RDONLY);
    if (f < 0) { send_404(fd); return; }

    struct stat st;
    fstat(f, &st);
    char hdr[256];
    snprintf(hdr, sizeof(hdr),
        "HTTP/1.1 200 OK\r\nContent-Type: application/gzip\r\nContent-Length: %lld\r\n\r\n",
        (long long)st.st_size);
    write(fd, hdr, strlen(hdr));
    char buf[4096];
    int n;
    while ((n = read(f, buf, sizeof(buf))) > 0)
        write(fd, buf, n);
    close(f);
}

void serve_index_txt(int fd) {
    char buf[8192];
    int len = 0;
    len += snprintf(buf+len, sizeof(buf)-len, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n");
    DIR *d = opendir(ROOT);
    if (d) {
        struct dirent *e;
        while ((e = readdir(d))) {
            if (e->d_name[0] != '.') {
                len += snprintf(buf+len, sizeof(buf)-len, "%s\n", e->d_name);
            }
        }
        closedir(d);
    }
    write(fd, buf, len);
}

void handle(int fd) {
    char req[1024] = {0};
    read(fd, req, sizeof(req)-1);
    char method[16], path[512];
    sscanf(req, "%15s %511s", method, path);

    if (strcmp(path, "/") == 0 || strcmp(path, "/index.html") == 0) {
        serve_index(fd);
    } else if (strcmp(path, "/index.txt") == 0) {
        serve_index_txt(fd);
    } else if (strstr(path, ".tar.gz")) {
        serve_file(fd, path+1);
    } else {
        send_404(fd);
    }
    close(fd);
}

int main() {
    int srv = socket(AF_INET, SOCK_STREAM, 0);
    int opt = 1;
    setsockopt(srv, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));
    struct sockaddr_in addr = {AF_INET, htons(PORT), {INADDR_ANY}};
    bind(srv, (struct sockaddr*)&addr, sizeof(addr));
    listen(srv, 10);
    printf("NexaOS httpd package server on port %d...\n", PORT);
    while (1) {
        int fd = accept(srv, NULL, NULL);
        if (fd >= 0) {
            if (fork() == 0) { close(srv); handle(fd); exit(0); }
            close(fd);
        }
    }
}
CSRC

cc -Wall -O2 -static -o "$BASE/httpd_bin" "$BASE/httpd2.c"
cp "$BASE/httpd_bin" "$MNT/sbin/httpd"
echo "  OK: httpd upgraded to package server"

# --- Create var directories ---
mkdir -p "$MNT/var/log" "$MNT/var/run" "$MNT/var/spool/cron/crontabs"

# --- Set up home directory ---
mkdir -p "$MNT/home/nexa"
cat > "$MNT/home/nexa/.profile" << 'EOF'
. /etc/profile
EOF
chown -R 1000:1000 "$MNT/home/nexa"

# --- Ensure /etc/passwd uses nexa-admin ---
cat > "$MNT/etc/passwd" << 'EOF'
root:x:0:0:root:/root:/bin/sh
nexa:x:1000:1000:Nexa User:/home/nexa:/sbin/nexa-admin
EOF

HASH=$(openssl passwd -1 password)
printf 'root:%s:19000:0:99999:7:::\n' "$HASH" > "$MNT/etc/shadow"
printf 'nexa:%s:19000:0:99999:7:::\n' "$HASH" >> "$MNT/etc/shadow"
chmod 600 "$MNT/etc/shadow"

echo "[6/6] Unmounting..."
umount "$MNT"

echo
echo "=============================="
echo "  BUILD COMPLETE!"
echo "  Boot with: ./boot.sh gui"
echo "  Login: nexa / password"
echo "=============================="
