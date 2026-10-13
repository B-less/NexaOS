/* vpointer: drive a virtual mouse on a wlroots compositor (Sway, Hyprland)
 * for GUI tests, e.g. dragging a window by its title bar.
 *
 *   vpointer W H abs X Y press move DX DY ... release sleep MS
 *
 * W H is the output size (for absolute moves). Built and used by
 * scripts/test-app-headless.sh's DRAG helper; needs wayland-dev and
 * wlr-protocols (wayland-scanner generates the protocol code).
 */
#include <linux/input-event-codes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "wlr-virtual-pointer-unstable-v1-client-protocol.h"

static struct wl_seat *seat;
static struct zwlr_virtual_pointer_manager_v1 *manager;

static void global(void *data, struct wl_registry *reg, uint32_t name, const char *iface, uint32_t ver) {
    if (!strcmp(iface, wl_seat_interface.name))
        seat = wl_registry_bind(reg, name, &wl_seat_interface, 1);
    else if (!strcmp(iface, zwlr_virtual_pointer_manager_v1_interface.name))
        manager = wl_registry_bind(reg, name, &zwlr_virtual_pointer_manager_v1_interface, 1);
}
static void global_remove(void *data, struct wl_registry *reg, uint32_t name) {}
static const struct wl_registry_listener listener = {global, global_remove};

static uint32_t now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: vpointer W H [abs X Y|move DX DY|press|release|sleep MS]...\n");
        return 2;
    }
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) { fprintf(stderr, "vpointer: no Wayland display\n"); return 1; }
    struct wl_registry *reg = wl_display_get_registry(display);
    wl_registry_add_listener(reg, &listener, NULL);
    wl_display_roundtrip(display);
    if (!manager) { fprintf(stderr, "vpointer: compositor has no virtual pointer support\n"); return 1; }
    struct zwlr_virtual_pointer_v1 *ptr = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager, seat);
    wl_display_roundtrip(display);
    usleep(200000);  /* let clients see the new pointer capability */

    uint32_t w = atoi(argv[1]), h = atoi(argv[2]);
    for (int i = 3; i < argc; i++) {
        if (!strcmp(argv[i], "abs") && i + 2 < argc) {
            zwlr_virtual_pointer_v1_motion_absolute(ptr, now_ms(), atoi(argv[i + 1]), atoi(argv[i + 2]), w, h);
            i += 2;
        } else if (!strcmp(argv[i], "move") && i + 2 < argc) {
            zwlr_virtual_pointer_v1_motion(ptr, now_ms(), wl_fixed_from_int(atoi(argv[i + 1])),
                                           wl_fixed_from_int(atoi(argv[i + 2])));
            i += 2;
        } else if (!strcmp(argv[i], "press") || !strcmp(argv[i], "release")) {
            zwlr_virtual_pointer_v1_button(ptr, now_ms(), BTN_LEFT,
                                           !strcmp(argv[i], "press") ? WL_POINTER_BUTTON_STATE_PRESSED
                                                                     : WL_POINTER_BUTTON_STATE_RELEASED);
        } else if (!strcmp(argv[i], "sleep") && i + 1 < argc) {
            wl_display_flush(display);
            usleep(atoi(argv[++i]) * 1000);
            continue;
        } else {
            fprintf(stderr, "vpointer: bad argument %s\n", argv[i]);
            return 2;
        }
        zwlr_virtual_pointer_v1_frame(ptr);
        wl_display_flush(display);
        usleep(30000);
    }
    wl_display_roundtrip(display);
    zwlr_virtual_pointer_v1_destroy(ptr);
    wl_display_roundtrip(display);
    wl_display_disconnect(display);
    return 0;
}
