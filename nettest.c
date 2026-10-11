#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/ip_icmp.h>
#include <arpa/inet.h>

int main() {
    printf("euid=%d egid=%d\n", geteuid(), getegid());

    // Try SOCK_DGRAM ICMP (unprivileged ping)
    int s = socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP);
    if (s < 0) {
        printf("SOCK_DGRAM ICMP failed: %s (errno=%d)\n", strerror(errno), errno);
    } else {
        printf("SOCK_DGRAM ICMP socket opened OK (fd=%d)\n", s);
        close(s);
    }

    // Try SOCK_RAW ICMP (privileged ping)
    s = socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
    if (s < 0) {
        printf("SOCK_RAW  ICMP failed: %s (errno=%d)\n", strerror(errno), errno);
    } else {
        printf("SOCK_RAW  ICMP socket opened OK (fd=%d)\n", s);
        close(s);
    }

    // Try TCP to confirm basic networking works
    s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) {
        printf("SOCK_STREAM TCP failed: %s\n", strerror(errno));
    } else {
        struct sockaddr_in addr = { .sin_family=AF_INET, .sin_port=htons(80) };
        inet_pton(AF_INET, "8.8.8.8", &addr.sin_addr);
        if (connect(s, (struct sockaddr*)&addr, sizeof(addr)) == 0)
            printf("TCP connect to 8.8.8.8:80 OK — internet works!\n");
        else
            printf("TCP connect failed: %s\n", strerror(errno));
        close(s);
    }
    return 0;
}
