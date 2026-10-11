#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/wait.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <sys/mount.h>
#include <fcntl.h>
#include <string.h>
#include <sys/sysmacros.h>

#define MAX_SERVICES 10

typedef struct {
    char id[16];
    char cmd[128];
    pid_t pid;
    int respawn;
} Service;

Service services[MAX_SERVICES];
int num_services = 0;

void parse_inittab() {
    FILE *f = fopen("/etc/inittab", "r");
    if (!f) return;
    char line[256];
    while (fgets(line, sizeof(line), f)) {
        if (line[0] == '#' || line[0] == '\n') continue;
        char *id = strtok(line, ":");
        char *runlevels = strtok(NULL, ":");
        char *action = strtok(NULL, ":");
        char *process = strtok(NULL, "\n");
        
        if (id && action && process && num_services < MAX_SERVICES) {
            strncpy(services[num_services].id, id, 15);
            strncpy(services[num_services].cmd, process, 127);
            services[num_services].pid = 0;
            services[num_services].respawn = (strcmp(action, "respawn") == 0);
            num_services++;
        }
    }
    fclose(f);
}

void spawn_service(int idx) {
    pid_t pid = fork();
    if (pid == 0) {
        setsid();
        /* Redirect stdio: try to open device from command, fallback to /dev/console */
        char *tty = "/dev/console";
        if (strstr(services[idx].cmd, "ttyS0")) tty = "/dev/ttyS0";
        else if (strstr(services[idx].cmd, "tty0")) tty = "/dev/tty0";
        int fd = open(tty, O_RDWR);
        if (fd >= 0) {
            dup2(fd, 0);
            dup2(fd, 1);
            dup2(fd, 2);
            if (fd > 2) close(fd);
        }
        char *args[] = { "/bin/sh", "-c", services[idx].cmd, NULL };
        execv("/bin/sh", args);
        exit(1);
    } else if (pid > 0) {
        services[idx].pid = pid;
    }
}

int main() {
    if (getpid() != 1) {
        printf("init must be run as PID 1\n");
        return 1;
    }

    // Early mounts
    mount("proc", "/proc", "proc", 0, NULL);
    mount("sysfs", "/sys", "sysfs", 0, NULL);
    mount("devtmpfs", "/dev", "devtmpfs", 0, NULL);

    printf("NexaOS C-Based Init System Starting...\n");

    // Set up /dev/null if missing
    mknod("/dev/null", S_IFCHR | 0666, makedev(1, 3));
    mknod("/dev/tty", S_IFCHR | 0666, makedev(5, 0));
    mknod("/dev/ttyS0", S_IFCHR | 0660, makedev(4, 64));
    mknod("/dev/tty0", S_IFCHR | 0660, makedev(4, 0));

    // Apply kernel sysctls as PID 1 (root) — guaranteed before any user process
    {
        int fd;
        // Allow unprivileged ping for all groups (SOCK_DGRAM ICMP)
        fd = open("/proc/sys/net/ipv4/ping_group_range", O_WRONLY);
        if (fd >= 0) { write(fd, "0\t2147483647\n", 13); close(fd); }
        // Disable IPv4 reverse-path filter to allow QEMU routing
        fd = open("/proc/sys/net/ipv4/conf/all/rp_filter", O_WRONLY);
        if (fd >= 0) { write(fd, "0\n", 2); close(fd); }
    }

    // Always run rcS sysinit
    pid_t sysinit = fork();
    if (sysinit == 0) {
        setsid();
        execl("/bin/sh", "sh", "/etc/init.d/rcS", NULL);
        exit(0);
    }
    if (sysinit > 0) waitpid(sysinit, NULL, 0);

    parse_inittab();

    for (int i = 0; i < num_services; i++) {
        spawn_service(i);
    }

    // Wait loop
    while (1) {
        int status;
        pid_t p = wait(&status);
        if (p > 0) {
            for (int i = 0; i < num_services; i++) {
                if (services[i].pid == p) {
                    services[i].pid = 0;
                    if (services[i].respawn) {
                        spawn_service(i);
                    }
                    break;
                }
            }
        } else {
            sleep(1); // Prevent CPU spin if wait fails
        }
    }

    return 0;
}
