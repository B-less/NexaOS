/* nexa-living: the Living Visual Engine (TODOnexa_magic.md, phase 3).
 *
 * One process draws the desktop's background layer (layer-shell,
 * "background"): the wallpaper, an optional desktop style around the
 * windows, and inside each first-party app's window area that app's style.
 * The apps have glass (translucent windows), so their style shows through
 * behind their content; nothing is ever drawn above a window, and this
 * surface takes no input except on the bare desktop.
 *
 * Cost control: only what moves is redrawn and reported to the compositor
 * (pixman regions: styled windows, and the desktop minus every window);
 * windows that aren't focused hold still (High: they slow down); a frame
 * cap per quality (High 60, Balanced 30, Low 12, Static: drawn only when
 * something changes); the quality steps down when frames take too long or
 * MicroBrain says Battery saver / Low resource, on battery, with reduced
 * motion, and back up with hysteresis. Measured numbers go to
 * $XDG_RUNTIME_DIR/nexa-living.stats every 5 s.
 *
 * Windows come from the compositor: Sway's IPC (window events, then the
 * tree) or Hyprland's sockets (events, then j/clients and j/monitors).
 * Apps can tell it things on $XDG_RUNTIME_DIR/nexa-living.sock (datagrams):
 *   click APP X Y      a ripple at X,Y in that app's window
 *   level APP 0.0-1.0  real activity (load, playback, copying) for 5 s
 *   reload             read the settings again (also on file change)
 *
 * Strata (TODOnexa_identity.md): each workspace 1 to 5 is a layer with its
 * own colour; the wallpaper takes the colour of the layer on its screen,
 * faintly (strata.conf: tint), cross-fading in 0.4 s when it changes.
 *
 * Settings: "living" in ~/.config/nexa/magic.json (nexa_common.magic).
 *
 *   nexa-living            run
 *   nexa-living --styles   list the styles
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/timerfd.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

#include <cairo.h>
#include <pixman.h>
#include <wayland-client.h>

#include "json.h"
#include "layer-shell.h"
#include "layers.h"
#include "styles.h"

#define MAX_OUT 8
#define MAX_WIN 32
#define MAX_RIPPLES 12
#define MAX_APPS 48

enum { Q_STATIC = -1, Q_LOW = 0, Q_BAL = 1, Q_HIGH = 2 };
static const int FPS[3] = {12, 30, 60};

struct app_style { char app[48]; int style; double level, level_at; };

struct config {
    int enabled, desktop_style, quality_pref;  /* quality_pref: -2 auto, -1 static, 0..2 */
    double speed, bright, density, reaction, veil;
    int fps_cap, battery_lower, reduced;
    struct app_style apps[MAX_APPS];
    int napps;
    char wallpaper[512];
    double strata_tint;  /* strata.conf: tint; 0 = off */
};

struct win {
    char app[48];
    long id;
    int x, y, w, h, focused, visible, fullscreen;
    char output[32];
    double first_seen;
    struct region *reg;
    int seen;
};

struct buffer {
    struct wl_buffer *wl;
    void *data;
    size_t size;
    int w, h, busy;
    cairo_surface_t *cs;
    pixman_region32_t last;  /* what was drawn into it last time */
};

struct output {
    struct wl_output *wl;
    uint32_t global;
    char name[32];
    int scale, gx, gy;   /* global logical position (from the compositor) */
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer;
    int w, h, configured, frame_pending, full;
    struct buffer buf[2];
    cairo_surface_t *base;
    struct region desk;
    double desk_at;
    /* Strata: the layer on this screen and its tint, fading from..to */
    int ws;
    rgb tint_from, tint_to;
    double tint_from_a, tint_to_a, tint_at;
};

struct ripple { char app[48]; double x, y, t0; int desk; struct output *o; };

static struct {
    struct wl_display *display;
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    struct wl_seat *seat;
    struct wl_pointer *pointer;
    struct zwlr_layer_shell_v1 *layer_shell;
    struct output out[MAX_OUT];
    int nout;
    struct output *ptr_out;
    double px, py;
    struct config cfg;
    struct win win[MAX_WIN];
    int nwin, windows_dirty;
    struct ripple rip[MAX_RIPPLES];
    int ipc_kind;          /* 0 none, 1 sway, 2 hyprland */
    int ev_fd;             /* compositor events */
    char ev_buf[65536];
    size_t ev_len;
    int app_fd, ino_fd, timer_fd;
    char cfg_path[512], strata_path[512], sock_path[512], stats_path[512], run[256];
    /* quality */
    int quality, cap_quality, perf_quality;
    double slow_since, fast_since, last_policy, last_frame, last_stats;
    double ms_sum; int ms_n, frames;
    double base_ms, desk_ms, win_ms, last_tree;
    int on_battery, brain_low, low_memory;
    double t0;
} L = {.ev_fd = -1, .app_fd = -1, .ino_fd = -1, .timer_fd = -1};

