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
 * The bar can't move itself, so the motion is drawn here, on a "ghost"
 * surface above it, from a snapshot of that strip of the screen
 * (wlr-screencopy):
 *   hiding:  a picture of the bar slides up into the top edge and fades,
 *            a line of the theme's accent running along its lower edge;
 *   showing: a picture of what's under the bar covers it as it appears,
 *            then wipes away downwards behind a bright scan line.
 * The ghost takes no input. Without screencopy, or with "motion": "none"
 * in Interface DNA, the bar just appears and disappears.
 *
 *   nexa-barhide [BAR_HEIGHT]     (default 38, the bar's height in px)
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/timerfd.h>
#include <time.h>
#include <unistd.h>

#include <wayland-client.h>
#include "layer-shell.h"
#include "screencopy.h"

#define EDGE_H 2
#define DWELL_MS 150
#define HIDE_MS 210
#define SHOW_MS 240
#define SETTLE_MS 70   /* the bar's first frame, under the ghost, before the wipe */
#define MAX_OUTPUTS 8

enum { HIDDEN, SHOWN };
enum { A_IDLE, A_CAPTURE, A_MAPPING, A_SETTLE, A_RUN };

struct shmbuf {
    struct wl_buffer *wl;
    uint32_t *px;
    size_t size;
    int w, h, busy;
};

static struct {
    struct wl_display *display;
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    struct wl_seat *seat;
    struct wl_pointer *pointer;
    struct zwlr_layer_shell_v1 *layer_shell;
    struct zwlr_screencopy_manager_v1 *screencopy;
    uint32_t screencopy_version;
    struct wl_output *outputs[MAX_OUTPUTS];
    int noutputs;
    struct wl_output *here;   /* the output the strip (and pointer) is on */
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer;
    struct wl_buffer *buffer;
    int width, height, bar_h, mode, inside;
    int dwell_fd, anim_fd;
    pid_t bar_pid;
    /* the animation */
    int anim, showing, toggle_pending;
    double t0;
    struct zwlr_screencopy_frame_v1 *frame;
    struct shmbuf cap;
    uint32_t cap_format, cap_stride, cap_flags;
    uint32_t *snap;           /* the captured strip, ARGB, top row first */
    int snap_w, snap_h;
    struct wl_surface *ghost;
    struct zwlr_layer_surface_v1 *ghost_layer;
    struct wl_callback *ghost_cb;
    struct shmbuf gbuf[2], *gbuf_cur;
    int gw, gh, gscale, configured;
    uint32_t accent;
} S = {.dwell_fd = -1, .anim_fd = -1, .bar_h = 38, .accent = 0xff4da3ff};

static double now_ms(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1e3 + ts.tv_nsec / 1e6;
}

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

static void config_path(char *out, size_t n, const char *rel)
{
    const char *cfg = getenv("XDG_CONFIG_HOME"), *home = getenv("HOME");
    if (cfg && *cfg) snprintf(out, n, "%s/%s", cfg, rel);
    else snprintf(out, n, "%s/.config/%s", home ? home : "/", rel);
}

/* The theme's accent (nexa-theme writes it for the lock screen too). */
static void load_accent(void)
{
    char path[512], line[256];
    config_path(path, sizeof path, "nexa/lock.conf");
    FILE *f = fopen(path, "r");
    if (!f) return;
    while (fgets(line, sizeof line, f)) {
        unsigned v;
        if (sscanf(line, "accent=#%6x", &v) == 1) S.accent = 0xff000000u | v;
    }
    fclose(f);
}

/* Interface DNA's "motion": "none": no animation. */
static int reduce_motion(void)
{
    char path[512], buf[4096];
    config_path(path, sizeof path, "nexa/interface/global.json");
    FILE *f = fopen(path, "r");
    if (!f) return 0;
    size_t n = fread(buf, 1, sizeof buf - 1, f);
    fclose(f);
    buf[n] = 0;
    char *m = strstr(buf, "\"motion\"");
    if (!m) return 0;
    m = strchr(m + 8, '"');
    return m && !strncmp(m, "\"none\"", 6);
}

static int shm_alloc(struct shmbuf *b, int w, int h, int stride, uint32_t format)
{
    size_t size = (size_t)stride * h;
    int fd = memfd_create("nexa-barhide", MFD_CLOEXEC);
    if (fd < 0) return -1;
    void *px = MAP_FAILED;
    if (ftruncate(fd, (off_t)size) == 0) px = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (px == MAP_FAILED) {
        close(fd);
        return -1;
    }
    struct wl_shm_pool *pool = wl_shm_create_pool(S.shm, fd, (int32_t)size);
    b->wl = wl_shm_pool_create_buffer(pool, 0, w, h, stride, format);
    wl_shm_pool_destroy(pool);
    close(fd);
    b->px = px;
    b->size = size;
    b->w = w;
    b->h = h;
    b->busy = 0;
    return 0;
}

static void shm_free(struct shmbuf *b)
{
    if (b->wl) wl_buffer_destroy(b->wl);
    if (b->px) munmap(b->px, b->size);
    memset(b, 0, sizeof *b);
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

/* ---------- the ghost: the animation's surface ---------- */

static void arm_anim_timer(int ms)
{
    struct itimerspec its = {0};
    its.it_value.tv_nsec = (long)ms * 1000000L;
    timerfd_settime(S.anim_fd, 0, &its, NULL);
}

/* Ends the animation now, the bar left as it should be. */
static void finish(void)
{
    if (S.toggle_pending) {
        toggle_bar();
        S.toggle_pending = 0;
    }
    if (S.frame) {
        zwlr_screencopy_frame_v1_destroy(S.frame);
        S.frame = NULL;
    }
    shm_free(&S.cap);
    if (S.ghost_cb) wl_callback_destroy(S.ghost_cb);
    S.ghost_cb = NULL;
    if (S.ghost_layer) zwlr_layer_surface_v1_destroy(S.ghost_layer);
    if (S.ghost) wl_surface_destroy(S.ghost);
    S.ghost_layer = NULL;
    S.ghost = NULL;
    for (int i = 0; i < 2; i++) shm_free(&S.gbuf[i]);
    S.configured = 0;
    S.anim = A_IDLE;
    arm_anim_timer(0);
}

static void gbuf_release(void *data, struct wl_buffer *b) { ((struct shmbuf *)data)->busy = 0; }
static const struct wl_buffer_listener gbuf_listener = {.release = gbuf_release};

static uint32_t blend(uint32_t under, uint32_t over, double a)
{
    /* both premultiplied ARGB; over is opaque, scaled by a */
    uint32_t out = 0;
    for (int sh = 0; sh < 32; sh += 8) {
        double u = (under >> sh) & 0xff, o = (over >> sh) & 0xff;
        uint32_t v = (uint32_t)(o * a + u * (1 - a) + 0.5);
        out |= (v > 255 ? 255 : v) << sh;
    }
    return out;
}

static uint32_t scale_px(uint32_t p, double a)
{
    uint32_t out = 0;
    for (int sh = 0; sh < 32; sh += 8) out |= (uint32_t)(((p >> sh) & 0xff) * a + 0.5) << sh;
    return out;
}

/* One frame of the ghost at progress k (0..1). */
static void draw_ghost(double k);

static void ghost_frame_done(void *data, struct wl_callback *cb, uint32_t t);
static const struct wl_callback_listener ghost_frame_listener = {.done = ghost_frame_done};

static void commit_ghost(double k)
{
    struct shmbuf *b = !S.gbuf[0].busy ? &S.gbuf[0] : !S.gbuf[1].busy ? &S.gbuf[1] : NULL;
    if (b) {
        S.gbuf_cur = b;
        draw_ghost(k);
        wl_surface_attach(S.ghost, b->wl, 0, 0);
        wl_surface_damage_buffer(S.ghost, 0, 0, b->w, b->h);
        b->busy = 1;
    }
    if (S.ghost_cb) wl_callback_destroy(S.ghost_cb);
    S.ghost_cb = wl_surface_frame(S.ghost);
    wl_callback_add_listener(S.ghost_cb, &ghost_frame_listener, NULL);
    wl_surface_commit(S.ghost);
}

static double ease(double x)
{
    if (x < 0) x = 0;
    if (x > 1) x = 1;
    return 1 - (1 - x) * (1 - x) * (1 - x); /* ease-out cubic */
}

static void ghost_frame_done(void *data, struct wl_callback *cb, uint32_t t)
{
    wl_callback_destroy(cb);
    S.ghost_cb = NULL;
    double now = now_ms();
    if (S.anim == A_MAPPING) {
        /* The ghost is on screen, matching what's there: now the bar can
         * change under it. */
        toggle_bar();
        S.toggle_pending = 0;
        S.t0 = now;
        S.anim = S.showing ? A_SETTLE : A_RUN;
    }
    if (S.anim == A_SETTLE && now - S.t0 >= SETTLE_MS) {
        S.anim = A_RUN;
        S.t0 = now;
    }
    double k = 0;
    if (S.anim == A_RUN) {
        k = (now - S.t0) / (S.showing ? SHOW_MS : HIDE_MS);
        if (k >= 1) {
            finish();
            return;
        }
    }
    commit_ghost(k);
}

static void ghost_configure(void *d, struct zwlr_layer_surface_v1 *l, uint32_t serial, uint32_t w, uint32_t h)
{
    zwlr_layer_surface_v1_ack_configure(l, serial);
    if (S.configured) return;
    S.configured = 1;
    /* The snapshot is in buffer pixels; the surface in logical ones. */
    S.gscale = w > 0 && S.snap_w >= (int)w ? S.snap_w / (int)w : 1;
    if (S.gscale < 1) S.gscale = 1;
    S.gw = S.snap_w;
    S.gh = S.snap_h;
    for (int i = 0; i < 2; i++) {
        if (shm_alloc(&S.gbuf[i], S.gw, S.gh, S.gw * 4, WL_SHM_FORMAT_ARGB8888) < 0) {
            finish();
            return;
        }
        wl_buffer_add_listener(S.gbuf[i].wl, &gbuf_listener, &S.gbuf[i]);
    }
    wl_surface_set_buffer_scale(S.ghost, S.gscale);
    commit_ghost(0);
}

static void ghost_closed(void *d, struct zwlr_layer_surface_v1 *l) { finish(); }
static const struct zwlr_layer_surface_v1_listener ghost_listener = {.configure = ghost_configure, .closed = ghost_closed};

static void map_ghost(void)
{
    S.ghost = wl_compositor_create_surface(S.compositor);
    /* No input: clicks and the pointer go through to whatever is below. */
    struct wl_region *none = wl_compositor_create_region(S.compositor);
    wl_surface_set_input_region(S.ghost, none);
    wl_region_destroy(none);
    S.ghost_layer = zwlr_layer_shell_v1_get_layer_surface(S.layer_shell, S.ghost, S.here,
                                                          ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY, "barhide-ghost");
    zwlr_layer_surface_v1_add_listener(S.ghost_layer, &ghost_listener, NULL);
    zwlr_layer_surface_v1_set_anchor(S.ghost_layer, ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP | ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT |
                                                        ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
    zwlr_layer_surface_v1_set_size(S.ghost_layer, 0, (uint32_t)S.bar_h);
    zwlr_layer_surface_v1_set_exclusive_zone(S.ghost_layer, -1);
    zwlr_layer_surface_v1_set_keyboard_interactivity(S.ghost_layer, ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE);
    wl_surface_commit(S.ghost);
    S.anim = A_MAPPING;
}

/* ---------- the snapshot ---------- */

static void cap_buffer(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t format, uint32_t w, uint32_t h, uint32_t stride)
{
    if (S.cap.wl || (format != WL_SHM_FORMAT_ARGB8888 && format != WL_SHM_FORMAT_XRGB8888 &&
                     format != WL_SHM_FORMAT_ABGR8888 && format != WL_SHM_FORMAT_XBGR8888)) return;
    if (shm_alloc(&S.cap, (int)w, (int)h, (int)stride, format) < 0) return;
    S.cap_format = format;
    S.cap_stride = stride;
    if (S.screencopy_version < 3) zwlr_screencopy_frame_v1_copy(f, S.cap.wl);
}
static void cap_flags(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t flags) { S.cap_flags = flags; }
static void cap_ready(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t hi, uint32_t lo, uint32_t ns)
{
    int w = S.cap.w, h = S.cap.h;
    uint32_t *img = malloc((size_t)w * h * 4);
    if (img) {
        int swap = S.cap_format == WL_SHM_FORMAT_ABGR8888 || S.cap_format == WL_SHM_FORMAT_XBGR8888;
        for (int y = 0; y < h; y++) {
            int sy = (S.cap_flags & ZWLR_SCREENCOPY_FRAME_V1_FLAGS_Y_INVERT) ? h - 1 - y : y;
            const uint32_t *sp = (const uint32_t *)((const unsigned char *)S.cap.px + (size_t)sy * S.cap_stride);
            for (int x = 0; x < w; x++) {
                uint32_t v = sp[x];
                if (swap) v = (v & 0xff00ff00u) | ((v & 0xffu) << 16) | ((v >> 16) & 0xffu);
                img[(size_t)y * w + x] = v | 0xff000000u;
            }
        }
        free(S.snap);
        S.snap = img;
        S.snap_w = w;
        S.snap_h = h;
    }
    zwlr_screencopy_frame_v1_destroy(f);
    S.frame = NULL;
    shm_free(&S.cap);
    if (img) map_ghost();
    else finish();
}
static void cap_failed(void *d, struct zwlr_screencopy_frame_v1 *f)
{
    zwlr_screencopy_frame_v1_destroy(f);
    S.frame = NULL;
    finish();
}
static void cap_damage(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t x, uint32_t y, uint32_t w, uint32_t h) {}
static void cap_dmabuf(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t fmt, uint32_t w, uint32_t h) {}
static void cap_buffer_done(void *d, struct zwlr_screencopy_frame_v1 *f)
{
    if (S.cap.wl) zwlr_screencopy_frame_v1_copy(f, S.cap.wl);
    else cap_failed(d, f);
}
static const struct zwlr_screencopy_frame_v1_listener cap_listener = {
    .buffer = cap_buffer, .flags = cap_flags, .ready = cap_ready, .failed = cap_failed,
    .damage = cap_damage, .linux_dmabuf = cap_dmabuf, .buffer_done = cap_buffer_done};

/* Show or hide the bar, animated when we can. */
static void change_bar(int show)
{
    if (S.anim != A_IDLE) finish();
    if (!S.screencopy || !S.here || S.width <= 0 || S.bar_pid <= 0 || reduce_motion()) {
        toggle_bar();
        return;
    }
    load_accent();
    S.showing = show;
    S.toggle_pending = 1; /* sent once the ghost covers the bar */
    S.frame = zwlr_screencopy_manager_v1_capture_output_region(S.screencopy, 0, S.here, 0, 0, S.width, S.bar_h);
    zwlr_screencopy_frame_v1_add_listener(S.frame, &cap_listener, NULL);
    S.anim = A_CAPTURE;
    arm_anim_timer(600); /* a compositor that never answers: no animation, just the change */
}

/* ---------- the strip ---------- */

/* A restarted waybar starts hidden again: follow it. */
static void follow_bar(void)
{
    pid_t now = find_bar();
    if (now != S.bar_pid) {
        S.bar_pid = now;
        if (S.anim != A_IDLE) {
            S.toggle_pending = 0;
            finish();
        }
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

static void surface_enter(void *d, struct wl_surface *s, struct wl_output *o) { S.here = o; }
static void surface_leave(void *d, struct wl_surface *s, struct wl_output *o) {}
static const struct wl_surface_listener surface_listener = {.enter = surface_enter, .leave = surface_leave};

static void hide_bar(void)
{
    change_bar(0);
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
    else if (!strcmp(iface, zwlr_screencopy_manager_v1_interface.name)) {
        S.screencopy_version = ver < 3 ? ver : 3;
        S.screencopy = wl_registry_bind(r, name, &zwlr_screencopy_manager_v1_interface, S.screencopy_version);
    } else if (!strcmp(iface, wl_output_interface.name) && S.noutputs < MAX_OUTPUTS)
        S.outputs[S.noutputs++] = wl_registry_bind(r, name, &wl_output_interface, 1);
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
    if (S.noutputs == 1) S.here = S.outputs[0];
    S.dwell_fd = timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC | TFD_NONBLOCK);
    S.anim_fd = timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC | TFD_NONBLOCK);
    S.bar_pid = find_bar();
    S.surface = wl_compositor_create_surface(S.compositor);
    wl_surface_add_listener(S.surface, &surface_listener, NULL);
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
        struct pollfd fds[3] = {{.fd = wl_display_get_fd(S.display), .events = POLLIN},
                                {.fd = S.dwell_fd, .events = POLLIN},
                                {.fd = S.anim_fd, .events = POLLIN}};
        int n = poll(fds, 3, 2000);
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
        uint64_t t;
        if (n > 0 && (fds[1].revents & POLLIN)) {
            if (read(S.dwell_fd, &t, sizeof t) < 0) {}
            follow_bar();
            if (S.inside && S.mode == HIDDEN && S.bar_pid > 0) {
                change_bar(1);
                S.mode = SHOWN;
                place();
            }
        }
        if (n > 0 && (fds[2].revents & POLLIN)) {
            if (read(S.anim_fd, &t, sizeof t) < 0) {}
            if (S.anim == A_CAPTURE) finish(); /* no snapshot came: just change the bar */
        }
        if (n == 0) follow_bar(); /* every 2 s: has waybar restarted? */
    }
}

static void draw_ghost(double k)
{
    struct shmbuf *b = S.gbuf_cur;
    int w = S.gw, h = S.gh;
    uint32_t *dst = b->px, glow = S.accent;
    double e = ease(k);
    if (!S.showing) {
        /* Hiding: the bar's picture slides up and fades; its lower edge
         * is a line of the accent. */
        int off = (int)(e * h + 0.5);
        double a = 1 - 0.7 * e;
        for (int y = 0; y < h; y++) {
            uint32_t *row = dst + (size_t)y * w;
            int sy = y + off;
            if (sy >= h) {
                memset(row, 0, (size_t)w * 4);
                continue;
            }
            const uint32_t *src = S.snap + (size_t)sy * w;
            for (int x = 0; x < w; x++) row[x] = scale_px(src[x], a);
        }
        int edge = h - off - 1, thick = S.gscale * 2;
        for (int y = edge - thick + 1; y <= edge; y++) {
            if (y < 0 || y >= h) continue;
            uint32_t *row = dst + (size_t)y * w;
            for (int x = 0; x < w; x++) {
                /* brightest in the middle, fading to the sides */
                double fx = 1 - fabs((x - w / 2.0) / (w / 2.0));
                row[x] = blend(row[x] | 0xff000000u, glow, (1 - e) * (0.35 + 0.65 * fx));
            }
        }
    } else {
        /* Showing: what was under the bar covers it, then wipes away
         * downwards behind a scan line. */
        int reveal = (int)(e * h + 0.5);
        for (int y = 0; y < h; y++) {
            uint32_t *row = dst + (size_t)y * w;
            if (y < reveal) memset(row, 0, (size_t)w * 4);
            else memcpy(row, S.snap + (size_t)y * w, (size_t)w * 4);
        }
        int thick = S.gscale * 2;
        for (int y = reveal; y < reveal + thick && y < h; y++) {
            uint32_t *row = dst + (size_t)y * w;
            for (int x = 0; x < w; x++) {
                double fx = 1 - fabs((x - w / 2.0) / (w / 2.0));
                row[x] = blend(row[x], glow, (k > 0 ? 1 - 0.5 * e : 0) * (0.4 + 0.6 * fx));
            }
        }
    }
}
