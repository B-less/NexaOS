/* nexa-barhide: the top bar's "hide when not in use" (Settings >
 * Personalization > Top bar).
 *
 * The bar (waybar) starts hidden and takes no space. This program keeps an
 * invisible strip on the screen, through the layer-shell protocol:
 *
 *   hidden: 2 px along the top edge. The pointer resting there for a moment
 *           shows the bar (SIGUSR1 to waybar toggles it).
 *   shown:  everything below the bar. The first move of the pointer back
 *           down to the windows lands on it (however fast: a thin strip
 *           could be jumped over) and hides the bar again; the strip is
 *           then back at the top edge, so the next move reaches the window.
 *
 * The strip is on the "top" layer, under fullscreen windows, so a film or
 * a game never brings the bar up. If waybar restarts (it starts hidden),
 * this notices the new process and starts over as hidden.
 *
 *   nexa-barhide [BAR_HEIGHT]     (default 38, the bar's height in px)
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/timerfd.h>
#include <unistd.h>

#include <wayland-client.h>
#include "layer-shell.h"

#define EDGE_H 2
#define DWELL_MS 150

enum { HIDDEN, SHOWN };

static struct {
    struct wl_display *display;
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    struct wl_seat *seat;
    struct wl_pointer *pointer;
    struct zwlr_layer_shell_v1 *layer_shell;
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer;
    struct wl_buffer *buffer;
    int width, height, bar_h, mode, inside;
    int dwell_fd;
    pid_t bar_pid;
} S = {.dwell_fd = -1, .bar_h = 38};

/* This user's waybar (the newest, if more than one). */
static pid_t find_bar(void)
{
    DIR *d = opendir("/proc");
    if (!d) return 0;
    pid_t best = 0;
    uid_t me = getuid();
    struct dirent *e;
    while ((e = readdir(d))) {
        char *end;
        long pid = strtol(e->d_name, &end, 10);
        if (*end || pid <= 0) continue;
        char path[64], comm[32] = "";
        struct stat st;
        snprintf(path, sizeof path, "/proc/%ld", pid);
        if (stat(path, &st) != 0 || st.st_uid != me) continue;
        snprintf(path, sizeof path, "/proc/%ld/comm", pid);
        FILE *f = fopen(path, "r");
        if (!f) continue;
        if (fgets(comm, sizeof comm, f)) comm[strcspn(comm, "\n")] = 0;
        fclose(f);
        if (!strcmp(comm, "waybar") && pid > best) best = (pid_t)pid;
    }
    closedir(d);
    return best;
}

static void make_buffer(int w, int h)
{
    if (S.buffer) wl_buffer_destroy(S.buffer);
    S.buffer = NULL;
    if (w <= 0 || h <= 0) return;
    int stride = w * 4;
    size_t size = (size_t)stride * h;
    int fd = memfd_create("nexa-barhide", MFD_CLOEXEC);
    if (fd < 0) return;
    /* A fresh memfd is all zeros: fully transparent ARGB. */
    if (ftruncate(fd, (off_t)size) == 0) {
        struct wl_shm_pool *pool = wl_shm_create_pool(S.shm, fd, (int32_t)size);
        S.buffer = wl_shm_pool_create_buffer(pool, 0, w, h, stride, WL_SHM_FORMAT_ARGB8888);
        wl_shm_pool_destroy(pool);
    }
    close(fd);
}

/* Where the strip goes for the current mode. */
static void place(void)
{
    uint32_t edges = ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP | ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT | ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT;
    if (S.mode == SHOWN) edges |= ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM;
    zwlr_layer_surface_v1_set_anchor(S.layer, edges);
    zwlr_layer_surface_v1_set_size(S.layer, 0, S.mode == HIDDEN ? EDGE_H : 0);
    zwlr_layer_surface_v1_set_margin(S.layer, S.mode == HIDDEN ? 0 : S.bar_h, 0, 0, 0);
    wl_surface_commit(S.surface);
}

static void toggle_bar(void)
{
    if (S.bar_pid > 0) kill(S.bar_pid, SIGUSR1);
}

