#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <errno.h>
#include <unistd.h>
#include <time.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <netinet/in.h>
#include <netinet/ip_icmp.h>
#include <arpa/inet.h>
#include <netdb.h>

static unsigned short checksum(void *b, int len) {
    unsigned short *buf = b;
    unsigned int sum = 0;
    for (; len > 1; len -= 2) sum += *buf++;
    if (len == 1) sum += *(unsigned char*)buf;
    sum = (sum >> 16) + (sum & 0xFFFF);
    sum += (sum >> 16);
    return ~sum;
}

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "Usage: ping <host>\n"); return 1; }

    int count = -1;
    char *host = argv[1];
    for (int i = 1; i < argc - 1; i++) {
        if (strcmp(argv[i], "-c") == 0) count = atoi(argv[i+1]);
    }
    host = argv[argc-1];

    struct hostent *he = gethostbyname(host);
    struct sockaddr_in addr = {0};
    if (he) {
        addr.sin_family = AF_INET;
        memcpy(&addr.sin_addr, he->h_addr, he->h_length);
    } else if (inet_pton(AF_INET, host, &addr.sin_addr) != 1) {
        fprintf(stderr, "ping: unknown host %s\n", host); return 1;
    }
    addr.sin_family = AF_INET;

    int sock = socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP);
    if (sock < 0) {
        perror("socket"); return 1;
    }

    struct timeval tv = {2, 0};
    setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

    printf("PING %s (%s): 56 data bytes\n", host, inet_ntoa(addr.sin_addr));

    int sent = 0, received = 0;
    while (count < 0 || sent < count) {
        char buf[64];
        struct icmphdr *icmp = (struct icmphdr*)buf;
        memset(buf, 0, sizeof(buf));
        icmp->type = ICMP_ECHO;
        icmp->code = 0;
        icmp->un.echo.id = getpid() & 0xFFFF;
        icmp->un.echo.sequence = sent + 1;
        memset(buf + sizeof(*icmp), 0xA5, 56 - sizeof(*icmp));
        icmp->checksum = checksum(buf, 56);

        struct timeval t0, t1;
        gettimeofday(&t0, NULL);
        if (sendto(sock, buf, 56, 0, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
            perror("sendto"); break;
        }
        sent++;

        struct sockaddr_in from;
        socklen_t flen = sizeof(from);
        int n = recvfrom(sock, buf, sizeof(buf), 0, (struct sockaddr*)&from, &flen);
        gettimeofday(&t1, NULL);
        if (n > 0) {
            double ms = (t1.tv_sec-t0.tv_sec)*1000.0 + (t1.tv_usec-t0.tv_usec)/1000.0;
            printf("64 bytes from %s: icmp_seq=%d ttl=64 time=%.1f ms\n",
                   inet_ntoa(from.sin_addr), sent, ms);
            received++;
        } else {
            printf("Request timeout for icmp_seq=%d\n", sent);
        }
        if (count < 0 || sent < count) sleep(1);
    }

    printf("\n--- %s ping statistics ---\n", host);
    printf("%d packets transmitted, %d received, %d%% packet loss\n",
           sent, received, sent ? (sent-received)*100/sent : 0);
    close(sock);
    return received == 0 ? 1 : 0;
}
