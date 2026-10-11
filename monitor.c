#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <unistd.h>
#include <ctype.h>
#include <sys/sysinfo.h>

void clear_screen() {
    printf("\033[H\033[J");
}

void print_header() {
    printf("\033[1;36m=== Nexa OS Process Monitor ===\033[0m\n\n");
    
    struct sysinfo info;
    sysinfo(&info);
    
    long total_mem = info.totalram / 1024 / 1024;
    long free_mem = info.freeram / 1024 / 1024;
    long used_mem = total_mem - free_mem;
    
    printf("Uptime: %ld seconds\n", info.uptime);
    printf("Memory: %ld MB used / %ld MB total (Free: %ld MB)\n", used_mem, total_mem, free_mem);
    printf("Procs:  %d running\n\n", info.procs);
    
    printf("\033[1;33m%-6s %-15s %-6s\033[0m\n", "PID", "NAME", "STATE");
    printf("-------------------------------\n");
}

int is_numeric(const char *str) {
    while (*str) {
        if (!isdigit(*str)) return 0;
        str++;
    }
    return 1;
}

void print_processes() {
    DIR *dir = opendir("/proc");
    if (!dir) return;

    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL) {
        if (is_numeric(ent->d_name)) {
            char path[256];
            snprintf(path, sizeof(path), "/proc/%s/stat", ent->d_name);
            FILE *f = fopen(path, "r");
            if (f) {
                int pid;
                char comm[256];
                char state;
                if (fscanf(f, "%d (%[^)]) %c", &pid, comm, &state) == 3) {
                    printf("%-6d %-15s %-6c\n", pid, comm, state);
                }
                fclose(f);
            }
        }
    }
    closedir(dir);
}

int main() {
    while (1) {
        clear_screen();
        print_header();
        print_processes();
        printf("\n\033[1;30mPress Ctrl+C to exit...\033[0m\n");
        sleep(2);
    }
    return 0;
}