/* A restarted waybar starts hidden again: follow it. */
static void follow_bar(void)
{
    pid_t now = find_bar();
    if (now != S.bar_pid) {
        S.bar_pid = now;
        if (S.mode != HIDDEN) {
            S.mode = HIDDEN;
            place();
        }
    }
}

static void arm_dwell(int on)
{
    struct itimerspec its = {0};
    if (on) its.it_value.tv_nsec = DWELL_MS * 1000000L;
    timerfd_settime(S.dwell_fd, 0, &its, NULL);
}

static void layer_configure(void *d, struct zwlr_layer_surface_v1 *l, uint32_t serial, uint32_t w, uint32_t h)
{
    zwlr_layer_surface_v1_ack_configure(l, serial);
    if ((int)w != S.width || (int)h != S.height || !S.buffer) {
        S.width = (int)w;
        S.height = (int)h;
        make_buffer(S.width, S.height);
    }
    wl_surface_attach(S.surface, S.buffer, 0, 0);
    wl_surface_damage_buffer(S.surface, 0, 0, S.width, S.height);
    wl_surface_commit(S.surface);
}

static void layer_closed(void *d, struct zwlr_layer_surface_v1 *l)
{
    exit(0);
}

static const struct zwlr_layer_surface_v1_listener layer_listener = {
    .configure = layer_configure, .closed = layer_closed};

static void hide_bar(void)
{
    toggle_bar();
    S.mode = HIDDEN;
    place();
}

static void ptr_enter(void *d, struct wl_pointer *p, uint32_t serial, struct wl_surface *s, wl_fixed_t x, wl_fixed_t y)
{
    if (s != S.surface) return;
    S.inside = 1;
    follow_bar();
    if (S.mode == HIDDEN)
        arm_dwell(1); /* a moment at the edge, not a pass over it */
    else
        hide_bar(); /* back down past the bar */
}

static void ptr_leave(void *d, struct wl_pointer *p, uint32_t serial, struct wl_surface *s)
{
    S.inside = 0;
    if (S.mode == HIDDEN) arm_dwell(0);
}

/* The strip grows into the area under the bar while the pointer is still
 * "on" it, so moving down brings only motion, no new enter: any motion
 * here while the bar is shown means the pointer is below it. */
static void ptr_motion(void *d, struct wl_pointer *p, uint32_t t, wl_fixed_t x, wl_fixed_t y)
{
    if (S.inside && S.mode == SHOWN && wl_fixed_to_double(y) >= 0)
        hide_bar();
}
static void ptr_button(void *d, struct wl_pointer *p, uint32_t s, uint32_t t, uint32_t b, uint32_t st) {}
static void ptr_axis(void *d, struct wl_pointer *p, uint32_t t, uint32_t a, wl_fixed_t v) {}
static void ptr_frame(void *d, struct wl_pointer *p) {}
static void ptr_axis_source(void *d, struct wl_pointer *p, uint32_t s) {}
static void ptr_axis_stop(void *d, struct wl_pointer *p, uint32_t t, uint32_t a) {}
static void ptr_axis_discrete(void *d, struct wl_pointer *p, uint32_t a, int32_t v) {}

static const struct wl_pointer_listener pointer_listener = {
    .enter = ptr_enter, .leave = ptr_leave, .motion = ptr_motion, .button = ptr_button, .axis = ptr_axis,
    .frame = ptr_frame, .axis_source = ptr_axis_source, .axis_stop = ptr_axis_stop, .axis_discrete = ptr_axis_discrete};

static void seat_caps(void *d, struct wl_seat *seat, uint32_t caps)
{
    if ((caps & WL_SEAT_CAPABILITY_POINTER) && !S.pointer) {
        S.pointer = wl_seat_get_pointer(seat);
        wl_pointer_add_listener(S.pointer, &pointer_listener, NULL);
    } else if (!(caps & WL_SEAT_CAPABILITY_POINTER) && S.pointer) {
        wl_pointer_release(S.pointer);
        S.pointer = NULL;
    }
}
static void seat_name(void *d, struct wl_seat *s, const char *n) {}
static const struct wl_seat_listener seat_listener = {.capabilities = seat_caps, .name = seat_name};

