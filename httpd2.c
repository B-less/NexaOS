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
