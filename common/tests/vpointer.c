/* A virtual mouse for the headless UI tests: holds one wlr virtual pointer
 * open for the whole test (a pointer that appears and vanishes per click
 * is never seen by the app) and reads commands from stdin:
 *
 *   move X Y          put the pointer at X,Y (layout pixels)
 *   click             press and release the left button
 *   press / release   the left button
 *
 * It prints "ok" after each command, once the compositor has it. Test-only:
 * built by scripts/test-titlebar.sh inside the test container, never
 * shipped. */
#define _GNU_SOURCE
#include <linux/input-event-codes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <wayland-client.h>
#include "wlr-virtual-pointer-unstable-v1-client-protocol.h"

static struct wl_seat *seat;
static struct zwlr_virtual_pointer_manager_v1 *manager;
static struct wl_output *output;
static int32_t out_w, out_h;

static void out_geometry(void *d, struct wl_output *o, int32_t x, int32_t y, int32_t pw, int32_t ph,
                         int32_t sub, const char *make, const char *model, int32_t transform) {
    (void)d; (void)o; (void)x; (void)y; (void)pw; (void)ph; (void)sub; (void)make; (void)model; (void)transform;
}
static void out_mode(void *d, struct wl_output *o, uint32_t flags, int32_t w, int32_t h, int32_t refresh) {
    (void)d; (void)o; (void)refresh;
    if (flags & WL_OUTPUT_MODE_CURRENT) { out_w = w; out_h = h; }
}
static void out_done(void *d, struct wl_output *o) { (void)d; (void)o; }
static void out_scale(void *d, struct wl_output *o, int32_t s) { (void)d; (void)o; (void)s; }
static const struct wl_output_listener out_listener = { out_geometry, out_mode, out_done, out_scale, NULL, NULL };

static void global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t version) {
    (void)d; (void)version;
    if (strcmp(iface, wl_seat_interface.name) == 0 && !seat)
        seat = wl_registry_bind(r, name, &wl_seat_interface, 1);
    else if (strcmp(iface, zwlr_virtual_pointer_manager_v1_interface.name) == 0)
        manager = wl_registry_bind(r, name, &zwlr_virtual_pointer_manager_v1_interface, 1);
    else if (strcmp(iface, wl_output_interface.name) == 0 && !output) {
        output = wl_registry_bind(r, name, &wl_output_interface, 2);
        wl_output_add_listener(output, &out_listener, NULL);
    }
}
static void global_remove(void *d, struct wl_registry *r, uint32_t name) { (void)d; (void)r; (void)name; }
static const struct wl_registry_listener reg_listener = { global, global_remove };

static uint32_t now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (uint32_t)(t.tv_sec * 1000 + t.tv_nsec / 1000000);
}

int main(void) {
    struct wl_display *dpy = wl_display_connect(NULL);
    if (!dpy) { fprintf(stderr, "no Wayland display\n"); return 1; }
    struct wl_registry *reg = wl_display_get_registry(dpy);
    wl_registry_add_listener(reg, &reg_listener, NULL);
    wl_display_roundtrip(dpy);
    wl_display_roundtrip(dpy);
    if (!manager || !seat || out_w <= 0) { fprintf(stderr, "no virtual pointer support or output\n"); return 1; }
    struct zwlr_virtual_pointer_v1 *ptr = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager, seat);
    wl_display_roundtrip(dpy);
    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("ready %dx%d\n", out_w, out_h);

    char line[128];
    while (fgets(line, sizeof line, stdin)) {
        int x, y;
        if (sscanf(line, "move %d %d", &x, &y) == 2) {
            if (x < 0) x = 0;
            if (y < 0) y = 0;
            if (x >= out_w) x = out_w - 1;
            if (y >= out_h) y = out_h - 1;
            zwlr_virtual_pointer_v1_motion_absolute(ptr, now_ms(), (uint32_t)x, (uint32_t)y, (uint32_t)out_w, (uint32_t)out_h);
            zwlr_virtual_pointer_v1_frame(ptr);
        } else if (strncmp(line, "click", 5) == 0 || strncmp(line, "press", 5) == 0 || strncmp(line, "release", 7) == 0) {
            if (line[0] != 'r') {
                zwlr_virtual_pointer_v1_button(ptr, now_ms(), BTN_LEFT, WL_POINTER_BUTTON_STATE_PRESSED);
                zwlr_virtual_pointer_v1_frame(ptr);
            }
            if (line[0] != 'p') {
                wl_display_roundtrip(dpy);
                zwlr_virtual_pointer_v1_button(ptr, now_ms(), BTN_LEFT, WL_POINTER_BUTTON_STATE_RELEASED);
                zwlr_virtual_pointer_v1_frame(ptr);
            }
        } else if (strncmp(line, "quit", 4) == 0) {
            break;
        } else {
            printf("unknown\n");
            continue;
        }
        wl_display_roundtrip(dpy);
        printf("ok\n");
    }
    zwlr_virtual_pointer_v1_destroy(ptr);
    wl_display_roundtrip(dpy);
    wl_display_disconnect(dpy);
    return 0;
}