static void reg_global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver)
{
    if (!strcmp(iface, wl_compositor_interface.name))
        S.compositor = wl_registry_bind(r, name, &wl_compositor_interface, ver < 4 ? ver : 4);
    else if (!strcmp(iface, wl_shm_interface.name))
        S.shm = wl_registry_bind(r, name, &wl_shm_interface, 1);
    else if (!strcmp(iface, wl_seat_interface.name) && !S.seat) {
        S.seat = wl_registry_bind(r, name, &wl_seat_interface, ver < 5 ? ver : 5);
        wl_seat_add_listener(S.seat, &seat_listener, NULL);
    } else if (!strcmp(iface, zwlr_layer_shell_v1_interface.name))
        S.layer_shell = wl_registry_bind(r, name, &zwlr_layer_shell_v1_interface, ver < 3 ? ver : 3);
}
static void reg_remove(void *d, struct wl_registry *r, uint32_t name) {}
static const struct wl_registry_listener registry_listener = {.global = reg_global, .global_remove = reg_remove};

int main(int argc, char **argv)
{
    if (argc > 1) {
        int h = atoi(argv[1]);
        if (h > 0 && h < 400) S.bar_h = h;
    }
    S.display = wl_display_connect(NULL);
    if (!S.display) {
        fprintf(stderr, "nexa-barhide: no Wayland display\n");
        return 1;
    }
    struct wl_registry *reg = wl_display_get_registry(S.display);
    wl_registry_add_listener(reg, &registry_listener, NULL);
    wl_display_roundtrip(S.display);
    if (!S.compositor || !S.shm || !S.layer_shell) {
        fprintf(stderr, "nexa-barhide: the compositor has no layer-shell\n");
        return 1;
    }
    S.dwell_fd = timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC | TFD_NONBLOCK);
    S.bar_pid = find_bar();
    S.surface = wl_compositor_create_surface(S.compositor);
    /* Not "nexa-*": Hyprland blurs those layers, and this one must stay invisible. */
    S.layer = zwlr_layer_shell_v1_get_layer_surface(S.layer_shell, S.surface, NULL,
                                                    ZWLR_LAYER_SHELL_V1_LAYER_TOP, "barhide-edge");
    zwlr_layer_surface_v1_add_listener(S.layer, &layer_listener, NULL);
    zwlr_layer_surface_v1_set_exclusive_zone(S.layer, -1); /* the very edge, whatever else reserves space */
    zwlr_layer_surface_v1_set_keyboard_interactivity(S.layer, ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE);
    S.mode = HIDDEN;
    place();

    for (;;) {
        while (wl_display_prepare_read(S.display) != 0) wl_display_dispatch_pending(S.display);
        wl_display_flush(S.display);
        struct pollfd fds[2] = {{.fd = wl_display_get_fd(S.display), .events = POLLIN}, {.fd = S.dwell_fd, .events = POLLIN}};
        int n = poll(fds, 2, 2000);
        if (n < 0 && errno != EINTR) {
            wl_display_cancel_read(S.display);
            return 1;
        }
        if (n > 0 && (fds[0].revents & POLLIN)) {
            if (wl_display_read_events(S.display) < 0) return 1;
        } else {
            wl_display_cancel_read(S.display);
        }
        if (n > 0 && (fds[0].revents & (POLLERR | POLLHUP))) return 1;
        if (wl_display_dispatch_pending(S.display) < 0) return 1;
        if (n > 0 && (fds[1].revents & POLLIN)) {
            uint64_t t;
            if (read(S.dwell_fd, &t, sizeof t) < 0) {}
            follow_bar();
            if (S.inside && S.mode == HIDDEN && S.bar_pid > 0) {
                toggle_bar();
                S.mode = SHOWN;
                place();
            }
        }
        if (n == 0) follow_bar(); /* every 2 s: has waybar restarted? */
    }
}