static double now(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

static char *read_file(const char *path, size_t max)
{
    FILE *f = fopen(path, "r");
    if (!f) return NULL;
    char *b = malloc(max + 1);
    size_t n = b ? fread(b, 1, max, f) : 0;
    fclose(f);
    if (b) b[n] = 0;
    return b;
}

/* ---------- settings ---------- */

static int quality_from(const char *s)
{
    if (!strcmp(s, "high")) return Q_HIGH;
    if (!strcmp(s, "balanced")) return Q_BAL;
    if (!strcmp(s, "low")) return Q_LOW;
    if (!strcmp(s, "static")) return Q_STATIC;
    return -2; /* auto */
}

static void defaults(struct config *c)
{
    memset(c, 0, sizeof *c);
    c->enabled = 1;
    c->desktop_style = ST_SIGNATURE;
    c->quality_pref = -2;
    c->speed = c->bright = c->density = 1;
    c->reaction = 0.5;
    c->veil = 0.3;
    c->battery_lower = 1;
    /* each first-party app's own personality, until changed in Settings */
    static const struct { const char *app; int st; } D[] = {
        {"nexa-settings", ST_HOLOGRID}, {"nexa-files", ST_CIRCUIT}, {"foot", ST_RAIN}, {"nexa-pulse", ST_PULSE},
        {"nexa-store", ST_ORBITAL}, {"nexa-software", ST_ORBITAL}, {"nexa-ai-gui", ST_NEURAL}, {"nexa-music", ST_PULSE},
        {"nexa-notes", ST_NEBULA}, {"nexa-calc", ST_GEOMETRY}, {"nexa-network", ST_QUANTUM}, {"nexa-power", ST_SIGNATURE}};
    for (size_t i = 0; i < sizeof D / sizeof *D; i++) {
        snprintf(c->apps[c->napps].app, sizeof c->apps[0].app, "%s", D[i].app);
        c->apps[c->napps++].style = D[i].st;
    }
    snprintf(c->wallpaper, sizeof c->wallpaper, "/usr/share/backgrounds/nexaos/wallpaper.png");
}

static int dna_reduced_motion(void)
{
    char path[600];
    const char *cfg = getenv("XDG_CONFIG_HOME"), *home = getenv("HOME");
    if (cfg && *cfg) snprintf(path, sizeof path, "%s/nexa/interface/global.json", cfg);
    else snprintf(path, sizeof path, "%s/.config/nexa/interface/global.json", home ? home : "/");
    char *t = read_file(path, 8192);
    if (!t) return 0;
    struct jdoc d = {0};
    int r = json_parse(&d, t) == 0 && !strcmp(json_str(&d, 0, "motion", ""), "none");
    json_free(&d);
    free(t);
    return r;
}

static void load_config(void)
{
    struct config c;
    defaults(&c);
    char *t = read_file(L.cfg_path, 1 << 16);
    struct jdoc d = {0};
    if (t && json_parse(&d, t) == 0) {
        int lv = json_get(&d, 0, "living");
        if (lv >= 0) {
            c.enabled = (int)json_num(&d, lv, "enabled", 1);
            const char *ds = json_str(&d, lv, "desktop", "signature");
            c.desktop_style = style_by_name(ds);  /* "none" -> -1 */
            c.quality_pref = quality_from(json_str(&d, lv, "quality", "auto"));
            if (!strcmp(json_str(&d, lv, "quality", ""), "off")) c.enabled = 0;
            c.speed = fmin(2, fmax(0.25, json_num(&d, lv, "speed", 1)));
            c.bright = fmin(1.5, fmax(0.2, json_num(&d, lv, "brightness", 1)));
            c.density = fmin(2, fmax(0.25, json_num(&d, lv, "density", 1)));
            c.reaction = fmin(1, fmax(0, json_num(&d, lv, "reaction", 0.5)));
            c.veil = fmin(0.85, fmax(0, json_num(&d, lv, "veil", 0.3)));
            c.fps_cap = (int)json_num(&d, lv, "fps_cap", 0);
            c.battery_lower = strcmp(json_str(&d, lv, "battery", "lower"), "same") != 0;
            c.reduced = (int)json_num(&d, lv, "reduced_motion", 0);
            const char *wp = json_str(&d, lv, "wallpaper", NULL);
            if (wp && *wp) snprintf(c.wallpaper, sizeof c.wallpaper, "%s", wp);
            int apps = json_get(&d, lv, "apps");
            if (apps >= 0 && d.n[apps].type == J_OBJ) {
                for (int a = d.n[apps].child; a >= 0; a = d.n[a].next) {
                    if (d.n[a].type != J_STR || !d.n[a].key) continue;
                    int slot = -1;
                    for (int i = 0; i < c.napps; i++) if (!strcmp(c.apps[i].app, d.n[a].key)) slot = i;
                    if (slot < 0 && c.napps < MAX_APPS) slot = c.napps++;
                    if (slot < 0) continue;
                    snprintf(c.apps[slot].app, sizeof c.apps[0].app, "%s", d.n[a].key);
                    c.apps[slot].style = style_by_name(d.n[a].str);  /* "off" -> -1 */
                }
            }
        }
    }
    json_free(&d);
    free(t);
    c.reduced |= dna_reduced_motion();
    {
        char *st = read_file(L.strata_path, 4096);
        struct strata_conf sc;
        strata_conf_parse(&sc, st);
        free(st);
        c.strata_tint = sc.tint;
    }
    /* keep the live levels across a reload */
    for (int i = 0; i < c.napps; i++)
        for (int j = 0; j < L.cfg.napps; j++)
            if (!strcmp(c.apps[i].app, L.cfg.apps[j].app)) {
                c.apps[i].level = L.cfg.apps[j].level;
                c.apps[i].level_at = L.cfg.apps[j].level_at;
            }
    int wall_changed = strcmp(c.wallpaper, L.cfg.wallpaper) != 0;
    L.cfg = c;
    for (int i = 0; i < L.nout; i++) {
        if (wall_changed && L.out[i].base) { cairo_surface_destroy(L.out[i].base); L.out[i].base = NULL; }
        L.out[i].full = 1;
    }
}

static struct app_style *style_of(const char *app)
{
    for (int i = 0; i < L.cfg.napps; i++)
        if (!strcmp(L.cfg.apps[i].app, app)) return &L.cfg.apps[i];
    return NULL;
}

/* ---------- quality policy ---------- */

/* MicroBrain's mode: Battery saver or Low resource hold the visuals down. */
static void ask_brain(void)
{
    char path[300];
    snprintf(path, sizeof path, "%s/nexa-microbrain.sock", L.run);
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return;
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    snprintf(a.sun_path, sizeof a.sun_path, "%.107s", path);
    struct timeval tv = {0, 300000};
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    char buf[16384];
    size_t got = 0;
    if (connect(fd, (struct sockaddr *)&a, sizeof a) == 0 && write(fd, "{\"cmd\":\"status\"}\n", 17) == 17) {
        ssize_t r;
        while (got < sizeof buf - 1 && (r = read(fd, buf + got, sizeof buf - 1 - got)) > 0) {
            got += (size_t)r;
            if (memchr(buf, '\n', got)) break;
        }
    }
    close(fd);
    buf[got] = 0;
    L.brain_low = strstr(buf, "\"battery-saver\"") || strstr(buf, "\"low-resource\"") || strstr(buf, "\"quiet\"");
}

static void policy(double t)
{
    if (t - L.last_policy < 5 && L.last_policy > 0) return;
    L.last_policy = t;
    /* on battery? */
    L.on_battery = 0;
    for (int i = 0; i < 4; i++) {
        char p[96];
        snprintf(p, sizeof p, "/sys/class/power_supply/BAT%d/status", i);
        char *s = read_file(p, 32);
        if (s && !strncmp(s, "Discharging", 11)) L.on_battery = 1;
        free(s);
    }
    ask_brain();
    /* the 2 GB profile: MemTotal under 3 GB */
    char *mi = read_file("/proc/meminfo", 4096);
    long kb = 0;
    if (mi) sscanf(mi, "MemTotal: %ld", &kb);
    free(mi);
    L.low_memory = kb > 0 && kb < 3L * 1024 * 1024;
    int q = L.cfg.quality_pref == -2 ? (L.low_memory ? Q_LOW : Q_BAL) : L.cfg.quality_pref;
    if (L.cfg.reduced) q = Q_STATIC;
    if ((L.on_battery && L.cfg.battery_lower) || L.brain_low) q = q > Q_LOW ? Q_LOW : q;
    L.cap_quality = q;
    int eff = q < L.perf_quality ? q : L.perf_quality;
    if (eff != L.quality) {
        L.quality = eff;
        for (int i = 0; i < L.nout; i++) L.out[i].full = 1;
    }
}

/* Frames taking more than half their time for 2 s: one step down. Back up
 * only after 10 s comfortably fast. */
static void adapt(double ms, double t)
{
    if (L.quality < 0) return;
    double period = 1000.0 / FPS[L.quality];
    if (ms > period * 0.5) {
        L.fast_since = 0;
        if (!L.slow_since) L.slow_since = t;
        if (t - L.slow_since > 2 && L.perf_quality > Q_STATIC) {
            L.perf_quality = (L.quality > Q_LOW ? L.quality : Q_LOW + 1) - 1;
            if (L.quality == Q_LOW) L.perf_quality = Q_STATIC;
            L.slow_since = 0;
            L.last_policy = 0;
        }
    } else if (ms < period * 0.2) {
        L.slow_since = 0;
        if (!L.fast_since) L.fast_since = t;
        if (t - L.fast_since > 10 && L.perf_quality < Q_HIGH) {
            L.perf_quality++;
            L.fast_since = 0;
            L.last_policy = 0;
        }
    } else {
        L.slow_since = L.fast_since = 0;
    }
}

/* ---------- the compositor's windows ---------- */

static int connect_unix(const char *path)
{
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    snprintf(a.sun_path, sizeof a.sun_path, "%.107s", path);
    if (connect(fd, (struct sockaddr *)&a, sizeof a) < 0) { close(fd); return -1; }
    return fd;
}

static int write_all(int fd, const void *p, size_t n)
{
    const char *c = p;
    while (n) {
        ssize_t w = write(fd, c, n);
        if (w <= 0) return -1;
        c += w;
        n -= (size_t)w;
    }
    return 0;
}

static int read_all(int fd, void *p, size_t n)
{
    char *c = p;
    while (n) {
        ssize_t r = read(fd, c, n);
        if (r <= 0) return -1;
        c += r;
        n -= (size_t)r;
    }
    return 0;
}

/* One Sway IPC request; the reply's payload (malloc'ed). */
static char *sway_request(uint32_t type, const char *payload)
{
    const char *sock = getenv("SWAYSOCK");
    int fd = sock ? connect_unix(sock) : -1;
    if (fd < 0) return NULL;
    uint32_t len = (uint32_t)strlen(payload), hdr[2] = {len, type};
    char reply = 0;
    char *out = NULL;
    if (write_all(fd, "i3-ipc", 6) == 0 && write_all(fd, hdr, 8) == 0 && write_all(fd, payload, len) == 0) {
        char magic[6];
        uint32_t rh[2];
        if (read_all(fd, magic, 6) == 0 && read_all(fd, rh, 8) == 0 && rh[0] < (64u << 20)) {
            out = malloc(rh[0] + 1);
            if (out && read_all(fd, out, rh[0]) == 0) { out[rh[0]] = 0; reply = 1; }
        }
    }
    close(fd);
    if (!reply) { free(out); return NULL; }
    return out;
}

static struct win *find_win(long id)
{
    for (int i = 0; i < L.nwin; i++) if (L.win[i].id == id) return &L.win[i];
    return NULL;
}

static void note_window(long id, const char *app, int x, int y, int w, int h, int focused, int visible, int fs, const char *output)
{
    struct win *wn = find_win(id);
    if (!wn) {
        if (L.nwin >= MAX_WIN) return;
        wn = &L.win[L.nwin++];
        memset(wn, 0, sizeof *wn);
        wn->id = id;
        wn->first_seen = now();
    }
    snprintf(wn->app, sizeof wn->app, "%s", app ? app : "");
    snprintf(wn->output, sizeof wn->output, "%s", output ? output : "");
    wn->x = x; wn->y = y; wn->w = w; wn->h = h;
    wn->focused = focused; wn->visible = visible; wn->fullscreen = fs;
    wn->seen = 1;
}

static void set_layer(struct output *o, int n);

static struct output *output_named(const char *name)
{
    for (int i = 0; i < L.nout; i++)
        if (!strcmp(L.out[i].name, name)) return &L.out[i];
    return NULL;
}

static void sway_walk(const struct jdoc *d, int n, const char *output)
{
    const char *type = json_str(d, n, "type", "");
    if (!strcmp(type, "output")) {
        output = json_str(d, n, "name", "");
        if (!strcmp(output, "__i3")) return;  /* the scratchpad: hidden */
        int r = json_get(d, n, "rect");
        for (int i = 0; i < L.nout; i++)
            if (!strcmp(L.out[i].name, output)) {
                L.out[i].gx = (int)json_num(d, r, "x", 0);
                L.out[i].gy = (int)json_num(d, r, "y", 0);
            }
    }
    if ((!strcmp(type, "con") || !strcmp(type, "floating_con")) && json_get(d, n, "pid") >= 0) {
        int r = json_get(d, n, "rect");
        note_window((long)json_num(d, n, "id", 0), json_str(d, n, "app_id", ""), (int)json_num(d, r, "x", 0), (int)json_num(d, r, "y", 0),
                    (int)json_num(d, r, "width", 0), (int)json_num(d, r, "height", 0), (int)json_num(d, n, "focused", 0),
                    (int)json_num(d, n, "visible", 0), (int)json_num(d, n, "fullscreen_mode", 0) > 0, output);
    }
    static const char *const kids[2] = {"nodes", "floating_nodes"};
    for (int k = 0; k < 2; k++) {
        int a = json_get(d, n, kids[k]);
        if (a < 0) continue;
        for (int c = d->n[a].child; c >= 0; c = d->n[c].next) sway_walk(d, c, output);
    }
}

static char *hypr_request(const char *what)
{
    const char *sig = getenv("HYPRLAND_INSTANCE_SIGNATURE");
    if (!sig) return NULL;
    char path[400];
    snprintf(path, sizeof path, "%s/hypr/%s/.socket.sock", L.run, sig);
    int fd = connect_unix(path);
    if (fd < 0) return NULL;
    if (write_all(fd, what, strlen(what)) < 0) { close(fd); return NULL; }
    size_t cap = 1 << 16, n = 0;
    char *b = malloc(cap);
    ssize_t r;
    while (b && (r = read(fd, b + n, cap - n - 1)) > 0) {
        n += (size_t)r;
        if (cap - n < 4096) {
            char *nb = realloc(b, cap * 2);
            if (!nb) break;
            b = nb;
            cap *= 2;
        }
    }
    close(fd);
    if (b) b[n] = 0;
    return b;
}

static void hypr_windows(void)
{
    char *mon = hypr_request("j/monitors"), *cl = hypr_request("j/clients");
    struct jdoc dm = {0}, dc = {0};
    if (mon && cl && json_parse(&dm, mon) == 0 && json_parse(&dc, cl) == 0) {
        for (int m = dm.n[0].child; m >= 0; m = dm.n[m].next) {
            struct output *o = output_named(json_str(&dm, m, "name", ""));
            int ws = (int)json_num(&dm, json_get(&dm, m, "activeWorkspace"), "id", 0);
            if (o) set_layer(o, ws > 0 ? ws : 0);
        }
        for (int c = dc.n[0].child; c >= 0; c = dc.n[c].next) {
            int ws = json_get(&dc, c, "workspace"), at = json_get(&dc, c, "at"), sz = json_get(&dc, c, "size");
            if (at < 0 || sz < 0 || dc.n[at].child < 0 || dc.n[sz].child < 0) continue;
            int wsid = (int)json_num(&dc, ws, "id", -99), monid = (int)json_num(&dc, c, "monitor", -1), visible = 0;
            const char *oname = "";
            for (int m = dm.n[0].child; m >= 0; m = dm.n[m].next) {
                if ((int)json_num(&dm, m, "id", -2) != monid) continue;
                oname = json_str(&dm, m, "name", "");
                visible = (int)json_num(&dm, json_get(&dm, m, "activeWorkspace"), "id", -98) == wsid ||
                          (int)json_num(&dm, json_get(&dm, m, "specialWorkspace"), "id", -98) == wsid;
                for (int i = 0; i < L.nout; i++)
                    if (!strcmp(L.out[i].name, oname)) {
                        L.out[i].gx = (int)json_num(&dm, m, "x", 0);
                        L.out[i].gy = (int)json_num(&dm, m, "y", 0);
                    }
            }
            const char *addr = json_str(&dc, c, "address", "0x0");
            int ax = dc.n[at].child, sx = dc.n[sz].child;
            note_window(strtol(addr, NULL, 16), json_str(&dc, c, "class", ""), (int)dc.n[ax].num, (int)dc.n[dc.n[ax].next].num,
                        (int)dc.n[sx].num, (int)dc.n[dc.n[sx].next].num, (int)json_num(&dc, c, "focusHistoryID", 1) == 0,
                        visible && !(int)json_num(&dc, c, "hidden", 0) && (int)json_num(&dc, c, "mapped", 1),
                        (int)json_num(&dc, c, "fullscreen", 0) > 0, oname);
        }
    }
    json_free(&dm);
    json_free(&dc);
    free(mon);
    free(cl);
}

/* What matters for drawing, to tell whether a refresh changed anything. */
static unsigned long layout_hash(void)
{
    unsigned long h = 1469598103934665603UL ^ (unsigned long)L.nwin;
    for (int i = 0; i < L.nwin; i++) {
        const struct win *w = &L.win[i];
        long v[8] = {w->id, w->x, w->y, w->w, w->h, w->visible, w->focused, w->fullscreen};
        for (int k = 0; k < 8; k++) h = (h ^ (unsigned long)v[k]) * 1099511628211UL;
    }
    return h;
}

static void refresh_windows(void)
{
    unsigned long before = layout_hash();
    for (int i = 0; i < L.nwin; i++) L.win[i].seen = 0;
    if (L.ipc_kind == 1) {
        char *tree = sway_request(4, "");
        struct jdoc d = {0};
        if (tree && json_parse(&d, tree) == 0) sway_walk(&d, 0, "");
        json_free(&d);
        free(tree);
        char *ws = sway_request(1, "");
        struct jdoc dw = {0};
        if (ws && json_parse(&dw, ws) == 0 && dw.n[0].type == J_ARR)
            for (int c = dw.n[0].child; c >= 0; c = dw.n[c].next) {
                struct output *o = output_named(json_str(&dw, c, "output", ""));
                int num = (int)json_num(&dw, c, "num", 0);
                if (o && json_num(&dw, c, "visible", 0)) set_layer(o, num > 0 ? num : 0);
            }
        json_free(&dw);
        free(ws);
    } else if (L.ipc_kind == 2) {
        hypr_windows();
    }
    /* windows gone: drop them (and their geometry) */
    for (int i = 0; i < L.nwin;) {
        if (!L.win[i].seen) {
            if (L.win[i].reg) { region_free(L.win[i].reg); free(L.win[i].reg); }
            L.win[i] = L.win[--L.nwin];
        } else {
            i++;
        }
    }
    L.windows_dirty = 0;
    if (layout_hash() != before)
        for (int i = 0; i < L.nout; i++) L.out[i].full = 1;
}

static void connect_events(void)
{
    if (getenv("HYPRLAND_INSTANCE_SIGNATURE")) {
        char path[400];
        snprintf(path, sizeof path, "%s/hypr/%s/.socket2.sock", L.run, getenv("HYPRLAND_INSTANCE_SIGNATURE"));
        L.ev_fd = connect_unix(path);
        L.ipc_kind = 2;
    } else if (getenv("SWAYSOCK")) {
        L.ev_fd = connect_unix(getenv("SWAYSOCK"));
        const char *sub = "[\"window\",\"workspace\",\"output\"]";
        uint32_t hdr[2] = {(uint32_t)strlen(sub), 2};
        if (L.ev_fd >= 0 && (write_all(L.ev_fd, "i3-ipc", 6) < 0 || write_all(L.ev_fd, hdr, 8) < 0 || write_all(L.ev_fd, sub, strlen(sub)) < 0)) {
            close(L.ev_fd);
            L.ev_fd = -1;
        }
        L.ipc_kind = 1;
    }
    L.windows_dirty = 1;
}

/* Anything from the compositor's event stream means "look again". */
static void read_events(void)
{
    char tmp[8192];
    ssize_t r = read(L.ev_fd, tmp, sizeof tmp);
    if (r <= 0) {
        close(L.ev_fd);
        L.ev_fd = -1;
        return;
    }
    L.windows_dirty = 1;
}

/* ---------- drawing ---------- */

static rgb hexcol(const char *s, rgb fb)
{
    unsigned v;
    if (!s || sscanf(s, "#%6x", &v) != 1) return fb;
    return (rgb){((v >> 16) & 255) / 255.0, ((v >> 8) & 255) / 255.0, (v & 255) / 255.0};
}

static rgb A1 = {0.24, 0.55, 1.0}, A2 = {0.54, 0.36, 1.0}, A3 = {0.92, 0.95, 1.0}, BG = {0.03, 0.05, 0.12};

/* The theme's colours (nexa-theme writes them for the lock screen too). */
static void load_colours(void)
{
    char path[600];
    const char *cfg = getenv("XDG_CONFIG_HOME"), *home = getenv("HOME");
    if (cfg && *cfg) snprintf(path, sizeof path, "%s/nexa/lock.conf", cfg);
    else snprintf(path, sizeof path, "%s/.config/nexa/lock.conf", home ? home : "/");
    FILE *f = fopen(path, "r");
    if (!f) return;
    char line[256];
    while (fgets(line, sizeof line, f)) {
        line[strcspn(line, "\n")] = 0;
        if (!strncmp(line, "accent=", 7)) A1 = hexcol(line + 7, A1);
        else if (!strncmp(line, "accent2=", 8)) A2 = hexcol(line + 8, A2);
        else if (!strncmp(line, "bg=", 3)) BG = hexcol(line + 3, BG);
        else if (!strncmp(line, "text=", 5)) A3 = hexcol(line + 5, A3);
    }
    fclose(f);
}

/* ---------- Strata's tint ---------- */

static rgb tint_of(int n)
{
    strata_rgb t = strata_tint((strata_rgb){A1.r, A1.g, A1.b}, n);
    return (rgb){t.r, t.g, t.b};
}

/* How far the fade to the new layer's tint is: 0..1. */
static double tint_k(const struct output *o)
{
    if (L.cfg.reduced || o->tint_at <= 0) return 1;
    return fmin(1, fmax(0, (now() - o->tint_at) / 0.4));
}

static void tint_now(const struct output *o, rgb *c, double *a)
{
    double k = tint_k(o);
    *c = (rgb){o->tint_from.r + (o->tint_to.r - o->tint_from.r) * k, o->tint_from.g + (o->tint_to.g - o->tint_from.g) * k,
               o->tint_from.b + (o->tint_to.b - o->tint_from.b) * k};
    *a = o->tint_from_a + (o->tint_to_a - o->tint_from_a) * k;
}

static void tint_target(struct output *o, rgb fallback)
{
    int on = o->ws >= 1 && o->ws <= STRATA_LAYERS;
    o->tint_to = on ? tint_of(o->ws) : fallback;
    o->tint_to_a = on ? L.cfg.strata_tint : 0;  /* not a layer (a named workspace): no tint */
}

static void set_layer(struct output *o, int n)
{
    if (n == o->ws && o->tint_at > 0) return;
    rgb c;
    double a;
    tint_now(o, &c, &a);
    int first = o->tint_at <= 0;
    o->ws = n;
    tint_target(o, c);
    o->tint_from = first ? o->tint_to : c;  /* at start: no fade */
    o->tint_from_a = first ? o->tint_to_a : a;
    o->tint_at = now();
    o->full = 1;
}

/* New colours or strength: straight to them, no fade. */
static void retint(void)
{
    for (int i = 0; i < L.nout; i++) {
        struct output *o = &L.out[i];
        tint_target(o, o->tint_to);
        o->tint_from = o->tint_to;
        o->tint_from_a = o->tint_to_a;
        o->full = 1;
    }
}

static cairo_surface_t *make_base(int BW, int BH)
{
    cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, BW, BH);
    cairo_t *cr = cairo_create(s);
    cairo_set_source_rgb(cr, BG.r, BG.g, BG.b);
    cairo_paint(cr);
    cairo_surface_t *wp = cairo_image_surface_create_from_png(L.cfg.wallpaper);
    if (cairo_surface_status(wp) == CAIRO_STATUS_SUCCESS) {
        double ww = cairo_image_surface_get_width(wp), wh = cairo_image_surface_get_height(wp);
        double sc = fmax(BW / ww, BH / wh);
        cairo_translate(cr, (BW - ww * sc) / 2, (BH - wh * sc) / 2);
        cairo_scale(cr, sc, sc);
        cairo_set_source_surface(cr, wp, 0, 0);
        cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_GOOD);
        cairo_paint(cr);
    }
    cairo_surface_destroy(wp);
    cairo_destroy(cr);
    return s;
}

