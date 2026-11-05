/* nexa-strata: move through Strata's five layers (workspaces 1 to 5).
 * Design: TODOnexa_identity.md, section "Strata". Bound by both sessions
 * to Super+scroll and Super+Page Up/Down (with Shift: carry). The layer
 * stack and the tint follow by themselves (Orbit and Living watch the
 * compositor).
 *
 *   nexa-strata dive +1|-1    one layer deeper / back up (stops at 1 and 5)
 *   nexa-strata carry +1|-1   the same, taking the window in front along
 *   nexa-strata current       print the layer on screen (0: not a layer)
 *
 * No GTK: it starts in about a millisecond, so each scroll step can run it. */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "layers.h"
#include "wm.h"

static long long now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

/* A touchpad sends a stream of scroll steps: one move per 150 ms. 1 if
 * this move is too soon after the last one. */
static int too_soon(void)
{
    const char *run = getenv("XDG_RUNTIME_DIR");
    if (!run || !*run) return 0;
    char path[512];
    snprintf(path, sizeof path, "%s/nexa-strata.last", run);
    long long t = now_ms(), last = 0;
    int fd = open(path, O_RDWR | O_CREAT | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (fd < 0) return 0;
    char buf[32] = {0};
    if (read(fd, buf, sizeof buf - 1) > 0) last = atoll(buf);
    int soon = last > 0 && t - last >= 0 && t - last < STRATA_GAP_MS;
    if (!soon) {
        int n = snprintf(buf, sizeof buf, "%lld\n", t);
        if (lseek(fd, 0, SEEK_SET) == 0 && ftruncate(fd, 0) == 0 && write(fd, buf, (size_t)n) < 0) {}
    }
    close(fd);
    return soon;
}

static int usage(void)
{
    fprintf(stderr, "usage: nexa-strata dive +1|-1 | carry +1|-1 | current\n");
    return 2;
}

int main(int argc, char **argv)
{
    if (wm_kind() == WM_NONE) {
        fprintf(stderr, "nexa-strata: needs Sway or Hyprland\n");
        return 1;
    }
    if (argc == 2 && !strcmp(argv[1], "current")) {
        int cur = wm_workspace();
        if (cur < 0) {
            fprintf(stderr, "nexa-strata: the compositor didn't answer\n");
            return 1;
        }
        printf("%d\n", cur >= 1 && cur <= STRATA_LAYERS ? cur : 0);
        return 0;
    }
    if (argc != 3 || (strcmp(argv[1], "dive") && strcmp(argv[1], "carry"))) return usage();
    int step = strata_step(argv[2]);
    if (!step) return usage();
    int cur = wm_workspace();
    if (cur < 0) {
        fprintf(stderr, "nexa-strata: the compositor didn't answer\n");
        return 1;
    }
    int to = strata_target(cur, step);
    if (to == cur || too_soon()) return 0;
    int r = !strcmp(argv[1], "carry") ? wm_workspace_carry(to) : wm_workspace_go(to);
    if (r < 0) {
        fprintf(stderr, "nexa-strata: couldn't reach layer %d\n", to);
        return 1;
    }
    return 0;
}
