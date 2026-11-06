/* nexa-constellation: opens Constellation, the window overview, or closes
 * it again (Super+Tab). The sky itself lives in Orbit's process
 * (tools-c/nexa-constellation/sky.c); this tells this user's Orbit, with
 * SIGUSR2. Exit status 1 when Orbit isn't running. */
#include <dirent.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    if (argc > 1) {
        fprintf(stderr, "usage: nexa-constellation   (opens or closes the window overview)\n");
        return 2;
    }
    DIR *d = opendir("/proc");
    if (!d) return 1;
    struct dirent *e;
    int sent = 0;
    uid_t me = getuid();
    while ((e = readdir(d))) {
        char *end;
        long pid = strtol(e->d_name, &end, 10);
        if (*end || pid <= 0) continue;
        char path[64], comm[32] = "";
        struct stat st;
        snprintf(path, sizeof path, "/proc/%ld", pid);
        if (stat(path, &st) != 0 || st.st_uid != me) continue;  /* only your own Orbit */
        snprintf(path, sizeof path, "/proc/%ld/comm", pid);
        FILE *f = fopen(path, "r");
        if (!f) continue;
        if (fgets(comm, sizeof comm, f)) comm[strcspn(comm, "\n")] = 0;
        fclose(f);
        if (!strcmp(comm, "nexa-orbit") && kill((pid_t)pid, SIGUSR2) == 0) sent++;
    }
    closedir(d);
    if (!sent) {
        fprintf(stderr, "nexa-constellation: Orbit isn't running, and the window overview lives in it\n");
        return 1;
    }
    return 0;
}