static void buffer_release(void *data, struct wl_buffer *b) { ((struct buffer *)data)->busy = 0; }
static const struct wl_buffer_listener buffer_listener = {.release = buffer_release};

static void free_buffer(struct buffer *b)
{
    if (b->cs) cairo_surface_destroy(b->cs);
    if (b->wl) wl_buffer_destroy(b->wl);
    if (b->data) munmap(b->data, b->size);
    pixman_region32_fini(&b->last);
    memset(b, 0, sizeof *b);
    pixman_region32_init(&b->last);
}

static int make_buffer(struct buffer *b, int w, int h)
{
    free_buffer(b);
    int stride = w * 4;
    size_t size = (size_t)stride * h;
    int fd = memfd_create("nexa-living", MFD_CLOEXEC);
    if (fd < 0) return -1;
    if (ftruncate(fd, (off_t)size) < 0) { close(fd); return -1; }
    void *data = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (data == MAP_FAILED) { close(fd); return -1; }
    struct wl_shm_pool *pool = wl_shm_create_pool(L.shm, fd, (int32_t)size);
    b->wl = wl_shm_pool_create_buffer(pool, 0, w, h, stride, WL_SHM_FORMAT_XRGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    wl_buffer_add_listener(b->wl, &buffer_listener, b);
    b->data = data;
    b->size = size;
    b->w = w;
    b->h = h;
    b->cs = cairo_image_surface_create_for_data(data, CAIRO_FORMAT_RGB24, w, h, stride);
    pixman_region32_init_rect(&b->last, 0, 0, (unsigned)w, (unsigned)h);  /* nothing valid in it yet */
    return 0;
}

static struct params params_for(double t, double focus, double level)
{
    return (struct params){.t = t, .speed = L.cfg.speed, .bright = L.cfg.bright, .density = L.cfg.density, .level = level,
                           .focus = focus, .px = -1e9, .py = -1e9, .reaction = L.cfg.reaction,
                           .quality = L.quality < 0 ? Q_BAL : L.quality, .a1 = A1, .a2 = A2, .a3 = A3};
}

/* Does this window get a style, and is it moving now? */
static int win_style(const struct win *w)
{
    if (!w->visible || w->w < 40 || w->h < 40) return -1;
    struct app_style *a = style_of(w->app);
    return a ? a->style : -1;
}

static int win_animates(const struct win *w)
{
    if (L.quality < 0 || win_style(w) < 0) return 0;
    return w->focused || L.quality == Q_HIGH || now() - w->first_seen < 1.0;
}

static int on_output(const struct win *w, const struct output *o)
{
    return !strcmp(w->output, o->name) || (L.nout == 1 && !w->output[0]);
}

static void draw_ripples(cairo_t *cr, struct output *o, const char *app, double t)
{
    for (int i = 0; i < MAX_RIPPLES; i++) {
        struct ripple *r = &L.rip[i];
        double age = t - r->t0;
        if (r->t0 <= 0 || age > 0.9) continue;
        if (app ? (r->desk || strcmp(r->app, app)) : (!r->desk || r->o != o)) continue;
        double rad = 8 + age * 160, a = (1 - age / 0.9) * 0.55;
        cairo_arc(cr, r->x, r->y, rad, 0, 2 * M_PI);
        cairo_set_source_rgba(cr, A1.r, A1.g, A1.b, a);
        cairo_set_line_width(cr, 2);
        cairo_stroke(cr);
        cairo_arc(cr, r->x, r->y, rad * 0.6, 0, 2 * M_PI);
        cairo_set_source_rgba(cr, A2.r, A2.g, A2.b, a * 0.5);
        cairo_set_line_width(cr, 1);
        cairo_stroke(cr);
    }
}

static int ripples_live(double t)
{
    for (int i = 0; i < MAX_RIPPLES; i++) if (L.rip[i].t0 > 0 && t - L.rip[i].t0 < 0.9) return 1;
    return 0;
}

static int app_ripple_live(const char *app, double t)
{
    for (int i = 0; i < MAX_RIPPLES; i++)
        if (L.rip[i].t0 > 0 && t - L.rip[i].t0 < 0.9 && !L.rip[i].desk && !strcmp(L.rip[i].app, app)) return 1;
    return 0;
}

static void add_rect(pixman_region32_t *r, int x, int y, int w, int h)
{
    if (w <= 0 || h <= 0) return;
    pixman_region32_union_rect(r, r, x, y, (unsigned)w, (unsigned)h);
}

static void render(struct output *o)
{
    if (!o->configured || !o->w || !o->h) return;
    struct buffer *b = !o->buf[0].busy ? &o->buf[0] : !o->buf[1].busy ? &o->buf[1] : NULL;
    if (!b) return;
    int s = o->scale > 0 ? o->scale : 1, BW = o->w * s, BH = o->h * s;
    if ((b->w != BW || b->h != BH) && make_buffer(b, BW, BH) < 0) return;
    if (!o->base) {
        o->base = make_base(BW, BH);
        o->full = 1;
    }
    double t = now() - L.t0, t_start = now();

    /* What moves this frame, in output-local logical pixels. */
    pixman_region32_t moving, windows, desk, damage;
    pixman_region32_init(&moving);
    pixman_region32_init(&windows);
    pixman_region32_init_rect(&desk, 0, 0, (unsigned)o->w, (unsigned)o->h);
    int fullscreen = 0;
    for (int i = 0; i < L.nwin; i++) {
        struct win *w = &L.win[i];
        if (!w->visible || !on_output(w, o)) continue;
        add_rect(&windows, w->x - o->gx, w->y - o->gy, w->w, w->h);
        fullscreen |= w->fullscreen;
        if (win_animates(w) || (win_style(w) >= 0 && app_ripple_live(w->app, t)))
            add_rect(&moving, w->x - o->gx, w->y - o->gy, w->w, w->h);
    }
    pixman_region32_subtract(&desk, &desk, &windows);
    /* the desktop is slow and calm: 15 frames a second at most */
    int desk_moves = L.cfg.desktop_style >= 0 && L.quality >= 0 && !fullscreen && t - o->desk_at >= 1.0 / 15 - 0.004;
    if (desk_moves) o->desk_at = t;
    if (desk_moves) pixman_region32_union(&moving, &moving, &desk);
    if (ripples_live(t)) pixman_region32_union(&moving, &moving, &desk);

    /* This buffer is two frames old: redraw what changed since then too. */
    pixman_region32_init(&damage);
    if (o->full) pixman_region32_init_rect(&damage, 0, 0, (unsigned)o->w, (unsigned)o->h);
    else pixman_region32_union(&damage, &moving, &moving);
    pixman_region32_t todo;
    pixman_region32_init(&todo);
    pixman_region32_union(&todo, &damage, &b->last);
    pixman_region32_copy(&b->last, &damage);
    if (!pixman_region32_not_empty(&todo)) {
        pixman_region32_fini(&moving); pixman_region32_fini(&windows); pixman_region32_fini(&desk);
        pixman_region32_fini(&damage); pixman_region32_fini(&todo);
        return;
    }

    double p0 = now();
    cairo_t *cr = cairo_create(b->cs);
    int nr;
    pixman_box32_t *bx = pixman_region32_rectangles(&todo, &nr);
    for (int i = 0; i < nr; i++) cairo_rectangle(cr, bx[i].x1 * s, bx[i].y1 * s, (bx[i].x2 - bx[i].x1) * s, (bx[i].y2 - bx[i].y1) * s);
    cairo_clip(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    cairo_set_source_surface(cr, o->base, 0, 0);
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
    {  /* Strata: the layer's colour over the wallpaper, under every style */
        rgb tc;
        double ta;
        tint_now(o, &tc, &ta);
        if (ta > 0.002) {
            cairo_set_source_rgba(cr, tc.r, tc.g, tc.b, ta);
            cairo_paint(cr);
        }
    }
    cairo_scale(cr, s, s);
    double p1 = now();

    /* the desktop, around the windows */
    if (L.cfg.desktop_style >= 0 && !fullscreen) {
        cairo_save(cr);
        int nd;
        pixman_box32_t *db = pixman_region32_rectangles(&desk, &nd);
        cairo_new_path(cr);
        for (int i = 0; i < nd; i++) cairo_rectangle(cr, db[i].x1, db[i].y1, db[i].x2 - db[i].x1, db[i].y2 - db[i].y1);
        cairo_clip(cr);
        region_prepare(&o->desk, L.cfg.desktop_style, o->w, o->h, L.cfg.density * (L.quality == Q_LOW ? 0.7 : 1), 3);
        struct params p = params_for(L.quality < 0 ? 4.0 : t, 0.75, 0);
        if (L.ptr_out == o) { p.px = L.px; p.py = L.py; }
        style_draw(cr, &o->desk, &p);
        draw_ripples(cr, o, NULL, t);
        cairo_restore(cr);
    }
    double p2 = now();
    /* each styled window, behind its glass */
    for (int i = 0; i < L.nwin; i++) {
        struct win *w = &L.win[i];
        int st = win_style(w);
        if (st < 0 || !on_output(w, o)) continue;
        double x = w->x - o->gx, y = w->y - o->gy;
        cairo_save(cr);
        cairo_rectangle(cr, x, y, w->w, w->h);
        cairo_clip(cr);
        cairo_translate(cr, x, y);
        cairo_set_source_rgba(cr, BG.r, BG.g, BG.b, L.cfg.veil);
        cairo_paint(cr);
        if (!w->reg) w->reg = calloc(1, sizeof *w->reg);
        if (w->reg) {
            region_prepare(w->reg, st, w->w, w->h, L.cfg.density * (L.quality == Q_LOW ? 0.7 : 1), (unsigned)w->id);
            struct app_style *a = style_of(w->app);
            double level = a && now() - a->level_at < 5 ? a->level : 0;
            double intro = fmin(1, (now() - w->first_seen) / 0.6);
            /* still while not focused (it keeps its last pose), slower on High */
            double tt = L.quality < 0 ? 4.0 : w->focused ? t : L.quality == Q_HIGH ? t * 0.35 : w->first_seen - L.t0 + 1;
            struct params p = params_for(tt, (w->focused ? 1 : 0.55) * intro, level);
            if (L.quality == Q_HIGH && !w->focused) p.t = t * 0.35;
            style_draw(cr, w->reg, &p);
            draw_ripples(cr, o, w->app, t);
        }
        cairo_restore(cr);
    }
    cairo_destroy(cr);
    cairo_surface_flush(b->cs);
    double p3 = now();
    L.base_ms += (p1 - p0) * 1000;
    L.desk_ms += (p2 - p1) * 1000;
    L.win_ms += (p3 - p2) * 1000;

    wl_surface_set_buffer_scale(o->surface, s);
    wl_surface_attach(o->surface, b->wl, 0, 0);
    bx = pixman_region32_rectangles(&todo, &nr);
    for (int i = 0; i < nr; i++)
        wl_surface_damage_buffer(o->surface, bx[i].x1 * s, bx[i].y1 * s, (bx[i].x2 - bx[i].x1) * s, (bx[i].y2 - bx[i].y1) * s);
    b->busy = 1;
    double ms = (now() - t_start) * 1000;
    L.ms_sum += ms;
    L.ms_n++;
    L.frames++;
    /* Full redraws (start, a window moved, new colours: caches being made)
     * aren't what a frame normally costs: they don't count against it. */
    if (!o->full) adapt(ms, now());
    o->full = tint_k(o) < 1;  /* while the tint fades, the whole screen changes */
    pixman_region32_fini(&moving); pixman_region32_fini(&windows); pixman_region32_fini(&desk);
    pixman_region32_fini(&damage); pixman_region32_fini(&todo);
}

static void frame_done(void *data, struct wl_callback *cb, uint32_t t);
static const struct wl_callback_listener frame_listener = {.done = frame_done};

static void commit(struct output *o)
{
    struct wl_callback *cb = wl_surface_frame(o->surface);
    wl_callback_add_listener(cb, &frame_listener, o);
    o->frame_pending = 1;
    wl_surface_commit(o->surface);
}

static void frame_done(void *data, struct wl_callback *cb, uint32_t t)
{
    wl_callback_destroy(cb);
    ((struct output *)data)->frame_pending = 0;
}

/* Is anything on this output moving right now? */
static int animating(struct output *o)
{
    if (tint_k(o) < 1) return 1;
    if (L.quality < 0) return ripples_live(now() - L.t0);
    if (L.cfg.desktop_style >= 0) return 1;
    for (int i = 0; i < L.nwin; i++) if (on_output(&L.win[i], o) && win_animates(&L.win[i])) return 1;
    return ripples_live(now() - L.t0);
}

static void draw_all(int force)
{
    double t = now();
    int fps = L.quality < 0 ? 30 : FPS[L.quality];
    if (L.cfg.fps_cap > 0 && L.cfg.fps_cap < fps) fps = L.cfg.fps_cap;
    if (!force && t - L.last_frame < 1.0 / fps - 0.002) return;
    int drew = 0;
    for (int i = 0; i < L.nout; i++) {
        struct output *o = &L.out[i];
        if (!o->configured || o->frame_pending) continue;
        if (!o->full && !animating(o)) continue;
        render(o);
        commit(o);
        drew = 1;
    }
    if (drew) L.last_frame = t;
}

static void write_stats(void)
{
    double t = now();
    if (t - L.last_stats < 5) return;
    double span = t - L.last_stats;
    L.last_stats = t;
    char *st = read_file("/proc/self/statm", 128);
    long pages = 0, rss = 0;
    if (st) sscanf(st, "%ld %ld", &pages, &rss);
    free(st);
    static const char *qn[4] = {"static", "low", "balanced", "high"};
    int styled = 0;
    for (int i = 0; i < L.nwin; i++) styled += win_style(&L.win[i]) >= 0;
    rgb tc = {0, 0, 0};
    double ta = 0;
    if (L.nout) tint_now(&L.out[0], &tc, &ta);
    FILE *f = fopen(L.stats_path, "w");
    if (f) {
        fprintf(f, "{\"quality\": \"%s\", \"cap\": \"%s\", \"fps\": %.1f, \"frame_ms\": %.2f, \"rss_kb\": %ld, \"windows\": %d, \"styled\": %d, "
                   "\"on_battery\": %d, \"brain_low\": %d, \"low_memory\": %d, \"enabled\": %d, "
                   "\"base_ms\": %.2f, \"desktop_ms\": %.2f, \"windows_ms\": %.2f, "
                   "\"layer\": %d, \"tint\": \"#%02x%02x%02x\", \"tint_alpha\": %.3f}\n",
                qn[L.quality + 1], qn[L.cap_quality + 1], L.frames / span, L.ms_n ? L.ms_sum / L.ms_n : 0.0,
                rss * (sysconf(_SC_PAGESIZE) / 1024), L.nwin, styled, L.on_battery, L.brain_low, L.low_memory, L.cfg.enabled,
                L.ms_n ? L.base_ms / L.ms_n : 0.0, L.ms_n ? L.desk_ms / L.ms_n : 0.0, L.ms_n ? L.win_ms / L.ms_n : 0.0,
                L.nout ? L.out[0].ws : 0, (int)lround(tc.r * 255), (int)lround(tc.g * 255), (int)lround(tc.b * 255), ta);
        fclose(f);
    }
    L.frames = 0;
    L.base_ms = L.desk_ms = L.win_ms = 0;
    L.ms_sum = 0;
    L.ms_n = 0;
}

/* The theme changed (nexa-theme rewrote lock.conf) or the settings did:
 * new colours everywhere, so caches made with the old ones go too. */
static void restyle(void)
{
    rgb o1 = A1, o2 = A2, o3 = A3, ob = BG;
    load_colours();
    load_config();
    retint();
    L.last_policy = 0;
    int recolour = memcmp(&o1, &A1, sizeof A1) || memcmp(&o2, &A2, sizeof A2) || memcmp(&o3, &A3, sizeof A3) || memcmp(&ob, &BG, sizeof BG);
    for (int i = 0; i < L.nout; i++) {
        struct output *o = &L.out[i];
        if (recolour) {
            if (o->base) { cairo_surface_destroy(o->base); o->base = NULL; }
            region_free(&o->desk);
        }
        o->full = 1;
    }
    if (recolour)
        for (int i = 0; i < L.nwin; i++) if (L.win[i].reg) region_free(L.win[i].reg);
}

/* ---------- apps telling us things ---------- */

static void app_message(void)
{
    char m[256];
    ssize_t n = recv(L.app_fd, m, sizeof m - 1, 0);
    if (n <= 0) return;
    m[n] = 0;
    char app[48] = "";
    double x, y, v;
    double t = now() - L.t0;
    if (sscanf(m, "click %47s %lf %lf", app, &x, &y) == 3) {
        int slot = 0;
        for (int i = 0; i < MAX_RIPPLES; i++) if (L.rip[i].t0 < L.rip[slot].t0) slot = i;
        L.rip[slot] = (struct ripple){.x = x, .y = y, .t0 = t};
        snprintf(L.rip[slot].app, sizeof L.rip[slot].app, "%s", app);
    } else if (sscanf(m, "level %47s %lf", app, &v) == 2) {
        struct app_style *a = style_of(app);
        if (a) { a->level = fmin(1, fmax(0, v)); a->level_at = now(); }
    } else if (!strncmp(m, "reload", 6)) {
        restyle();
    }
}

/* ---------- Wayland ---------- */

static void layer_configure(void *data, struct zwlr_layer_surface_v1 *l, uint32_t serial, uint32_t w, uint32_t h)
{
    struct output *o = data;
    zwlr_layer_surface_v1_ack_configure(l, serial);
    if ((int)w != o->w || (int)h != o->h) {
        o->w = (int)w;
        o->h = (int)h;
        if (o->base) { cairo_surface_destroy(o->base); o->base = NULL; }
    }
    o->configured = 1;
    o->full = 1;
    if (!o->frame_pending) {
        render(o);
        commit(o);
    }
}
static void layer_closed(void *data, struct zwlr_layer_surface_v1 *l) { ((struct output *)data)->configured = 0; }
static const struct zwlr_layer_surface_v1_listener layer_listener = {.configure = layer_configure, .closed = layer_closed};

static void map_output(struct output *o)
{
    if (o->surface || !L.layer_shell || !L.cfg.enabled) return;
    o->surface = wl_compositor_create_surface(L.compositor);
    o->layer = zwlr_layer_shell_v1_get_layer_surface(L.layer_shell, o->surface, o->wl, ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND, "nexa-living");
    zwlr_layer_surface_v1_add_listener(o->layer, &layer_listener, o);
    zwlr_layer_surface_v1_set_anchor(o->layer, 15);
    zwlr_layer_surface_v1_set_exclusive_zone(o->layer, -1);
    zwlr_layer_surface_v1_set_keyboard_interactivity(o->layer, 0);
    wl_surface_commit(o->surface);
}

static void unmap_output(struct output *o)
{
    if (o->layer) zwlr_layer_surface_v1_destroy(o->layer);
    if (o->surface) wl_surface_destroy(o->surface);
    o->layer = NULL;
    o->surface = NULL;
    o->configured = o->frame_pending = 0;
    for (int i = 0; i < 2; i++) free_buffer(&o->buf[i]);
    if (o->base) cairo_surface_destroy(o->base);
    o->base = NULL;
    region_free(&o->desk);
}

static void out_geometry(void *d, struct wl_output *w, int32_t x, int32_t y, int32_t pw, int32_t ph, int32_t sp,
                         const char *mk, const char *md, int32_t tr) {}
static void out_mode(void *d, struct wl_output *w, uint32_t f, int32_t wd, int32_t ht, int32_t r) {}
static void out_done(void *d, struct wl_output *w) {}
static void out_scale(void *d, struct wl_output *w, int32_t f) { ((struct output *)d)->scale = f; }
static void out_name(void *d, struct wl_output *w, const char *n) { snprintf(((struct output *)d)->name, 32, "%s", n); }
static void out_desc(void *d, struct wl_output *w, const char *n) {}
static const struct wl_output_listener output_listener = {
    .geometry = out_geometry, .mode = out_mode, .done = out_done, .scale = out_scale, .name = out_name, .description = out_desc};

static struct output *output_of(struct wl_surface *s)
{
    for (int i = 0; i < L.nout; i++) if (L.out[i].surface == s) return &L.out[i];
    return NULL;
}
static void ptr_enter(void *d, struct wl_pointer *p, uint32_t serial, struct wl_surface *s, wl_fixed_t x, wl_fixed_t y)
{
    L.ptr_out = output_of(s);
    L.px = wl_fixed_to_double(x);
    L.py = wl_fixed_to_double(y);
}
static void ptr_leave(void *d, struct wl_pointer *p, uint32_t serial, struct wl_surface *s) { L.ptr_out = NULL; }
static void ptr_motion(void *d, struct wl_pointer *p, uint32_t t, wl_fixed_t x, wl_fixed_t y)
{
    L.px = wl_fixed_to_double(x);
    L.py = wl_fixed_to_double(y);
}
static void ptr_button(void *d, struct wl_pointer *p, uint32_t serial, uint32_t t, uint32_t button, uint32_t state)
{
    if (state != WL_POINTER_BUTTON_STATE_PRESSED || !L.ptr_out || L.cfg.reaction <= 0) return;
    int slot = 0;
    for (int i = 0; i < MAX_RIPPLES; i++) if (L.rip[i].t0 < L.rip[slot].t0) slot = i;
    L.rip[slot] = (struct ripple){.x = L.px, .y = L.py, .t0 = now() - L.t0, .desk = 1, .o = L.ptr_out};
}
static void ptr_axis(void *d, struct wl_pointer *p, uint32_t t, uint32_t a, wl_fixed_t v) {}
static void ptr_frame(void *d, struct wl_pointer *p) {}
static void ptr_axis_source(void *d, struct wl_pointer *p, uint32_t s) {}
static void ptr_axis_stop(void *d, struct wl_pointer *p, uint32_t t, uint32_t a) {}
static void ptr_axis_discrete(void *d, struct wl_pointer *p, uint32_t a, int32_t v) {}
static const struct wl_pointer_listener pointer_listener = {
    .enter = ptr_enter, .leave = ptr_leave, .motion = ptr_motion, .button = ptr_button, .axis = ptr_axis,
    .frame = ptr_frame, .axis_source = ptr_axis_source, .axis_stop = ptr_axis_stop, .axis_discrete = ptr_axis_discrete};

static void seat_caps(void *d, struct wl_seat *s, uint32_t caps)
{
    if ((caps & WL_SEAT_CAPABILITY_POINTER) && !L.pointer) {
        L.pointer = wl_seat_get_pointer(s);
        wl_pointer_add_listener(L.pointer, &pointer_listener, NULL);
    }
}
static void seat_name(void *d, struct wl_seat *s, const char *n) {}
static const struct wl_seat_listener seat_listener = {.capabilities = seat_caps, .name = seat_name};

static void reg_global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver)
{
    if (!strcmp(iface, wl_compositor_interface.name))
        L.compositor = wl_registry_bind(r, name, &wl_compositor_interface, ver < 4 ? ver : 4);
    else if (!strcmp(iface, wl_shm_interface.name))
        L.shm = wl_registry_bind(r, name, &wl_shm_interface, 1);
    else if (!strcmp(iface, wl_seat_interface.name) && !L.seat) {
        L.seat = wl_registry_bind(r, name, &wl_seat_interface, ver < 5 ? ver : 5);
        wl_seat_add_listener(L.seat, &seat_listener, NULL);
    } else if (!strcmp(iface, zwlr_layer_shell_v1_interface.name))
        L.layer_shell = wl_registry_bind(r, name, &zwlr_layer_shell_v1_interface, ver < 3 ? ver : 3);
    else if (!strcmp(iface, wl_output_interface.name) && L.nout < MAX_OUT) {
        struct output *o = &L.out[L.nout++];
        memset(o, 0, sizeof *o);
        o->global = name;
        o->scale = 1;
        for (int i = 0; i < 2; i++) pixman_region32_init(&o->buf[i].last);
        o->wl = wl_registry_bind(r, name, &wl_output_interface, ver < 4 ? ver : 4);
        wl_output_add_listener(o->wl, &output_listener, o);
        if (L.compositor) map_output(o);
    }
}
static void reg_remove(void *d, struct wl_registry *r, uint32_t name)
{
    for (int i = 0; i < L.nout; i++)
        if (L.out[i].global == name) {
            unmap_output(&L.out[i]);
            wl_output_destroy(L.out[i].wl);
            L.out[i] = L.out[--L.nout];
            return;
        }
}
static const struct wl_registry_listener registry_listener = {.global = reg_global, .global_remove = reg_remove};

/* ---------- main ---------- */

static void setup_paths(void)
{
    const char *run = getenv("XDG_RUNTIME_DIR"), *cfg = getenv("XDG_CONFIG_HOME"), *home = getenv("HOME");
    snprintf(L.run, sizeof L.run, "%s", run ? run : "/tmp");
    if (cfg && *cfg) snprintf(L.cfg_path, sizeof L.cfg_path, "%s/nexa/magic.json", cfg);
    else snprintf(L.cfg_path, sizeof L.cfg_path, "%s/.config/nexa/magic.json", home ? home : "/");
    if (cfg && *cfg) snprintf(L.strata_path, sizeof L.strata_path, "%s/nexa/strata.conf", cfg);
    else snprintf(L.strata_path, sizeof L.strata_path, "%s/.config/nexa/strata.conf", home ? home : "/");
    snprintf(L.sock_path, sizeof L.sock_path, "%s/nexa-living.sock", L.run);
    snprintf(L.stats_path, sizeof L.stats_path, "%s/nexa-living.stats", L.run);
}

static void open_app_socket(void)
{
    L.app_fd = socket(AF_UNIX, SOCK_DGRAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0);
    if (L.app_fd < 0) return;
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    snprintf(a.sun_path, sizeof a.sun_path, "%.107s", L.sock_path);
    unlink(L.sock_path);
    mode_t old = umask(0077);
    if (bind(L.app_fd, (struct sockaddr *)&a, sizeof a) < 0) { close(L.app_fd); L.app_fd = -1; }
    umask(old);
}

/* Watch the settings folder: Settings saves by rename, so watch the dir. */
static void watch_config(void)
{
    L.ino_fd = inotify_init1(IN_CLOEXEC | IN_NONBLOCK);
    if (L.ino_fd < 0) return;
    char dir[512];
    snprintf(dir, sizeof dir, "%s", L.cfg_path);
    char *slash = strrchr(dir, '/');
    if (slash) *slash = 0;
    mkdir(dir, 0700);
    inotify_add_watch(L.ino_fd, dir, IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE);
    /* interface/global.json holds reduced motion ("motion": "none"). */
    size_t len = strlen(dir);
    snprintf(dir + len, sizeof dir - len, "/interface");
    mkdir(dir, 0700);
    inotify_add_watch(L.ino_fd, dir, IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE);
}

int main(int argc, char **argv)
{
    if (argc > 1 && !strcmp(argv[1], "--styles")) {
        for (int i = 0; i < ST_COUNT; i++) printf("%s\t%s\n", style_names[i], style_titles[i]);
        return 0;
    }
    signal(SIGPIPE, SIG_IGN);
    setup_paths();
    load_colours();
    load_config();
    L.quality = -2;
    L.perf_quality = Q_HIGH;
    L.t0 = now();
    L.display = wl_display_connect(NULL);
    if (!L.display) {
        fprintf(stderr, "nexa-living: no Wayland display\n");
        return 1;
    }
    struct wl_registry *reg = wl_display_get_registry(L.display);
    wl_registry_add_listener(reg, &registry_listener, NULL);
    wl_display_roundtrip(L.display);
    if (!L.compositor || !L.shm || !L.layer_shell) {
        fprintf(stderr, "nexa-living: the compositor has no layer-shell\n");
        return 1;
    }
    wl_display_roundtrip(L.display);  /* output names */
    policy(now());
    for (int i = 0; i < L.nout; i++) map_output(&L.out[i]);
    open_app_socket();
    watch_config();
    connect_events();
    L.timer_fd = timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC | TFD_NONBLOCK);
    L.last_stats = now();

    for (;;) {
        /* Floating windows moved by hand send no event: look again 4x a
         * second while something animates, once a second otherwise. */
        {
            int moving = 0;
            for (int i = 0; i < L.nout; i++) moving |= L.cfg.enabled && L.out[i].configured && animating(&L.out[i]);
            if (now() - L.last_tree > (moving ? 0.25 : 1.0)) L.windows_dirty = 1;
        }
        if (L.windows_dirty) { refresh_windows(); L.last_tree = now(); }
        policy(now());
        if (L.cfg.enabled) {
            for (int i = 0; i < L.nout; i++) map_output(&L.out[i]);
            draw_all(0);
        } else {
            for (int i = 0; i < L.nout; i++) unmap_output(&L.out[i]);
        }
        write_stats();
        /* sleep until the next frame is due, or something happens */
        int any_anim = 0;
        for (int i = 0; i < L.nout && L.cfg.enabled; i++) any_anim |= L.out[i].configured && animating(&L.out[i]);
        int fps = L.quality < 0 ? 30 : FPS[L.quality];
        if (L.cfg.fps_cap > 0 && L.cfg.fps_cap < fps) fps = L.cfg.fps_cap;
        struct itimerspec its = {0};
        long ns = any_anim ? (long)(1e9 / fps) : 1000000000L;
        its.it_value.tv_sec = ns / 1000000000L;
        its.it_value.tv_nsec = ns % 1000000000L;
        timerfd_settime(L.timer_fd, 0, &its, NULL);

        while (wl_display_prepare_read(L.display) != 0) wl_display_dispatch_pending(L.display);
        wl_display_flush(L.display);
        struct pollfd fds[5] = {{.fd = wl_display_get_fd(L.display), .events = POLLIN},
                                {.fd = L.timer_fd, .events = POLLIN},
                                {.fd = L.ev_fd, .events = POLLIN},
                                {.fd = L.app_fd, .events = POLLIN},
                                {.fd = L.ino_fd, .events = POLLIN}};
        int n = poll(fds, 5, -1);
        if (n < 0 && errno != EINTR) { wl_display_cancel_read(L.display); break; }
        if (n > 0 && (fds[0].revents & POLLIN)) {
            if (wl_display_read_events(L.display) < 0) break;
        } else {
            wl_display_cancel_read(L.display);
        }
        if (n > 0 && (fds[0].revents & (POLLERR | POLLHUP))) break;
        if (wl_display_dispatch_pending(L.display) < 0) break;
        uint64_t ticks;
        if (n > 0 && (fds[1].revents & POLLIN) && read(L.timer_fd, &ticks, sizeof ticks) < 0) {}
        if (n > 0 && L.ev_fd >= 0 && (fds[2].revents & (POLLIN | POLLHUP))) read_events();
        if (n > 0 && L.app_fd >= 0 && (fds[3].revents & POLLIN)) app_message();
        if (n > 0 && L.ino_fd >= 0 && (fds[4].revents & POLLIN)) {
            char ib[4096];
            while (read(L.ino_fd, ib, sizeof ib) > 0) {}
            restyle();
        }
        if (L.ev_fd < 0 && L.ipc_kind) {
            /* the compositor's event stream closed: poll the windows instead */
            static double last;
            if (now() - last > 1) { last = now(); L.windows_dirty = 1; connect_events(); }
        }
    }
    unlink(L.sock_path);
    return 0;
}
