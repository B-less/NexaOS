/* nexa-later: start the desktop's background services once what is on the
 * screen has drawn.
 *
 *   nexa-later [--min S] [--max S] 'command' ['command' ...]
 *
 * At login the compositor starts everything at once: the lock screen, bar
 * and Dock compete with MicroBrain, Patterns, Link, Doctor... for the CPU
 * and the disk, and the parts you see come up last. nexa-later waits until
 * the system has settled -- the kernel's pressure counters (PSI) show
 * almost no task waiting for the CPU or the disk for two seconds in a row
 * -- then starts each command (through /bin/sh -c, in its own session, 0.2 s
 * apart) and exits. --min (default 2 s) lets the foreground start loading
 * first; --max (default 20 s) starts them anyway on a machine that never
 * looks idle. Without PSI it waits until the CPU (counting disk waits) is
 * mostly idle, by /proc/stat.
 *
 * Says what it decided on stderr (the session log), e.g.
 *   nexa-later: settled after 6.0 s (cpu 4%, io 1%); starting 6
 */
#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define CPU_SETTLED 0.25  /* share of time some task waited for a CPU */
#define IO_SETTLED  0.10  /* share of time some task waited for the disk */
#define STAT_SETTLED 0.30 /* fallback: busy share of all CPUs, disk waits included */
#define QUIET_SAMPLES 2   /* one-second samples in a row */

static double now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

static void sleep_s(double s) {
    if (s <= 0) return;
    struct timespec ts = {(time_t)s, (long)((s - (time_t)s) * 1e9)};
    while (nanosleep(&ts, &ts) < 0 && errno == EINTR) {}
}

/* "some ... total=N" from /proc/pressure/<what>: microseconds during which
 * at least one task stalled. -1 when PSI isn't there. */
static long long psi_some_total(const char *what) {
    char path[64], line[256];
    snprintf(path, sizeof(path), "/proc/pressure/%s", what);
    FILE *f = fopen(path, "re");
    if (!f) return -1;
    long long total = -1;
    while (fgets(line, sizeof(line), f)) {
        char *t = strstr(line, "total=");
        if (strncmp(line, "some", 4) == 0 && t) total = atoll(t + 6);
    }
    fclose(f);
    return total;
}

/* Busy and total jiffies of all CPUs; disk waits (iowait) count as busy. */
static int stat_sample(unsigned long long *busy, unsigned long long *total) {
    FILE *f = fopen("/proc/stat", "re");
    if (!f) return -1;
    unsigned long long v[8] = {0};
    int n = fscanf(f, "cpu %llu %llu %llu %llu %llu %llu %llu %llu",
                   &v[0], &v[1], &v[2], &v[3], &v[4], &v[5], &v[6], &v[7]);
    fclose(f);
    if (n < 5) return -1;
    *total = 0;
    for (int i = 0; i < 8; i++) *total += v[i];
    *busy = *total - v[3];  /* everything but idle */
    return 0;
}

static void start(const char *cmd) {
    pid_t pid = fork();
    if (pid != 0) return;
    setsid();
    signal(SIGCHLD, SIG_DFL);
    execl("/bin/sh", "sh", "-c", cmd, (char *)NULL);
    _exit(127);
}

static int usage(void) {
    fprintf(stderr, "usage: nexa-later [--min SECONDS] [--max SECONDS] 'command' ['command' ...]\n");
    return 2;
}

int main(int argc, char **argv) {
    double min = 2, max = 20;
    int i = 1;
    for (; i < argc && strncmp(argv[i], "--", 2) == 0; i++) {
        if (strcmp(argv[i], "--") == 0) { i++; break; }
        if (i + 1 >= argc) return usage();
        if (strcmp(argv[i], "--min") == 0) min = atof(argv[++i]);
        else if (strcmp(argv[i], "--max") == 0) max = atof(argv[++i]);
        else return usage();
    }
    if (i >= argc) return usage();
    if (max < min) max = min;
    signal(SIGCHLD, SIG_IGN);  /* the started services are never our zombies */

    double t0 = now();
    sleep_s(min);

    int psi = psi_some_total("cpu") >= 0 && psi_some_total("io") >= 0;
    long long cpu0 = psi_some_total("cpu"), io0 = psi_some_total("io");
    unsigned long long busy0 = 0, total0 = 0;
    stat_sample(&busy0, &total0);
    double last = now(), cpu_share = 0, io_share = 0, busy_share = 0;
    int quiet = 0, settled = 0;

    while (now() - t0 < max) {
        sleep_s(1);
        double t = now(), dt = t - last;
        last = t;
        if (psi) {
            long long cpu1 = psi_some_total("cpu"), io1 = psi_some_total("io");
            cpu_share = (cpu1 - cpu0) / (dt * 1e6);
            io_share = (io1 - io0) / (dt * 1e6);
            cpu0 = cpu1;
            io0 = io1;
            quiet = cpu_share < CPU_SETTLED && io_share < IO_SETTLED ? quiet + 1 : 0;
        } else {
            unsigned long long busy1, total1;
            if (stat_sample(&busy1, &total1) != 0 || total1 == total0) break;
            busy_share = (double)(busy1 - busy0) / (double)(total1 - total0);
            busy0 = busy1;
            total0 = total1;
            quiet = busy_share < STAT_SETTLED ? quiet + 1 : 0;
        }
        if (quiet >= QUIET_SAMPLES) { settled = 1; break; }
    }

    int n = argc - i;
    if (psi)
        fprintf(stderr, "nexa-later: %s after %.1f s (cpu %.0f%%, io %.0f%%); starting %d\n",
                settled ? "settled" : "still busy, starting anyway", now() - t0,
                cpu_share * 100, io_share * 100, n);
    else
        fprintf(stderr, "nexa-later: %s after %.1f s (busy %.0f%%, no PSI); starting %d\n",
                settled ? "settled" : "still busy, starting anyway", now() - t0, busy_share * 100, n);
    for (int k = i; k < argc; k++) {
        start(argv[k]);
        if (k + 1 < argc) sleep_s(0.2);
    }
    return 0;
}
