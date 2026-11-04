/* nexa-vpointer: a pretend mouse for tests (wlr virtual pointer). Headless
 * Sway has no pointer, so apps get no clicks; this gives the seat one.
 * Built into build/test/ only: never part of the system image.
 *
 *   nexa-vpointer W H [move X Y] [click] [rclick] [mclick] [press] [release] [drag X Y] [wait MS]...
 *
 * W H: the output size (positions are absolute, in that space). drag moves
 * in 12 steps with the button held.
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <linux/input-event-codes.h>
#include <wayland-client.h>
#include "virtual-pointer.h"

static struct wl_seat *seat;
static struct zwlr_virtual_pointer_manager_v1 *mgr;

static void global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver)
{
    if (!strcmp(iface, wl_seat_interface.name) && !seat)
        seat = wl_registry_bind(r, name, &wl_seat_interface, 1);
    else if (!strcmp(iface, zwlr_virtual_pointer_manager_v1_interface.name))
        mgr = wl_registry_bind(r, name, &zwlr_virtual_pointer_manager_v1_interface, 1);
}
static void global_remove(void *d, struct wl_registry *r, uint32_t name) {}
static const struct wl_registry_listener reg = {global, global_remove};

static uint32_t ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (uint32_t)(t.tv_sec * 1000 + t.tv_nsec / 1000000);
}

int main(int argc, char **argv)
{
    if (argc < 3) {
        fprintf(stderr, "usage: nexa-vpointer W H [move X Y] [click] [press] [release] [drag X Y] [wait MS]...\n");
        return 2;
    }
    uint32_t W = atoi(argv[1]), H = atoi(argv[2]);
    struct wl_display *dpy = wl_display_connect(NULL);
    if (!dpy) { fprintf(stderr, "nexa-vpointer: no Wayland display\n"); return 1; }
    struct wl_registry *r = wl_display_get_registry(dpy);
    wl_registry_add_listener(r, &reg, NULL);
    wl_display_roundtrip(dpy);
    if (!seat || !mgr) { fprintf(stderr, "nexa-vpointer: the compositor has no virtual pointer\n"); return 1; }
    struct zwlr_virtual_pointer_v1 *p = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(mgr, seat);
    wl_display_roundtrip(dpy);
    usleep(150000); /* the seat gains a pointer; let clients bind it */
    wl_display_roundtrip(dpy);
    double cx = 0, cy = 0;
    for (int i = 3; i < argc; i++) {
        if (!strcmp(argv[i], "move") && i + 2 < argc) {
            cx = atof(argv[i + 1]); cy = atof(argv[i + 2]); i += 2;
            zwlr_virtual_pointer_v1_motion_absolute(p, ms(), (uint32_t)cx, (uint32_t)cy, W, H);
            zwlr_virtual_pointer_v1_frame(p);
        } else if (!strcmp(argv[i], "press") || !strcmp(argv[i], "release") || !strcmp(argv[i], "click") ||
                   !strcmp(argv[i], "rclick") || !strcmp(argv[i], "mclick")) {
            int press = strcmp(argv[i], "release") != 0, click = strstr(argv[i], "click") != NULL;
            uint32_t btn = argv[i][0] == 'r' && click ? BTN_RIGHT : argv[i][0] == 'm' ? BTN_MIDDLE : BTN_LEFT;
            zwlr_virtual_pointer_v1_button(p, ms(), btn, press ? WL_POINTER_BUTTON_STATE_PRESSED : WL_POINTER_BUTTON_STATE_RELEASED);
            zwlr_virtual_pointer_v1_frame(p);
            if (click) {
                wl_display_roundtrip(dpy);
                usleep(40000);
                zwlr_virtual_pointer_v1_button(p, ms(), btn, WL_POINTER_BUTTON_STATE_RELEASED);
                zwlr_virtual_pointer_v1_frame(p);
            }
        } else if (!strcmp(argv[i], "drag") && i + 2 < argc) {
            double tx = atof(argv[i + 1]), ty = atof(argv[i + 2]);
            i += 2;
            for (int s = 1; s <= 12; s++) {
                double x = cx + (tx - cx) * s / 12, y = cy + (ty - cy) * s / 12;
                zwlr_virtual_pointer_v1_motion_absolute(p, ms(), (uint32_t)x, (uint32_t)y, W, H);
                zwlr_virtual_pointer_v1_frame(p);
                wl_display_roundtrip(dpy);
                usleep(16000);
            }
            cx = tx; cy = ty;
        } else if (!strcmp(argv[i], "wait") && i + 1 < argc) {
            usleep(atoi(argv[++i]) * 1000);
        } else {
            fprintf(stderr, "nexa-vpointer: what is %s?\n", argv[i]);
            return 2;
        }
        wl_display_roundtrip(dpy);
    }
    usleep(100000);
    wl_display_roundtrip(dpy);
    zwlr_virtual_pointer_v1_destroy(p);
    wl_display_roundtrip(dpy);
    wl_display_disconnect(dpy);
    return 0;
}
