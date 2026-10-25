/* nexa-locker: the NexaOS lock screen.
 *
 * A Wayland client for ext-session-lock-v1 (Sway and Hyprland both have
 * it). Once the compositor confirms the lock, only this program's surfaces
 * are shown and only they get input, until it unlocks. If this program
 * dies, the compositor keeps the session locked; the watchdog process (see
 * main) then starts swaylock to take the lock over, so a crash never opens
 * the screen and never leaves the user stuck.
 *
 * The password is checked by PAM (service "nexa-lock": pam_unix through its
 * setgid unix_chkpwd helper) in a forked child, so the screen keeps moving
 * while it checks. The typed password sits in mlock'ed memory and is wiped
 * after every attempt; the process is not dumpable (no core files, no
 * ptrace from other programs of the same user).
 *
 * What's on screen: the blurred wallpaper under a hexagon grid that ripples
 * with each key, the clock inside a hexagon whose edge traces the seconds,
 * live readings (CPU, memory, battery, temperature, network, uptime), a
 * short event log, Caps Lock and keyboard-layout hints, and Sleep / Restart
 * / Shut down buttons (through nexa-brokerd; a second click confirms).
 *
 * Switch person: everyone else who may use this computer is a hexagon under
 * the password field (click, or Tab then Enter). The screen dissolves into
 * the honeycomb from the middle out, the other person's hexagon warps in,
 * and nexa-sessiond puts their desktop on screen. --switch-to NAME does the
 * same from an unlocked desktop (Power, Settings > Users): it locks, takes a
 * snapshot of the desktop first (wlr-screencopy) so that is what dissolves.
 * Coming back (nexa-sessiond sends SIGUSR2) or a new desktop (--boot) plays
 * the reverse: the person's hexagon grows into the clock as the honeycomb
 * reassembles. "motion": "none" in Interface DNA (~/.config/nexa/interface/
 * global.json) skips both.
 *
 *   nexa-locker [--daemonize] [--boot] [--switch-to NAME]
 *   nexa-locker --preview W H FILE.png [idle|typing|checking|wrong|caps|live|boot|people|depart:P|arrive:P]
 *
 * --daemonize returns once the screen is locked (exit 0) or couldn't be
 * locked (exit 2), so callers can suspend right after. Theme colours come
 * from ~/.config/nexa/lock.conf (written by nexa-theme).
 * Exit codes: 0 unlocked, 2 couldn't lock, 3 can't switch to that person
 * (nothing locked), 1 other errors.
 */
#define _GNU_SOURCE
#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <glob.h>
#include <linux/input-event-codes.h>
#include <math.h>
#include <poll.h>
#include <pwd.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/prctl.h>
#include <sys/signalfd.h>
#include <sys/socket.h>
#include <sys/timerfd.h>
#include <sys/un.h>
#include <sys/utsname.h>
#include <sys/wait.h>
#include <syslog.h>
#include <time.h>
#include <unistd.h>

#include <cairo.h>
#include <pango/pangocairo.h>
#include <security/pam_appl.h>
#include <wayland-client.h>
#include <wayland-cursor.h>
#include <xkbcommon/xkbcommon.h>

#include "ext-session-lock-v1-client-protocol.h"
#include "wlr-screencopy-unstable-v1-client-protocol.h"

#define MAX_PW 256
#define LOG_LINES 5
#define MAX_RIPPLES 10
#define MAX_PEOPLE 6
#define DEPART_S 1.3
#define ARRIVE_S 1.2

typedef struct { double r, g, b; } rgb;

struct theme {
    rgb bg, surface, text, muted, accent, accent2, crit, ok, warn;
    char wallpaper[512];
};

struct telemetry {
    double cpu;                        /* percent; < 0 while unknown */
    unsigned long long cpu_total, cpu_idle;
    double mem_used, mem_total;        /* GiB */
    int bat;                           /* percent; -1 without a battery */
    char bat_state[24];
    double temp;                       /* deg C; < 0 unknown */
    char net[48];
    int net_level;                     /* 0..100 for the meter; -1 none */
    long uptime;
    char kernel[64];
};

struct buffer {
    struct wl_buffer *wl;
    void *data;
    size_t size;
    int w, h, busy;
    cairo_surface_t *cs;
};

struct rect { double x, y, w, h; };

/* Someone else who may use this computer (from nexa-sessiond). */
struct person { char name[64], display[64]; int signed_in; };

struct output {
    struct wl_list link;
    struct wl_output *wl;
    uint32_t name;
    int scale;
    struct wl_surface *surface;
    struct ext_session_lock_surface_v1 *lock_surface;
    int width, height, configured;
    struct buffer buf[2];
    cairo_surface_t *bg;
    int bg_w, bg_h;
    struct rect power[3];
    struct rect people[MAX_PEOPLE];
    cairo_surface_t *scene;   /* the lock screen, drawn here first during a transition */
    cairo_surface_t *snap;    /* the desktop just before locking (--switch-to) */
    struct zwlr_screencopy_frame_v1 *frame;
    struct buffer cap;
    uint32_t cap_format, cap_stride, cap_flags;
    int cap_state;            /* 1 waiting, 2 done, -1 failed */
};

enum { P_SLEEP, P_RESTART, P_OFF };
enum { RP_KEY, RP_WRONG, RP_GOOD };
/* Transitions: leaving for someone else's desktop, the black screen while
 * it's theirs, and arriving (a new desktop, or back to this one). */
enum { TR_NONE, TR_DEPART, TR_AWAY, TR_ARRIVE };

struct ripple { double t0; int kind; };

static struct app {
    struct wl_display *display;
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    struct wl_seat *seat;
    struct wl_keyboard *keyboard;
    struct wl_pointer *pointer;
    struct ext_session_lock_manager_v1 *lock_manager;
    struct ext_session_lock_v1 *lock;
    struct zwlr_screencopy_manager_v1 *screencopy;
    uint32_t screencopy_version;
    struct wl_list outputs;
    struct xkb_context *xkb;
    struct xkb_keymap *keymap;
    struct xkb_state *xkb_state;
    struct wl_cursor_theme *cursor_theme;
    struct wl_surface *cursor_surface;
    struct output *pointer_output;
    double px, py;
    int hover;
    int locked, running, exit_code, redraw;
    int ready_fd, watch_fd;
    char *pw;
    size_t pw_len;
    int pw_chars;
    int checking, pam_fd;
    pid_t pam_pid;
    double wrong_at, unlock_at, start_at, cooldown_until, typed_at, armed_at;
    int fails, caps, boot, live, armed;
    char layout[32];
    struct ripple ripples[MAX_RIPPLES];
    char log[LOG_LINES][96];
    int nlog;
    struct theme th;
    struct telemetry tm;
    char user[64], display_name[64], host[64];
    const char *pam_service;
    int timer_fd, repeat_fd;
    int32_t repeat_rate, repeat_delay;
    xkb_keysym_t repeat_sym;
    uint32_t repeat_key;
    char repeat_utf8[16];
    int preview;
    double preview_now;
    /* Switch person */
    struct person people[MAX_PEOPLE];
    int npeople, focus;            /* focus: -1 the password field, else a person */
    int tr, tr_back;
    double tr_at, sw_error_at;
    char tr_name[64], tr_display[64], switch_to[64], sw_error[120];
    int sw_fd, sig_fd;
    pid_t sw_pid;
} A = {.ready_fd = -1, .watch_fd = -1, .pam_fd = -1, .hover = -1, .armed = -1, .timer_fd = -1, .repeat_fd = -1,
       .focus = -1, .sw_fd = -1, .sig_fd = -1};

/* ---------- small helpers ---------- */

static double now_s(void) {
    if (A.preview) return A.preview_now;
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

static double clampd(double v, double lo, double hi) { return v < lo ? lo : v > hi ? hi : v; }
static double sq(double v) { return v * v; }

static rgb hex_rgb(const char *s, rgb fallback) {
    unsigned r, g, b;
    if (s && *s == '#') s++;
    if (!s || strlen(s) < 6 || sscanf(s, "%2x%2x%2x", &r, &g, &b) != 3) return fallback;
    return (rgb){r / 255.0, g / 255.0, b / 255.0};
}

static void set_rgb(cairo_t *cr, rgb c, double a) { cairo_set_source_rgba(cr, c.r, c.g, c.b, a); }

static rgb mix(rgb a, rgb b, double t) {
    return (rgb){a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t};
}

static double smooth(double x) { x = clampd(x, 0, 1); return x * x * (3 - 2 * x); }
static double ease_back(double x) {
    x = clampd(x, 0, 1) - 1;
    return 1 + 2.70158 * x * x * x + 1.70158 * x * x;
}

static void add_log(const char *fmt, ...) {
    char msg[80];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(msg, sizeof msg, fmt, ap);
    va_end(ap);
    time_t t = time(NULL);
    struct tm tmv;
    localtime_r(&t, &tmv);
    if (A.nlog == LOG_LINES) {
        memmove(A.log[0], A.log[1], sizeof(A.log[0]) * (LOG_LINES - 1));
        A.nlog--;
    }
    snprintf(A.log[A.nlog++], sizeof A.log[0], "%02d:%02d:%02d  %s", tmv.tm_hour, tmv.tm_min, tmv.tm_sec, msg);
    A.redraw = 1;
}

static void add_ripple(int kind) {
    double t = now_s();
    int slot = 0;
    for (int i = 0; i < MAX_RIPPLES; i++) {
        if (A.ripples[i].t0 <= 0 || t - A.ripples[i].t0 > 1.6) { slot = i; break; }
        if (A.ripples[i].t0 < A.ripples[slot].t0) slot = i;
    }
    A.ripples[slot] = (struct ripple){t, kind};
    A.redraw = 1;
}

/* ---------- theme and system readings ---------- */

static void load_theme(void) {
    struct theme *t = &A.th;
    /* Horizon, the default theme, until the file says otherwise. */
    t->bg = hex_rgb("#080c1e", t->bg);
    t->surface = hex_rgb("#141c3a", t->surface);
    t->text = hex_rgb("#e6ecff", t->text);
    t->muted = hex_rgb("#8f9bc4", t->muted);
    t->accent = hex_rgb("#4da3ff", t->accent);
    t->accent2 = hex_rgb("#8a6bff", t->accent2);
    t->crit = hex_rgb("#ff5c7a", t->crit);
    t->ok = hex_rgb("#5ee6a8", t->ok);
    t->warn = hex_rgb("#ffb547", t->warn);
    snprintf(t->wallpaper, sizeof t->wallpaper, "/usr/share/backgrounds/nexaos/wallpaper.png");

    const char *home = getenv("HOME");
    char path[512];
    const char *cfg = getenv("XDG_CONFIG_HOME");
    if (cfg && *cfg) snprintf(path, sizeof path, "%s/nexa/lock.conf", cfg);
    else snprintf(path, sizeof path, "%s/.config/nexa/lock.conf", home ? home : "/");
    FILE *f = fopen(path, "r");
    if (!f) return;
    char line[600];
    while (fgets(line, sizeof line, f)) {
        line[strcspn(line, "\r\n")] = 0;
        char *eq = strchr(line, '=');
        if (!eq || line[0] == '#') continue;
        *eq = 0;
        const char *k = line, *v = eq + 1;
        if (!strcmp(k, "bg")) t->bg = hex_rgb(v, t->bg);
        else if (!strcmp(k, "surface")) t->surface = hex_rgb(v, t->surface);
        else if (!strcmp(k, "text")) t->text = hex_rgb(v, t->text);
        else if (!strcmp(k, "muted")) t->muted = hex_rgb(v, t->muted);
        else if (!strcmp(k, "accent")) t->accent = hex_rgb(v, t->accent);
        else if (!strcmp(k, "accent2")) t->accent2 = hex_rgb(v, t->accent2);
        else if (!strcmp(k, "crit")) t->crit = hex_rgb(v, t->crit);
        else if (!strcmp(k, "ok")) t->ok = hex_rgb(v, t->ok);
        else if (!strcmp(k, "warn")) t->warn = hex_rgb(v, t->warn);
        else if (!strcmp(k, "wallpaper") && *v) snprintf(t->wallpaper, sizeof t->wallpaper, "%s", v);
    }
    fclose(f);
}

static int read_line(const char *path, char *out, size_t n) {
    FILE *f = fopen(path, "r");
    if (!f) return -1;
    int ok = fgets(out, (int)n, f) != NULL;
    fclose(f);
    if (!ok) return -1;
    out[strcspn(out, "\n")] = 0;
    return 0;
}

static void read_telemetry(void) {
    struct telemetry *t = &A.tm;
    char buf[256];

    FILE *f = fopen("/proc/stat", "r");
    if (f) {
        unsigned long long v[8] = {0};
        if (fscanf(f, "cpu %llu %llu %llu %llu %llu %llu %llu %llu", &v[0], &v[1], &v[2], &v[3], &v[4], &v[5], &v[6], &v[7]) >= 4) {
            unsigned long long total = 0, idle = v[3] + v[4];
            for (int i = 0; i < 8; i++) total += v[i];
            if (t->cpu_total && total > t->cpu_total)
                t->cpu = 100.0 * (1.0 - (double)(idle - t->cpu_idle) / (double)(total - t->cpu_total));
            t->cpu_total = total;
            t->cpu_idle = idle;
        }
        fclose(f);
    }

    f = fopen("/proc/meminfo", "r");
    if (f) {
        unsigned long long total = 0, avail = 0, val;
        char key[64];
        while (fscanf(f, "%63s %llu kB\n", key, &val) == 2) {
            if (!strcmp(key, "MemTotal:")) total = val;
            else if (!strcmp(key, "MemAvailable:")) avail = val;
        }
        fclose(f);
        t->mem_total = total / 1048576.0;
        t->mem_used = (total - avail) / 1048576.0;
    }

    t->bat = -1;
    glob_t g;
    if (glob("/sys/class/power_supply/BAT*/capacity", 0, NULL, &g) == 0 && g.gl_pathc) {
        if (read_line(g.gl_pathv[0], buf, sizeof buf) == 0) t->bat = atoi(buf);
        char sp[300];
        snprintf(sp, sizeof sp, "%.*s/status", (int)(strlen(g.gl_pathv[0]) - strlen("/capacity")), g.gl_pathv[0]);
        if (read_line(sp, buf, sizeof buf) == 0) snprintf(t->bat_state, sizeof t->bat_state, "%.23s", buf);
    }
    globfree(&g);

    t->temp = -1;
    if (glob("/sys/class/thermal/thermal_zone*/type", 0, NULL, &g) == 0) {
        double best = -1, pkg = -1;
        for (size_t i = 0; i < g.gl_pathc; i++) {
            char type[64], tp[300];
            if (read_line(g.gl_pathv[i], type, sizeof type) != 0) continue;
            snprintf(tp, sizeof tp, "%.*s/temp", (int)(strlen(g.gl_pathv[i]) - strlen("/type")), g.gl_pathv[i]);
            if (read_line(tp, buf, sizeof buf) != 0) continue;
            double c = atof(buf) / 1000.0;
            if (c <= 0 || c > 150) continue;
            if (!strcmp(type, "x86_pkg_temp")) pkg = c;
            if (c > best) best = c;
        }
        t->temp = pkg > 0 ? pkg : best;
    }
    globfree(&g);

    snprintf(t->net, sizeof t->net, "offline");
    t->net_level = -1;
    DIR *d = opendir("/sys/class/net");
    if (d) {
        struct dirent *e;
        while ((e = readdir(d))) {
            if (e->d_name[0] == '.' || !strcmp(e->d_name, "lo")) continue;
            char p[300];
            snprintf(p, sizeof p, "/sys/class/net/%s/operstate", e->d_name);
            if (read_line(p, buf, sizeof buf) != 0 || strcmp(buf, "up") != 0) continue;
            snprintf(p, sizeof p, "/sys/class/net/%s/wireless", e->d_name);
            if (access(p, F_OK) == 0) {
                int dbm = 0;
                FILE *w = fopen("/proc/net/wireless", "r");
                if (w) {
                    char l[256];
                    size_t n = strlen(e->d_name);
                    while (fgets(l, sizeof l, w)) {
                        char *s = l;
                        while (*s == ' ') s++;
                        if (!strncmp(s, e->d_name, n) && s[n] == ':') {
                            float q, lv;
                            if (sscanf(s + n + 1, "%*s %f %f", &q, &lv) == 2) dbm = (int)lv;
                        }
                    }
                    fclose(w);
                }
                if (dbm < 0) {
                    snprintf(t->net, sizeof t->net, "%.15s %d dBm", e->d_name, dbm);
                    t->net_level = (int)clampd((dbm + 90) * 100.0 / 60.0, 0, 100);
                } else {
                    snprintf(t->net, sizeof t->net, "%.15s linked", e->d_name);
                    t->net_level = 60;
                }
                break;
            }
            snprintf(t->net, sizeof t->net, "%.15s wired", e->d_name);
            t->net_level = 100;
        }
        closedir(d);
    }

    if (read_line("/proc/uptime", buf, sizeof buf) == 0) t->uptime = atol(buf);
    if (!t->kernel[0]) {
        struct utsname u;
        if (uname(&u) == 0) snprintf(t->kernel, sizeof t->kernel, "%.63s", u.release);
    }
}

/* ---------- drawing ---------- */

static void hex_path(cairo_t *cr, double cx, double cy, double r) {
    cairo_new_sub_path(cr);
    for (int i = 0; i < 6; i++) {
        double a = -M_PI / 2 + i * M_PI / 3;
        cairo_line_to(cr, cx + r * cos(a), cy + r * sin(a));
    }
    cairo_close_path(cr);
}

/* A point on a pointy-top hexagon's edge, t in [0, 1), clockwise from the top. */
static void hex_point(double cx, double cy, double r, double t, double *x, double *y) {
    t -= floor(t);
    int seg = (int)(t * 6);
    double f = t * 6 - seg;
    double a0 = -M_PI / 2 + seg * M_PI / 3, a1 = a0 + M_PI / 3;
    *x = cx + r * ((1 - f) * cos(a0) + f * cos(a1));
    *y = cy + r * ((1 - f) * sin(a0) + f * sin(a1));
}

static void rounded_rect(cairo_t *cr, double x, double y, double w, double h, double r) {
    cairo_new_sub_path(cr);
    cairo_arc(cr, x + w - r, y + r, r, -M_PI / 2, 0);
    cairo_arc(cr, x + w - r, y + h - r, r, 0, M_PI / 2);
    cairo_arc(cr, x + r, y + h - r, r, M_PI / 2, M_PI);
    cairo_arc(cr, x + r, y + r, r, M_PI, 3 * M_PI / 2);
    cairo_close_path(cr);
}

/* Text centred vertically on y; align 0 left, 1 centre, 2 right. Returns its width. */
static double text(cairo_t *cr, const char *font, double px, double spacing, const char *s,
                   double x, double y, int align, rgb c, double alpha) {
    PangoLayout *l = pango_cairo_create_layout(cr);
    PangoFontDescription *d = pango_font_description_from_string(font);
    pango_font_description_set_absolute_size(d, px * PANGO_SCALE);
    pango_layout_set_font_description(l, d);
    if (spacing > 0) {
        PangoAttrList *at = pango_attr_list_new();
        pango_attr_list_insert(at, pango_attr_letter_spacing_new((int)(spacing * PANGO_SCALE)));
        pango_layout_set_attributes(l, at);
        pango_attr_list_unref(at);
    }
    pango_layout_set_text(l, s, -1);
    int w, h;
    pango_layout_get_pixel_size(l, &w, &h);
    double xx = align == 1 ? x - w / 2.0 : align == 2 ? x - w : x;
    set_rgb(cr, c, alpha);
    cairo_move_to(cr, xx, y - h / 2.0);
    pango_cairo_show_layout(cr, l);
    pango_font_description_free(d);
    g_object_unref(l);
    return w;
}

/* Separable box blur on an ARGB32 image, in place. */
static void box_blur(cairo_surface_t *s, int radius, int passes) {
    cairo_surface_flush(s);
    int w = cairo_image_surface_get_width(s), h = cairo_image_surface_get_height(s);
    int stride = cairo_image_surface_get_stride(s);
    unsigned char *px = cairo_image_surface_get_data(s);
    int n = w > h ? w : h;
    unsigned char *tmp = malloc((size_t)n * 4);
    if (!tmp) return;
    for (int p = 0; p < passes; p++) {
        for (int dir = 0; dir < 2; dir++) {
            int lines = dir ? w : h, len = dir ? h : w;
            for (int li = 0; li < lines; li++) {
                for (int i = 0; i < len; i++) {
                    int sum[4] = {0}, cnt = 0;
                    for (int k = -radius; k <= radius; k++) {
                        int j = i + k;
                        if (j < 0 || j >= len) continue;
                        unsigned char *q = dir ? px + j * stride + li * 4 : px + li * stride + j * 4;
                        for (int c = 0; c < 4; c++) sum[c] += q[c];
                        cnt++;
                    }
                    for (int c = 0; c < 4; c++) tmp[i * 4 + c] = (unsigned char)(sum[c] / cnt);
                }
                for (int i = 0; i < len; i++) {
                    unsigned char *q = dir ? px + i * stride + li * 4 : px + li * stride + i * 4;
                    memcpy(q, tmp + i * 4, 4);
                }
            }
        }
    }
    free(tmp);
    cairo_surface_mark_dirty(s);
}

static double unit_for(double H) { return clampd(H / 1000.0, 0.85, 3.0); }

/* The hexagon grid: every cell, in logical coordinates. */
#define GRID_EACH(W, H, U, X, Y)                                                        \
    for (int _row = -1; _row * 1.5 * 30 * (U) < (H) + 30 * (U); _row++)                  \
        for (int _col = -1; _col * sqrt(3) * 30 * (U) < (W) + 60 * (U); _col++)          \
            for (double X = _col * sqrt(3) * 30 * (U) + ((_row & 1) ? sqrt(3) * 15 * (U) : 0), \
                        Y = _row * 1.5 * 30 * (U), _once = 1; _once; _once = 0)

static cairo_surface_t *make_background(int BW, int BH, int scale) {
    cairo_surface_t *out = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, BW, BH);
    cairo_t *cr = cairo_create(out);
    double W = (double)BW / scale, H = (double)BH / scale, U = unit_for(H);
    const struct theme *t = &A.th;

    cairo_pattern_t *lin = cairo_pattern_create_linear(0, 0, BW, BH);
    cairo_pattern_add_color_stop_rgb(lin, 0, t->bg.r, t->bg.g, t->bg.b);
    rgb mid = mix(t->bg, t->surface, 0.7);
    cairo_pattern_add_color_stop_rgb(lin, 1, mid.r, mid.g, mid.b);
    cairo_set_source(cr, lin);
    cairo_paint(cr);
    cairo_pattern_destroy(lin);

    cairo_surface_t *wp = cairo_image_surface_create_from_png(t->wallpaper);
    if (cairo_surface_status(wp) == CAIRO_STATUS_SUCCESS) {
        int sw = BW / 14 + 1, sh = BH / 14 + 1;
        int ww = cairo_image_surface_get_width(wp), wh = cairo_image_surface_get_height(wp);
        cairo_surface_t *small = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, sw, sh);
        cairo_t *c2 = cairo_create(small);
        double sc = fmax((double)sw / ww, (double)sh / wh);
        cairo_translate(c2, (sw - ww * sc) / 2, (sh - wh * sc) / 2);
        cairo_scale(c2, sc, sc);
        cairo_set_source_surface(c2, wp, 0, 0);
        cairo_pattern_set_filter(cairo_get_source(c2), CAIRO_FILTER_GOOD);
        cairo_paint(c2);
        cairo_destroy(c2);
        box_blur(small, 2, 2);
        cairo_save(cr);
        cairo_scale(cr, (double)BW / sw, (double)BH / sh);
        cairo_set_source_surface(cr, small, 0, 0);
        cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_BILINEAR);
        cairo_pattern_set_extend(cairo_get_source(cr), CAIRO_EXTEND_PAD);
        cairo_paint(cr);
        cairo_restore(cr);
        cairo_surface_destroy(small);
    }
    cairo_surface_destroy(wp);

    cairo_scale(cr, scale, scale);
    set_rgb(cr, t->bg, 0.58);
    cairo_paint(cr);

    double diag = hypot(W, H);
    cairo_pattern_t *vig = cairo_pattern_create_radial(W / 2, H * 0.42, diag * 0.15, W / 2, H * 0.42, diag * 0.7);
    cairo_pattern_add_color_stop_rgba(vig, 0, t->bg.r, t->bg.g, t->bg.b, 0);
    cairo_pattern_add_color_stop_rgba(vig, 1, t->bg.r, t->bg.g, t->bg.b, 0.75);
    cairo_set_source(cr, vig);
    cairo_paint(cr);
    cairo_pattern_destroy(vig);

    /* The resting grid, fading out towards the corners. */
    cairo_set_line_width(cr, 1.0 * U);
    GRID_EACH(W, H, U, x, y) {
        double d = hypot(x - W / 2, y - H * 0.42) / (diag * 0.5);
        double a = 0.07 * clampd(1.1 - d, 0, 1);
        if (a < 0.004) continue;
        hex_path(cr, x, y, 30 * U * 0.9);
        set_rgb(cr, t->accent, a);
        cairo_stroke(cr);
    }
    cairo_destroy(cr);
    return out;
}

/* ---------- people and switching ---------- */

/* One request to nexa-sessiond; its reply in out. -1 if it isn't there. */
static int session_request(const char *line, char *out, size_t n) {
    const char *path = getenv("NEXA_SESSION_SOCKET");
    if (!path || !*path) path = "/run/nexa-session.sock";
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    snprintf(a.sun_path, sizeof a.sun_path, "%s", path);
    struct timeval tv = {8, 0};
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    if (connect(fd, (struct sockaddr *)&a, sizeof a) < 0 || write(fd, line, strlen(line)) < 0) {
        close(fd);
        return -1;
    }
    shutdown(fd, SHUT_WR);
    size_t got = 0;
    ssize_t r;
    while (got + 1 < n && (r = read(fd, out + got, n - 1 - got)) > 0) got += (size_t)r;
    out[got] = 0;
    close(fd);
    return 0;
}

/* Everyone else who may use this computer, and who's signed in. */
static void load_people(void) {
    char buf[4096];
    A.npeople = 0;
    if (session_request("people\n", buf, sizeof buf) < 0 || strncmp(buf, "ok", 2)) return;
    char *save = NULL;
    for (char *line = strtok_r(buf, "\n", &save); line; line = strtok_r(NULL, "\n", &save)) {
        char *f[5], *s2 = NULL;
        int nf = 0;
        for (char *p = strtok_r(line, "\t", &s2); p && nf < 5; p = strtok_r(NULL, "\t", &s2)) f[nf++] = p;
        if (nf < 5 || !strcmp(f[0], A.user) || A.npeople >= MAX_PEOPLE) continue;
        struct person *p = &A.people[A.npeople++];
        snprintf(p->name, sizeof p->name, "%s", f[0]);
        snprintf(p->display, sizeof p->display, "%.20s", f[2]);
        p->signed_in = atoi(f[3]);
    }
    if (A.focus >= A.npeople) A.focus = -1;
}

/* A colour of their own, the same every time (FNV-1a of the name). */
static rgb person_colour(const char *name) {
    unsigned h = 2166136261u;
    for (const char *c = name; *c; c++) h = (h ^ (unsigned char)*c) * 16777619u;
    double hue = (h % 360) / 60.0, s = 0.55, v = 0.95, c = v * s, x = c * (1 - fabs(fmod(hue, 2) - 1)), m = v - c;
    rgb out = hue < 1 ? (rgb){c, x, 0} : hue < 2 ? (rgb){x, c, 0} : hue < 3 ? (rgb){0, c, x}
            : hue < 4 ? (rgb){0, x, c} : hue < 5 ? (rgb){x, 0, c} : (rgb){c, 0, x};
    return (rgb){out.r + m, out.g + m, out.b + m};
}

/* A person's hexagon: their colour, their initial. */
static void draw_avatar(cairo_t *cr, const char *name, const char *display, double cx, double cy, double r, double alpha) {
    if (alpha <= 0.01 || r <= 0.5) return;
    rgb c = person_colour(name), dark = mix(c, (rgb){0, 0, 0}, 0.45);
    cairo_pattern_t *g = cairo_pattern_create_linear(cx, cy - r, cx, cy + r);
    cairo_pattern_add_color_stop_rgba(g, 0, c.r, c.g, c.b, alpha);
    cairo_pattern_add_color_stop_rgba(g, 1, dark.r, dark.g, dark.b, alpha);
    hex_path(cr, cx, cy, r);
    cairo_set_source(cr, g);
    cairo_fill_preserve(cr);
    cairo_pattern_destroy(g);
    set_rgb(cr, (rgb){1, 1, 1}, alpha * 0.45);
    cairo_set_line_width(cr, fmax(1, r * 0.05));
    cairo_stroke(cr);
    char initial[8] = "";
    const unsigned char *d = (const unsigned char *)(display && *display ? display : name);
    size_t n = 1;
    while (d[n] && (d[n] & 0xC0) == 0x80 && n < 4) n++;
    memcpy(initial, d, n);
    if (n == 1) initial[0] = (char)toupper(initial[0]);
    text(cr, "Inter Semi-Bold", r * 0.85, 0, initial, cx, cy, 1, (rgb){1, 1, 1}, alpha);
}

/* Interface DNA's "motion": "none" means no transitions here either. */
static int reduce_motion(void) {
    char path[512], buf[4096];
    const char *cfg = getenv("XDG_CONFIG_HOME"), *home = getenv("HOME");
    if (cfg && *cfg) snprintf(path, sizeof path, "%s/nexa/interface/global.json", cfg);
    else snprintf(path, sizeof path, "%s/.config/nexa/interface/global.json", home ? home : "/");
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

static void begin_departure(const char *name, const char *display) {
    snprintf(A.tr_name, sizeof A.tr_name, "%s", name);
    snprintf(A.tr_display, sizeof A.tr_display, "%s", display);
    A.tr = TR_DEPART;
    A.tr_at = now_s() - (reduce_motion() ? DEPART_S : 0);
    A.focus = -1;
    A.armed = -1;
    A.sw_error[0] = 0;
    add_log("session: handing the screen to %s", name);
}

static void start_arrival(int back) {
    if (A.sw_pid > 0) return;  /* still waiting for the answer */
    A.tr = TR_ARRIVE;
    A.tr_back = back;
    A.tr_at = now_s() - (reduce_motion() ? ARRIVE_S : 0);
    load_people();
    if (back) add_log("session: back on screen");
    A.redraw = 1;
}

static void begin_switch(int i) {
    if (i < 0 || i >= A.npeople || A.checking || A.unlock_at > 0 || A.tr != TR_NONE) return;
    begin_departure(A.people[i].name, A.people[i].display);
}

/* After the departure: ask nexa-sessiond, off the event loop (starting a
 * desktop takes a moment); the screen stays dark meanwhile. */
static void send_switch(void) {
    int p[2];
    if (pipe2(p, O_CLOEXEC) < 0) { start_arrival(0); return; }
    pid_t pid = fork();
    if (pid == 0) {
        close(p[0]);
        char line[96], out[256];
        snprintf(line, sizeof line, "switch %s\n", A.tr_name);
        if (session_request(line, out, sizeof out) < 0)
            snprintf(out, sizeof out, "error Switching isn't available (nexa-sessiond isn't running)");
        if (write(p[1], out, strlen(out)) < 0) _exit(1);
        _exit(0);
    }
    close(p[1]);
    if (pid < 0) { close(p[0]); start_arrival(0); return; }
    A.sw_pid = pid;
    A.sw_fd = p[0];
    A.tr = TR_AWAY;
}

static void switch_done(void) {
    char out[256] = "";
    ssize_t r = read(A.sw_fd, out, sizeof out - 1);
    if (r > 0) out[r] = 0;
    close(A.sw_fd);
    A.sw_fd = -1;
    waitpid(A.sw_pid, NULL, 0);
    A.sw_pid = 0;
    out[strcspn(out, "\n")] = 0;
    if (!strncmp(out, "ok", 2) && strncmp(out, "ok already", 10)) {
        add_log("session: %s is on screen", A.tr_name);
        return;  /* dark until we're back (SIGUSR2) or a key is pressed */
    }
    if (strncmp(out, "ok", 2)) {
        snprintf(A.sw_error, sizeof A.sw_error, "%.119s", !strncmp(out, "error ", 6) ? out + 6 : "Couldn't switch");
        A.sw_error_at = now_s();
        add_log("session: couldn't switch to %s", A.tr_name);
    }
    start_arrival(0);
}

static void draw_power_icon(cairo_t *cr, int which, double cx, double cy, double s, rgb c, double a) {
    set_rgb(cr, c, a);
    cairo_set_line_width(cr, s * 0.16);
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
    cairo_new_path(cr);
    if (which == P_SLEEP) {
        /* a crescent: a disc with an offset disc cut out of it */
        cairo_push_group(cr);
        set_rgb(cr, c, 1);
        cairo_arc(cr, cx, cy, s * 0.5, 0, 2 * M_PI);
        cairo_fill(cr);
        cairo_set_operator(cr, CAIRO_OPERATOR_DEST_OUT);
        cairo_arc(cr, cx + s * 0.3, cy - s * 0.24, s * 0.44, 0, 2 * M_PI);
        cairo_fill(cr);
        cairo_pop_group_to_source(cr);
        cairo_paint_with_alpha(cr, a);
    } else if (which == P_RESTART) {
        cairo_arc(cr, cx, cy, s * 0.45, -M_PI * 0.35, M_PI * 1.45);
        cairo_stroke(cr);
        double ax = cx + s * 0.45 * cos(-M_PI * 0.35), ay = cy + s * 0.45 * sin(-M_PI * 0.35);
        cairo_move_to(cr, ax - s * 0.24, ay - s * 0.02);
        cairo_line_to(cr, ax + s * 0.02, ay + s * 0.02);
        cairo_line_to(cr, ax + s * 0.02, ay - s * 0.26);
        cairo_stroke(cr);
    } else {
        cairo_arc(cr, cx, cy, s * 0.45, -M_PI * 0.32, M_PI * 1.32);
        cairo_stroke(cr);
        cairo_move_to(cr, cx, cy - s * 0.58);
        cairo_line_to(cr, cx, cy - s * 0.08);
        cairo_stroke(cr);
    }
}

static void meter(cairo_t *cr, double x, double y, double U, double pct, rgb on, rgb off) {
    int lit = (int)lround(clampd(pct, 0, 100) / 10.0);
    for (int i = 0; i < 10; i++) {
        cairo_rectangle(cr, x + i * 9 * U, y - 4 * U, 6 * U, 8 * U);
        if (i < lit) set_rgb(cr, on, 0.9);
        else set_rgb(cr, off, 0.22);
        cairo_fill(cr);
    }
}

static int animating(void) {
    double t = now_s();
    if (A.tr == TR_DEPART || A.tr == TR_ARRIVE) return 1;
    if (A.checking || A.unlock_at > 0 || t - A.start_at < 1.4 || t - A.wrong_at < 0.7 || t - A.typed_at < 0.3)
        return 1;
    for (int i = 0; i < MAX_RIPPLES; i++)
        if (A.ripples[i].t0 > 0 && t - A.ripples[i].t0 < 1.5) return 1;
    return 0;
}

static void draw_scene(cairo_t *cr, struct output *o, double W, double H) {
    const struct theme *th = &A.th;
    double U = unit_for(H), t = now_s();
    double cx = W / 2, cy = H * 0.42, R = 170 * U;
    double diag = hypot(W, H);
    char buf[160];

    /* Ripples through the grid: every key, a wrong password, the unlock. */
    GRID_EACH(W, H, U, x, y) {
        double d = hypot(x - cx, y - cy), amp = 0;
        rgb col = th->accent;
        for (int i = 0; i < MAX_RIPPLES; i++) {
            const struct ripple *r = &A.ripples[i];
            double age = t - r->t0;
            if (r->t0 <= 0 || age < 0 || age > 1.5) continue;
            double speed = r->kind == RP_KEY ? 900 : 1300;
            double a = exp(-sq((d - age * speed * U) / (70 * U))) * (1 - age / 1.5) * (r->kind == RP_KEY ? 0.55 : 1);
            if (a > amp) {
                amp = a;
                col = r->kind == RP_WRONG ? th->crit : r->kind == RP_GOOD ? th->ok : th->accent;
            }
        }
        /* The opening: the grid lights up from the middle out. */
        double intro = t - A.start_at;
        if (intro >= 0 && intro < 1.4) {
            double a = exp(-sq((d - intro * diag * 0.6) / (90 * U))) * (1 - intro / 1.4) * 0.8;
            if (a > amp) { amp = a; col = th->accent2; }
        }
        if (amp < 0.03) continue;
        hex_path(cr, x, y, 30 * U * 0.9);
        set_rgb(cr, col, amp * 0.14);
        cairo_fill_preserve(cr);
        set_rgb(cr, col, amp * 0.55);
        cairo_set_line_width(cr, 1.2 * U);
        cairo_stroke(cr);
    }

    /* The core: a glass hexagon, the seconds running round its edge. */
    double shake = 0;
    if (t - A.wrong_at < 0.6) shake = sin((t - A.wrong_at) * 46) * exp(-(t - A.wrong_at) * 6) * 14 * U;
    hex_path(cr, cx, cy + 6 * U, R);
    set_rgb(cr, (rgb){0, 0, 0}, 0.25);
    cairo_fill(cr);
    hex_path(cr, cx, cy, R);
    set_rgb(cr, th->surface, 0.62);
    cairo_fill_preserve(cr);
    set_rgb(cr, th->accent, 0.35);
    cairo_set_line_width(cr, 1.5 * U);
    cairo_stroke(cr);
    hex_path(cr, cx, cy, R * 0.93);
    set_rgb(cr, th->text, 0.07);
    cairo_set_line_width(cr, 1 * U);
    cairo_stroke(cr);
    /* top-edge light */
    cairo_move_to(cr, cx + R * cos(-M_PI * 5 / 6), cy + R * sin(-M_PI * 5 / 6));
    cairo_line_to(cr, cx, cy - R);
    cairo_line_to(cr, cx + R * cos(-M_PI / 6), cy + R * sin(-M_PI / 6));
    set_rgb(cr, th->text, 0.22);
    cairo_set_line_width(cr, 1.2 * U);
    cairo_stroke(cr);

    time_t wall = time(NULL);
    struct tm tmv;
    localtime_r(&wall, &tmv);
    struct timespec rt;
    clock_gettime(CLOCK_REALTIME, &rt);
    double secs = tmv.tm_sec + (animating() ? rt.tv_nsec / 1e9 : 0);
    double head = secs / 60.0, spin = A.checking ? fmod(t * 0.9, 1.0) : -1;
    if (spin >= 0) head = spin;
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
    for (int i = 0; i < 24; i++) {
        double a0 = head - 0.22 * (i + 1) / 24.0, a1 = head - 0.22 * i / 24.0, x0, y0, x1, y1;
        if (spin < 0 && a1 < 0) break;
        hex_point(cx, cy, R, a0, &x0, &y0);
        hex_point(cx, cy, R, a1, &x1, &y1);
        cairo_move_to(cr, x0, y0);
        /* through the corner if the piece crosses one */
        if ((int)floor(a0 * 6) != (int)floor(a1 * 6)) {
            double xc, yc;
            hex_point(cx, cy, R, floor(a1 * 6) / 6.0, &xc, &yc);
            cairo_line_to(cr, xc, yc);
        }
        cairo_line_to(cr, x1, y1);
        set_rgb(cr, A.checking ? th->accent2 : th->accent, 0.95 * (1 - i / 24.0));
        cairo_set_line_width(cr, 3 * U);
        cairo_stroke(cr);
    }
    double hx, hy;
    hex_point(cx, cy, R, head, &hx, &hy);
    cairo_pattern_t *glow = cairo_pattern_create_radial(hx, hy, 0, hx, hy, 14 * U);
    cairo_pattern_add_color_stop_rgba(glow, 0, th->text.r, th->text.g, th->text.b, 0.9);
    cairo_pattern_add_color_stop_rgba(glow, 0.3, th->accent.r, th->accent.g, th->accent.b, 0.6);
    cairo_pattern_add_color_stop_rgba(glow, 1, th->accent.r, th->accent.g, th->accent.b, 0);
    cairo_set_source(cr, glow);
    cairo_arc(cr, hx, hy, 14 * U, 0, 2 * M_PI);
    cairo_fill(cr);
    cairo_pattern_destroy(glow);

    const char *tag = A.unlock_at > 0 ? "ACCESS GRANTED" : A.boot ? "WELCOME" : "SESSION LOCKED";
    text(cr, "JetBrains Mono Bold", 11 * U, 4 * U, tag, cx, cy - R * 0.42, 1,
         A.unlock_at > 0 ? th->ok : th->accent, 0.95);
    strftime(buf, sizeof buf, "%H:%M", &tmv);
    text(cr, "Inter Light", 92 * U, 0, buf, cx, cy - R * 0.06, 1, th->text, 1);
    strftime(buf, sizeof buf, "%A %e %B", &tmv);
    char *p = buf;
    while (*p == ' ') p++;
    for (char *q = p; *q; q++) if (q[0] == ' ' && q[1] == ' ') memmove(q, q + 1, strlen(q));
    text(cr, "Inter", 17 * U, 0, p, cx, cy + R * 0.30, 1, th->muted, 1);
    snprintf(buf, sizeof buf, "%02d", tmv.tm_sec);
    text(cr, "JetBrains Mono", 11 * U, 2 * U, buf, cx, cy + R * 0.52, 1, th->muted, 0.7);

    /* Who, and the password field. */
    double fy = cy + R + 34 * U;
    text(cr, "Inter Semi-Bold", 17 * U, 0, A.display_name, cx, fy, 1, th->text, 1);
    double fw = 340 * U, fh = 46 * U, fx = cx - fw / 2 + shake, fy0 = fy + 26 * U;
    rounded_rect(cr, fx, fy0, fw, fh, fh / 2);
    set_rgb(cr, th->surface, 0.72);
    cairo_fill_preserve(cr);
    rgb border = th->accent;
    double ba = 0.45;
    if (t - A.wrong_at < 1.2) { border = th->crit; ba = 0.9; }
    else if (A.checking) { border = th->accent2; ba = 0.6 + 0.3 * sin(t * 8); }
    else if (A.unlock_at > 0) { border = th->ok; ba = 0.9; }
    else if (A.pw_chars) ba = 0.75;
    set_rgb(cr, border, ba);
    cairo_set_line_width(cr, 1.5 * U);
    cairo_stroke(cr);

    /* a small padlock at the left */
    double lx = fx + 24 * U, ly = fy0 + fh / 2;
    rounded_rect(cr, lx - 7 * U, ly - 2 * U, 14 * U, 11 * U, 2 * U);
    set_rgb(cr, th->muted, 0.9);
    cairo_fill(cr);
    cairo_arc(cr, lx, ly - 2 * U, 4.5 * U, M_PI, 2 * M_PI);
    cairo_set_line_width(cr, 2 * U);
    cairo_stroke(cr);
    /* keyboard layout at the right */
    if (A.layout[0]) text(cr, "JetBrains Mono Bold", 10 * U, 1 * U, A.layout, fx + fw - 22 * U, ly, 2, th->muted, 0.85);

    if (A.checking) {
        for (int i = 0; i < 3; i++) {
            double ph = 0.5 + 0.5 * sin(t * 7 - i * 0.9);
            hex_path(cr, cx + shake + (i - 1) * 22 * U, ly, (5 + 2 * ph) * U);
            set_rgb(cr, th->accent2, 0.4 + 0.6 * ph);
            cairo_fill(cr);
        }
    } else if (A.pw_chars) {
        int shown = A.pw_chars > 16 ? 16 : A.pw_chars;
        double step = 16 * U, x0 = cx + shake - (shown - 1) * step / 2;
        for (int i = 0; i < shown; i++) {
            double r = 5.5 * U;
            if (i == shown - 1 && t - A.typed_at < 0.25) r *= 1 + 0.6 * (1 - (t - A.typed_at) / 0.25);
            hex_path(cr, x0 + i * step, ly, r);
            set_rgb(cr, i == shown - 1 ? th->accent : th->text, 0.95);
            cairo_fill(cr);
        }
    } else if (A.unlock_at <= 0) {
        text(cr, "Inter", 14 * U, 0, A.live ? "Press Enter to start" : "Password", cx + shake, ly, 1, th->muted, 0.85);
    }

    /* One line under the field: what's happening, or what to do. */
    rgb sc = th->muted;
    if (A.unlock_at > 0) { snprintf(buf, sizeof buf, "Unlocked"); sc = th->ok; }
    else if (A.checking) snprintf(buf, sizeof buf, "Checking...");
    else if (A.cooldown_until > t) {
        snprintf(buf, sizeof buf, "Too many tries. Wait %d s", (int)ceil(A.cooldown_until - t));
        sc = th->warn;
    } else if (t - A.wrong_at < 4) {
        if (A.fails > 1) snprintf(buf, sizeof buf, "Wrong password (%d tries)", A.fails);
        else snprintf(buf, sizeof buf, "Wrong password. Try again");
        sc = th->crit;
    } else if (A.sw_error[0] && t - A.sw_error_at < 8) { snprintf(buf, sizeof buf, "%s", A.sw_error); sc = th->warn; }
    else if (A.caps) { snprintf(buf, sizeof buf, "Caps Lock is on"); sc = th->warn; }
    else if (A.live) snprintf(buf, sizeof buf, "Live USB: this account has no password");
    else if (A.focus >= 0) snprintf(buf, sizeof buf, "Enter switches to %s  /  Esc goes back", A.people[A.focus].display);
    else if (A.npeople) snprintf(buf, sizeof buf, "Enter unlocks  /  Tab: switch person");
    else snprintf(buf, sizeof buf, "Enter unlocks  /  Esc clears");
    text(cr, "Inter", 13 * U, 0, buf, cx, fy0 + fh + 24 * U, 1, sc, 0.95);

    /* Switch person: everyone else who may use this computer. A dot: signed in. */
    if (A.npeople && A.unlock_at <= 0) {
        double py = fy0 + fh + 96 * U, step = 100 * U, x0 = cx - (A.npeople - 1) * step / 2;
        text(cr, "JetBrains Mono Bold", 9.5 * U, 3 * U, "SWITCH PERSON", cx, py - 40 * U, 1, th->muted, 0.7);
        for (int i = 0; i < A.npeople; i++) {
            double px = x0 + i * step;
            int hot = (A.hover == 10 + i && (!o || A.pointer_output == o)) || A.focus == i;
            double r = (hot ? 23 : 20) * U;
            if (hot) {
                hex_path(cr, px, py, r + 6 * U);
                set_rgb(cr, th->accent, 0.85);
                cairo_set_line_width(cr, 1.6 * U);
                cairo_stroke(cr);
            }
            draw_avatar(cr, A.people[i].name, A.people[i].display, px, py, r, hot ? 1 : 0.8);
            if (A.people[i].signed_in) {
                cairo_arc(cr, px + r * 0.78, py - r * 0.62, 4.5 * U, 0, 2 * M_PI);
                set_rgb(cr, th->bg, 1);
                cairo_fill_preserve(cr);
                set_rgb(cr, th->ok, 1);
                cairo_set_line_width(cr, 1.2 * U);
                cairo_stroke(cr);
                cairo_arc(cr, px + r * 0.78, py - r * 0.62, 2.6 * U, 0, 2 * M_PI);
                cairo_fill(cr);
            }
            text(cr, "Inter", 11.5 * U, 0, A.people[i].display, px, py + 34 * U, 1, hot ? th->text : th->muted, 0.95);
            if (o) o->people[i] = (struct rect){px - step / 2 + 4 * U, py - 30 * U, step - 8 * U, 76 * U};
        }
    }

    /* Top left: the mark and the machine. */
    double m = 40 * U;
    hex_path(cr, m + 10 * U, m, 11 * U);
    set_rgb(cr, th->accent, 0.9);
    cairo_set_line_width(cr, 2 * U);
    cairo_stroke(cr);
    cairo_arc(cr, m + 10 * U, m, 3 * U, 0, 2 * M_PI);
    cairo_fill(cr);
    double nw = text(cr, "JetBrains Mono Bold", 14 * U, 5 * U, "NEXA", m + 30 * U, m, 0, th->text, 1);
    snprintf(buf, sizeof buf, "//  %s@%s", A.user, A.host);
    text(cr, "JetBrains Mono", 12 * U, 0, buf, m + 40 * U + nw, m, 0, th->muted, 0.9);

    /* Top right: live readings. */
    const struct telemetry *tm = &A.tm;
    double tx = W - m - 250 * U, ty = m, row = 22 * U;
    int r = 0;
    struct { const char *k; char v[48]; double pct; rgb c; } rows[6];
    int nrows = 0;
    rows[nrows].k = "CPU";
    if (tm->cpu >= 0) snprintf(rows[nrows].v, 48, "%3.0f%%", tm->cpu); else snprintf(rows[nrows].v, 48, "  --");
    rows[nrows].pct = tm->cpu; rows[nrows].c = tm->cpu > 85 ? th->warn : th->accent; nrows++;
    rows[nrows].k = "MEM";
    snprintf(rows[nrows].v, 48, "%.1f/%.0fG", tm->mem_used, tm->mem_total);
    rows[nrows].pct = tm->mem_total > 0 ? 100 * tm->mem_used / tm->mem_total : 0; rows[nrows].c = th->accent; nrows++;
    if (tm->bat >= 0) {
        rows[nrows].k = "BAT";
        snprintf(rows[nrows].v, 48, "%3d%% %s", tm->bat, !strcmp(tm->bat_state, "Charging") ? "+" : !strcmp(tm->bat_state, "Discharging") ? "-" : "=");
        rows[nrows].pct = tm->bat; rows[nrows].c = tm->bat <= 15 ? th->crit : tm->bat <= 30 ? th->warn : th->ok; nrows++;
    }
    if (tm->temp > 0) {
        rows[nrows].k = "TMP";
        snprintf(rows[nrows].v, 48, "%3.0f C", tm->temp);
        rows[nrows].pct = tm->temp; rows[nrows].c = tm->temp > 85 ? th->crit : tm->temp > 70 ? th->warn : th->accent; nrows++;
    }
    rows[nrows].k = "NET";
    snprintf(rows[nrows].v, 48, "%s", tm->net);
    rows[nrows].pct = tm->net_level; rows[nrows].c = th->accent2; nrows++;
    rows[nrows].k = "UP";
    snprintf(rows[nrows].v, 48, "%ldh %02ldm", tm->uptime / 3600, (tm->uptime / 60) % 60);
    rows[nrows].pct = -1; nrows++;
    for (r = 0; r < nrows; r++) {
        double yy = ty + r * row;
        text(cr, "JetBrains Mono Bold", 10 * U, 2 * U, rows[r].k, tx, yy, 0, th->muted, 0.9);
        text(cr, "JetBrains Mono", 11.5 * U, 0, rows[r].v, tx + 44 * U, yy, 0, th->text, 0.95);
        if (rows[r].pct >= 0) meter(cr, tx + 160 * U, yy, U, rows[r].pct, rows[r].c, th->muted);
    }
    snprintf(buf, sizeof buf, "linux %s", tm->kernel);
    text(cr, "JetBrains Mono", 10 * U, 0, buf, tx, ty + nrows * row, 0, th->muted, 0.6);

    /* Bottom left: what happened, newest last. */
    for (int i = 0; i < A.nlog; i++) {
        double yy = H - m - (A.nlog - 1 - i) * 18 * U;
        double a = 0.35 + 0.6 * (i + 1) / A.nlog;
        text(cr, "JetBrains Mono", 11 * U, 0, A.log[i], m, yy, 0, i == A.nlog - 1 ? th->text : th->muted, a);
    }

    /* Bottom right: Sleep, Restart, Shut down. */
    static const char *labels[3] = {"Sleep", "Restart", "Shut down"};
    for (int i = 0; i < 3; i++) {
        double bx = W - m - 26 * U - (2 - i) * 78 * U, by = H - m - 18 * U;
        int hot = A.hover == i && (!o || A.pointer_output == o);
        int armed = A.armed == i && t - A.armed_at < 4;
        hex_path(cr, bx, by, 24 * U);
        set_rgb(cr, armed ? th->warn : th->accent, armed ? 0.25 : hot ? 0.22 : 0.06);
        cairo_fill_preserve(cr);
        set_rgb(cr, armed ? th->warn : th->accent, armed ? 0.9 : hot ? 0.8 : 0.3);
        cairo_set_line_width(cr, 1.3 * U);
        cairo_stroke(cr);
        draw_power_icon(cr, i, bx, by, 18 * U, armed ? th->warn : th->text, hot || armed ? 1 : 0.75);
        text(cr, "Inter", 11 * U, 0, armed ? "Click again" : labels[i], bx, by + 38 * U, 1,
             armed ? th->warn : th->muted, 0.9);
        if (o) o->power[i] = (struct rect){bx - 30 * U, by - 30 * U, 60 * U, 80 * U};
    }

    /* The unlock: everything washes out into the theme colour. */
    if (A.unlock_at > 0) {
        double k = clampd((t - (A.unlock_at - 0.45)) / 0.45, 0, 1);
        set_rgb(cr, th->bg, k * 0.85);
        cairo_paint(cr);
    }
}

/* ---------- Wayland buffers and surfaces ---------- */

static void buffer_release(void *data, struct wl_buffer *b) { ((struct buffer *)data)->busy = 0; }
static const struct wl_buffer_listener buffer_listener = {.release = buffer_release};

static void free_buffer(struct buffer *b) {
    if (b->cs) cairo_surface_destroy(b->cs);
    if (b->wl) wl_buffer_destroy(b->wl);
    if (b->data) munmap(b->data, b->size);
    memset(b, 0, sizeof *b);
}

static int make_buffer(struct buffer *b, int w, int h) {
    free_buffer(b);
    int stride = w * 4;
    size_t size = (size_t)stride * h;
    int fd = memfd_create("nexa-locker", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (fd < 0) return -1;
    if (ftruncate(fd, (off_t)size) < 0) { close(fd); return -1; }
    void *data = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (data == MAP_FAILED) { close(fd); return -1; }
    struct wl_shm_pool *pool = wl_shm_create_pool(A.shm, fd, (int32_t)size);
    b->wl = wl_shm_pool_create_buffer(pool, 0, w, h, stride, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    wl_buffer_add_listener(b->wl, &buffer_listener, b);
    b->data = data;
    b->size = size;
    b->w = w;
    b->h = h;
    b->cs = cairo_image_surface_create_for_data(data, CAIRO_FORMAT_ARGB32, w, h, stride);
    return 0;
}

/* The lock screen as it is: background, then the scene. */
static void paint_lock(cairo_t *cr, struct output *o, cairo_surface_t *bg, int BW, int BH, int s) {
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    cairo_set_source_surface(cr, bg, 0, 0);
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
    cairo_save(cr);
    cairo_scale(cr, s, s);
    draw_scene(cr, o, (double)BW / s, (double)BH / s);
    cairo_restore(cr);
}

/* A transition frame, in buffer pixels. The honeycomb is the picture (the
 * desktop or the lock screen) cut into the grid's hexagons; a wave runs
 * from the clock's centre outwards, each cell shrinking away as it passes
 * (leaving) or growing back (arriving), its edge lit as it goes. */
static void draw_transition(cairo_t *cr, cairo_surface_t *src, int BW, int BH, int s) {
    const struct theme *th = &A.th;
    double W = BW, H = BH, U = unit_for((double)BH / s) * s, t = now_s();
    double cx = W / 2, cy = H * 0.42, cell = 30 * U;
    double maxd = hypot(fmax(cx, W - cx), fmax(cy, H - cy)), diag = hypot(W, H);
    int depart = A.tr == TR_DEPART;
    double p = A.tr == TR_AWAY ? 1 : clampd((t - A.tr_at) / (depart ? DEPART_S : ARRIVE_S), 0, 1);
    rgb dark = mix(th->bg, (rgb){0, 0, 0}, 0.8);

    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    set_rgb(cr, dark, 1);
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
    if (A.tr == TR_AWAY) return;

    double f = depart ? clampd(p / 0.62, 0, 1) : clampd((p - 0.2) / 0.72, 0, 1);
#define CELL_C(x, y) smooth((f * 1.3 - hypot((x) - cx, (y) - cy) / maxd) / 0.3)
    if (src) {
        double sw = cairo_image_surface_get_width(src), sh = cairo_image_surface_get_height(src);
        int any = 0;
        cairo_new_path(cr);
        GRID_EACH(W, H, U, x, y) {
            double c = CELL_C(x, y), k = depart ? 1 - c : c;
            if (k <= 0.01) continue;
            hex_path(cr, x, y, cell * fmin(k, 1) * 1.035);
            any = 1;
        }
        if (any) {
            cairo_save(cr);
            cairo_clip(cr);
            cairo_scale(cr, W / sw, H / sh);
            cairo_set_source_surface(cr, src, 0, 0);
            cairo_paint(cr);
            cairo_restore(cr);
        }
        cairo_new_path(cr);
    }
    /* The wave's edge: cells part-way through glow, in three strengths. */
    for (int band = 1; band <= 3; band++) {
        cairo_new_path(cr);
        int any = 0;
        GRID_EACH(W, H, U, x, y) {
            double c = CELL_C(x, y), glow = sin(M_PI * c);
            if (glow <= (band - 1) / 3.0 || glow > band / 3.0 || glow < 0.08) continue;
            double k = depart ? 1 - c : c;
            hex_path(cr, x, y, cell * fmax(k, 0.35) * 0.94);
            any = 1;
        }
        if (!any) continue;
        set_rgb(cr, depart ? th->accent2 : th->accent, 0.10 * band);
        cairo_fill_preserve(cr);
        set_rgb(cr, depart ? th->accent2 : th->accent, 0.3 * band);
        cairo_set_line_width(cr, 1.4 * U);
        cairo_stroke(cr);
    }
#undef CELL_C

    /* The person: their hexagon warps in at the centre, inside turning rings. */
    const char *name = depart ? A.tr_name : A.user, *display = depart ? A.tr_display : A.display_name;
    double q = depart ? clampd((p - 0.28) / 0.34, 0, 1) : clampd(p / 0.3, 0, 1);
    double fade = depart ? 1 : 1 - smooth((p - 0.36) / 0.16);
    double Ra = 56 * U * ease_back(q);
    double a = fmin(q, 1) * fade;
    /* Arriving: the person's hexagon hands over to the clock's, an outline
     * growing from one to the other as the honeycomb comes back. */
    if (!depart && p > 0.36 && p < 0.95) {
        double g = smooth((p - 0.36) / 0.5);
        hex_path(cr, cx, cy, 56 * U + g * (170 - 56) * U);
        set_rgb(cr, th->accent, 0.9 * (1 - g));
        cairo_set_line_width(cr, 2.5 * U);
        cairo_stroke(cr);
    }
    if (a > 0.01) {
        for (int k = 0; k < 3; k++) {
            double rr = Ra * (1.45 + 0.42 * k) * (depart ? 1 + 0.25 * (1 - q) : 1 + 0.5 * (1 - q));
            cairo_save(cr);
            cairo_translate(cr, cx, cy);
            cairo_rotate(cr, t * (0.5 + 0.35 * k) * (k % 2 ? -1 : 1));
            hex_path(cr, 0, 0, rr);
            cairo_restore(cr);
            set_rgb(cr, k == 1 ? th->accent2 : th->accent, a * (0.55 - 0.15 * k));
            cairo_set_line_width(cr, (2.2 - 0.5 * k) * U);
            cairo_stroke(cr);
        }
        draw_avatar(cr, name, display, cx, cy, Ra, a);
        char buf[160];
        if (depart) snprintf(buf, sizeof buf, "Switching to %s", display);
        else snprintf(buf, sizeof buf, A.tr_back ? "Welcome back, %s" : "Welcome, %s", display);
        double ty = cy + 56 * U * 1.45 * 1.9 + 18 * U;
        text(cr, "Inter Semi-Bold", 22 * U, 0, buf, cx, ty, 1, th->text, a);
        /* a terminal line typing itself out */
        char line[200];
        if (depart) snprintf(line, sizeof line, "handover  %.32s@%.32s -> %.32s@%.32s", A.user, A.host, name, A.host);
        else snprintf(line, sizeof line, "session  %.32s@%.32s  restored", name, A.host);
        size_t len = strlen(line), shown = (size_t)(len * clampd((p - (depart ? 0.4 : 0.1)) / 0.3, 0, 1));
        line[shown] = 0;
        if (shown < len && fmod(t, 0.5) < 0.25) strcat(line, "_");
        text(cr, "JetBrains Mono", 12 * U, 0, line, cx, ty + 30 * U, 1, depart ? th->accent2 : th->accent, a * 0.9);
    }

    /* Leaving: a hexagon iris opens from the person to the screen's edges,
     * and everything goes dark behind it, ready for the other desktop. */
    if (depart && p > 0.76) {
        double k = smooth((p - 0.76) / 0.24);
        hex_path(cr, cx, cy, 56 * U + k * diag);
        set_rgb(cr, th->accent, 0.9 * (1 - k));
        cairo_set_line_width(cr, 3 * U);
        cairo_stroke(cr);
        set_rgb(cr, dark, k);
        cairo_paint(cr);
    }
}

/* One frame: the lock screen, or a transition built from it (or from the
 * desktop snapshot). */
static void render_frame(cairo_t *cr, struct output *o, cairo_surface_t *bg, cairo_surface_t **scene,
                         cairo_surface_t *snap, int BW, int BH, int s) {
    if (A.tr == TR_NONE) {
        paint_lock(cr, o, bg, BW, BH, s);
        return;
    }
    cairo_surface_t *src = NULL;
    if (A.tr == TR_DEPART && snap) {
        src = snap;
    } else if (A.tr != TR_AWAY) {
        if (*scene && (cairo_image_surface_get_width(*scene) != BW || cairo_image_surface_get_height(*scene) != BH)) {
            cairo_surface_destroy(*scene);
            *scene = NULL;
        }
        if (!*scene) *scene = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, BW, BH);
        cairo_t *c2 = cairo_create(*scene);
        paint_lock(c2, o, bg, BW, BH, s);
        cairo_destroy(c2);
        src = *scene;
    }
    draw_transition(cr, src, BW, BH, s);
}

static void render_output(struct output *o) {
    if (!o->configured || !o->width || !o->height) return;
    int s = o->scale > 0 ? o->scale : 1, BW = o->width * s, BH = o->height * s;
    struct buffer *b = NULL;
    for (int i = 0; i < 2; i++)
        if (!o->buf[i].busy) { b = &o->buf[i]; break; }
    if (!b) return;  /* both on screen still: the next tick draws */
    if ((b->w != BW || b->h != BH) && make_buffer(b, BW, BH) < 0) return;
    if (!o->bg || o->bg_w != BW || o->bg_h != BH) {
        if (o->bg) cairo_surface_destroy(o->bg);
        o->bg = make_background(BW, BH, s);
        o->bg_w = BW;
        o->bg_h = BH;
    }
    cairo_t *cr = cairo_create(b->cs);
    render_frame(cr, o, o->bg, &o->scene, o->snap, BW, BH, s);
    cairo_destroy(cr);
    cairo_surface_flush(b->cs);
    wl_surface_set_buffer_scale(o->surface, s);
    wl_surface_attach(o->surface, b->wl, 0, 0);
    wl_surface_damage_buffer(o->surface, 0, 0, BW, BH);
    wl_surface_commit(o->surface);
    b->busy = 1;
}

static void render_all(void) {
    struct output *o;
    wl_list_for_each(o, &A.outputs, link) render_output(o);
    A.redraw = 0;
}

static void lock_surface_configure(void *data, struct ext_session_lock_surface_v1 *ls, uint32_t serial,
                                   uint32_t w, uint32_t h) {
    struct output *o = data;
    ext_session_lock_surface_v1_ack_configure(ls, serial);
    o->width = (int)w;
    o->height = (int)h;
    o->configured = 1;
    render_output(o);
}
static const struct ext_session_lock_surface_v1_listener lock_surface_listener = {.configure = lock_surface_configure};

static void create_lock_surface(struct output *o) {
    if (o->surface || !A.lock) return;
    o->surface = wl_compositor_create_surface(A.compositor);
    o->lock_surface = ext_session_lock_v1_get_lock_surface(A.lock, o->surface, o->wl);
    ext_session_lock_surface_v1_add_listener(o->lock_surface, &lock_surface_listener, o);
}

/* ---------- the desktop snapshot (wlr-screencopy) ---------- */

static void cap_buffer(void *data, struct zwlr_screencopy_frame_v1 *f, uint32_t format, uint32_t w, uint32_t h, uint32_t stride) {
    struct output *o = data;
    if (o->cap.wl || (format != WL_SHM_FORMAT_ARGB8888 && format != WL_SHM_FORMAT_XRGB8888 &&
                      format != WL_SHM_FORMAT_ABGR8888 && format != WL_SHM_FORMAT_XBGR8888)) return;
    size_t size = (size_t)stride * h;
    int fd = memfd_create("nexa-locker-snap", MFD_CLOEXEC);
    if (fd < 0) return;
    void *px = MAP_FAILED;
    if (ftruncate(fd, (off_t)size) == 0) px = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (px == MAP_FAILED) { close(fd); return; }
    struct wl_shm_pool *pool = wl_shm_create_pool(A.shm, fd, (int32_t)size);
    o->cap.wl = wl_shm_pool_create_buffer(pool, 0, (int32_t)w, (int32_t)h, (int32_t)stride, format);
    wl_shm_pool_destroy(pool);
    close(fd);
    o->cap.data = px;
    o->cap.size = size;
    o->cap.w = (int)w;
    o->cap.h = (int)h;
    o->cap_format = format;
    o->cap_stride = stride;
    if (A.screencopy_version < 3) zwlr_screencopy_frame_v1_copy(f, o->cap.wl);
}
static void cap_flags(void *data, struct zwlr_screencopy_frame_v1 *f, uint32_t flags) { ((struct output *)data)->cap_flags = flags; }
static void cap_ready(void *data, struct zwlr_screencopy_frame_v1 *f, uint32_t hi, uint32_t lo, uint32_t ns) {
    struct output *o = data;
    int w = o->cap.w, h = o->cap.h;
    cairo_surface_t *img = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, w, h);
    if (cairo_surface_status(img) == CAIRO_STATUS_SUCCESS) {
        unsigned char *dst = cairo_image_surface_get_data(img);
        int ds = cairo_image_surface_get_stride(img);
        int swap = o->cap_format == WL_SHM_FORMAT_ABGR8888 || o->cap_format == WL_SHM_FORMAT_XBGR8888;
        for (int y = 0; y < h; y++) {
            int sy = (o->cap_flags & ZWLR_SCREENCOPY_FRAME_V1_FLAGS_Y_INVERT) ? h - 1 - y : y;
            const uint32_t *sp = (const uint32_t *)((const unsigned char *)o->cap.data + (size_t)sy * o->cap_stride);
            uint32_t *dp = (uint32_t *)(dst + (size_t)y * ds);
            for (int x = 0; x < w; x++) {
                uint32_t v = sp[x];
                if (swap) v = (v & 0xff00ff00u) | ((v & 0xffu) << 16) | ((v >> 16) & 0xffu);
                dp[x] = v | 0xff000000u;  /* the screen is opaque */
            }
        }
        cairo_surface_mark_dirty(img);
        o->snap = img;
        o->cap_state = 2;
    } else {
        cairo_surface_destroy(img);
        o->cap_state = -1;
    }
    zwlr_screencopy_frame_v1_destroy(f);
    o->frame = NULL;
    if (o->cap.wl) wl_buffer_destroy(o->cap.wl);
    if (o->cap.data) munmap(o->cap.data, o->cap.size);
    memset(&o->cap, 0, sizeof o->cap);
}
static void cap_failed(void *data, struct zwlr_screencopy_frame_v1 *f) {
    struct output *o = data;
    o->cap_state = -1;
    zwlr_screencopy_frame_v1_destroy(f);
    o->frame = NULL;
}
static void cap_damage(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t x, uint32_t y, uint32_t w, uint32_t h) {}
static void cap_dmabuf(void *d, struct zwlr_screencopy_frame_v1 *f, uint32_t fmt, uint32_t w, uint32_t h) {}
static void cap_buffer_done(void *data, struct zwlr_screencopy_frame_v1 *f) {
    struct output *o = data;
    if (o->cap.wl) zwlr_screencopy_frame_v1_copy(f, o->cap.wl);
    else cap_failed(data, f);
}
static const struct zwlr_screencopy_frame_v1_listener cap_listener = {
    .buffer = cap_buffer, .flags = cap_flags, .ready = cap_ready, .failed = cap_failed,
    .damage = cap_damage, .linux_dmabuf = cap_dmabuf, .buffer_done = cap_buffer_done};

/* Every screen as it is now, so the hand-over can dissolve the desktop
 * itself. Half a second at most; without it the lock screen dissolves. */
static void snapshot_desktop(void) {
    if (!A.screencopy) return;
    struct output *o;
    wl_list_for_each(o, &A.outputs, link) {
        o->frame = zwlr_screencopy_manager_v1_capture_output(A.screencopy, 0, o->wl);
        zwlr_screencopy_frame_v1_add_listener(o->frame, &cap_listener, o);
        o->cap_state = 1;
    }
    double until = now_s() + 0.5;
    for (;;) {
        int waiting = 0;
        wl_list_for_each(o, &A.outputs, link) waiting |= o->cap_state == 1;
        if (!waiting || now_s() > until) break;
        wl_display_flush(A.display);
        struct pollfd pf = {.fd = wl_display_get_fd(A.display), .events = POLLIN};
        if (poll(&pf, 1, 50) > 0) {
            if (wl_display_dispatch(A.display) < 0) break;
        } else {
            wl_display_dispatch_pending(A.display);
        }
    }
}

static void destroy_output(struct output *o) {
    if (A.pointer_output == o) A.pointer_output = NULL;
    if (o->lock_surface) ext_session_lock_surface_v1_destroy(o->lock_surface);
    if (o->surface) wl_surface_destroy(o->surface);
    for (int i = 0; i < 2; i++) free_buffer(&o->buf[i]);
    if (o->bg) cairo_surface_destroy(o->bg);
    if (o->scene) cairo_surface_destroy(o->scene);
    if (o->snap) cairo_surface_destroy(o->snap);
    if (o->frame) zwlr_screencopy_frame_v1_destroy(o->frame);
    free_buffer(&o->cap);
    wl_output_destroy(o->wl);
    wl_list_remove(&o->link);
    free(o);
}

/* ---------- outputs ---------- */

static void out_geometry(void *d, struct wl_output *w, int32_t x, int32_t y, int32_t pw, int32_t ph, int32_t sp,
                         const char *mk, const char *md, int32_t tr) {}
static void out_mode(void *d, struct wl_output *w, uint32_t f, int32_t wd, int32_t ht, int32_t r) {}
static void out_done(void *d, struct wl_output *w) {}
static void out_scale(void *data, struct wl_output *w, int32_t factor) {
    struct output *o = data;
    o->scale = factor;
    if (o->configured) render_output(o);
}
static void out_name(void *d, struct wl_output *w, const char *n) {}
static void out_desc(void *d, struct wl_output *w, const char *n) {}
static const struct wl_output_listener output_listener = {
    .geometry = out_geometry, .mode = out_mode, .done = out_done, .scale = out_scale,
    .name = out_name, .description = out_desc};

/* ---------- keyboard ---------- */

static void wipe_password(void) {
    if (A.pw) explicit_bzero(A.pw, MAX_PW);
    A.pw_len = 0;
    A.pw_chars = 0;
}

static int conv(int n, const struct pam_message **msg, struct pam_response **resp, void *data) {
    struct pam_response *r = calloc((size_t)n, sizeof *r);
    if (!r) return PAM_BUF_ERR;
    for (int i = 0; i < n; i++)
        if (msg[i]->msg_style == PAM_PROMPT_ECHO_OFF || msg[i]->msg_style == PAM_PROMPT_ECHO_ON)
            r[i].resp = strdup((const char *)data);
    *resp = r;
    return PAM_SUCCESS;
}

/* In the forked child: 1 if PAM accepts the password. */
static int pam_check(void) {
    struct pam_conv c = {conv, A.pw};
    pam_handle_t *h = NULL;
    if (pam_start(A.pam_service, A.user, &c, &h) != PAM_SUCCESS) return 0;
    int rc = pam_authenticate(h, 0);
    pam_end(h, rc);
    return rc == PAM_SUCCESS;
}

static void submit(void) {
    double t = now_s();
    if (A.checking || A.unlock_at > 0 || A.cooldown_until > t) return;
    int p[2];
    if (pipe2(p, O_CLOEXEC) < 0) return;
    pid_t pid = fork();
    if (pid == 0) {
        close(p[0]);
        char c = pam_check() ? 'y' : 'n';
        explicit_bzero(A.pw, MAX_PW);
        if (write(p[1], &c, 1) < 0) _exit(1);
        _exit(0);
    }
    close(p[1]);
    wipe_password();
    if (pid < 0) {
        close(p[0]);
        add_log("auth: couldn't start the check");
        return;
    }
    A.pam_pid = pid;
    A.pam_fd = p[0];
    A.checking = 1;
    add_log("auth: checking password (pam %s)", A.pam_service);
}

static void pam_done(void) {
    char c = 'n';
    if (read(A.pam_fd, &c, 1) != 1) c = 'n';
    close(A.pam_fd);
    A.pam_fd = -1;
    waitpid(A.pam_pid, NULL, 0);
    A.checking = 0;
    double t = now_s();
    if (c == 'y') {
        add_log("auth: accepted, unlocking");
        add_ripple(RP_GOOD);
        A.unlock_at = t + 0.45;
        return;
    }
    A.fails++;
    A.wrong_at = t;
    add_ripple(RP_WRONG);
    add_log("auth: denied (%d)", A.fails);
    if (A.fails >= 5) {
        int wait = (A.fails - 4) * 10;
        A.cooldown_until = t + (wait > 60 ? 60 : wait);
        add_log("auth: too many tries, waiting %d s", wait > 60 ? 60 : wait);
    }
}

static int ctrl_down(void) {
    return A.xkb_state && xkb_state_mod_name_is_active(A.xkb_state, XKB_MOD_NAME_CTRL, XKB_STATE_MODS_EFFECTIVE) > 0;
}

static void handle_key(xkb_keysym_t sym, const char *utf8) {
    if (A.tr == TR_AWAY && A.sw_pid <= 0) { start_arrival(1); return; }
    if (A.tr != TR_NONE || A.checking || A.unlock_at > 0) return;
    A.armed = -1;
    switch (sym) {
    case XKB_KEY_Tab:
    case XKB_KEY_ISO_Left_Tab:
        /* the password field, then each person, round again */
        if (A.npeople) {
            int dir = sym == XKB_KEY_ISO_Left_Tab ? -1 : 1;
            A.focus = (A.focus + 1 + dir + A.npeople + 1) % (A.npeople + 1) - 1;
            A.redraw = 1;
        }
        return;
    case XKB_KEY_Left:
    case XKB_KEY_Right:
        if (A.focus >= 0) {
            A.focus = (A.focus + (sym == XKB_KEY_Right ? 1 : A.npeople - 1)) % A.npeople;
            A.redraw = 1;
        }
        return;
    case XKB_KEY_Return:
    case XKB_KEY_KP_Enter:
        if (A.focus >= 0) begin_switch(A.focus);
        else submit();
        return;
    case XKB_KEY_BackSpace:
        if (ctrl_down()) wipe_password();
        else if (A.pw_len) {
            do A.pw_len--; while (A.pw_len && (A.pw[A.pw_len] & 0xC0) == 0x80);
            A.pw[A.pw_len] = 0;
            A.pw_chars--;
        }
        break;
    case XKB_KEY_Escape:
        if (A.focus >= 0) { A.focus = -1; A.redraw = 1; return; }
        wipe_password();
        break;
    default:
        A.focus = -1;  /* typing goes to the password */
        if (ctrl_down()) {
            if (sym == XKB_KEY_u || sym == XKB_KEY_U) wipe_password();
            else return;
        } else {
            size_t n = strlen(utf8);
            if (!n || (unsigned char)utf8[0] < 0x20 || utf8[0] == 0x7f || A.pw_len + n >= MAX_PW - 1) return;
            memcpy(A.pw + A.pw_len, utf8, n);
            A.pw_len += n;
            A.pw[A.pw_len] = 0;
            A.pw_chars++;
        }
    }
    double t = now_s();
    A.typed_at = t;
    add_ripple(RP_KEY);
}

static void update_mods(void) {
    int caps = xkb_state_mod_name_is_active(A.xkb_state, XKB_MOD_NAME_CAPS, XKB_STATE_MODS_LOCKED) > 0;
    xkb_layout_index_t li = xkb_state_serialize_layout(A.xkb_state, XKB_STATE_LAYOUT_EFFECTIVE);
    const char *name = xkb_keymap_layout_get_name(A.keymap, li);
    char lay[32] = "";
    if (name) {
        /* "English (US)" -> "US"; otherwise the first two letters. */
        const char *paren = strrchr(name, '(');
        if (paren) snprintf(lay, sizeof lay, "%.*s", (int)strcspn(paren + 1, ")"), paren + 1);
        else snprintf(lay, sizeof lay, "%.2s", name);
        for (char *c = lay; *c; c++) *c = (char)toupper((unsigned char)*c);
    }
    if (caps != A.caps || strcmp(lay, A.layout)) {
        A.caps = caps;
        snprintf(A.layout, sizeof A.layout, "%s", lay);
        A.redraw = 1;
    }
}

static void kb_keymap(void *d, struct wl_keyboard *k, uint32_t format, int32_t fd, uint32_t size) {
    if (format != WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1) { close(fd); return; }
    char *map = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (map == MAP_FAILED) return;
    struct xkb_keymap *km = xkb_keymap_new_from_string(A.xkb, map, XKB_KEYMAP_FORMAT_TEXT_V1, XKB_KEYMAP_COMPILE_NO_FLAGS);
    munmap(map, size);
    if (getenv("NEXA_LOCKER_DEBUG")) fprintf(stderr, "keymap %s (%u bytes)\n", km ? "loaded" : "FAILED", size);
    if (!km) return;
    struct xkb_state *st = xkb_state_new(km);
    if (!st) { xkb_keymap_unref(km); return; }
    if (A.xkb_state) xkb_state_unref(A.xkb_state);
    if (A.keymap) xkb_keymap_unref(A.keymap);
    A.keymap = km;
    A.xkb_state = st;
    update_mods();
}
static void kb_enter(void *d, struct wl_keyboard *k, uint32_t s, struct wl_surface *sf, struct wl_array *keys) {}
static void kb_leave(void *d, struct wl_keyboard *k, uint32_t s, struct wl_surface *sf) {}

static void arm_repeat(int on) {
    struct itimerspec its = {0};
    if (on && A.repeat_rate > 0) {
        its.it_value.tv_sec = A.repeat_delay / 1000;
        its.it_value.tv_nsec = (A.repeat_delay % 1000) * 1000000L;
        its.it_interval.tv_nsec = 1000000000L / A.repeat_rate;
        if (A.repeat_rate == 1) { its.it_interval.tv_sec = 1; its.it_interval.tv_nsec = 0; }
    }
    timerfd_settime(A.repeat_fd, 0, &its, NULL);
}

static void kb_key(void *d, struct wl_keyboard *k, uint32_t serial, uint32_t time, uint32_t key, uint32_t state) {
    if (!A.xkb_state) return;
    uint32_t code = key + 8;
    if (state != WL_KEYBOARD_KEY_STATE_PRESSED) {
        if (code == A.repeat_key) { A.repeat_key = 0; arm_repeat(0); }
        return;
    }
    xkb_keysym_t sym = xkb_state_key_get_one_sym(A.xkb_state, code);
    char utf8[16] = "";
    xkb_state_key_get_utf8(A.xkb_state, code, utf8, sizeof utf8);
    if (getenv("NEXA_LOCKER_DEBUG")) {
        /* the kind of key only, never what was typed */
        char name[64] = "";
        if ((unsigned char)utf8[0] < 0x20) xkb_keysym_get_name(sym, name, sizeof name);
        fprintf(stderr, "key %u: %s\n", code, name[0] ? name : "text");
    }
    handle_key(sym, utf8);
    if (sym != XKB_KEY_Return && sym != XKB_KEY_KP_Enter && xkb_keymap_key_repeats(A.keymap, code)) {
        A.repeat_key = code;
        A.repeat_sym = sym;
        memcpy(A.repeat_utf8, utf8, sizeof utf8);
        arm_repeat(1);
    }
    explicit_bzero(utf8, sizeof utf8);
}
static void kb_modifiers(void *d, struct wl_keyboard *k, uint32_t s, uint32_t dep, uint32_t lat, uint32_t lck, uint32_t grp) {
    if (!A.xkb_state) return;
    xkb_state_update_mask(A.xkb_state, dep, lat, lck, 0, 0, grp);
    update_mods();
}
static void kb_repeat_info(void *d, struct wl_keyboard *k, int32_t rate, int32_t delay) {
    A.repeat_rate = rate;
    A.repeat_delay = delay;
}
static const struct wl_keyboard_listener keyboard_listener = {
    .keymap = kb_keymap, .enter = kb_enter, .leave = kb_leave, .key = kb_key,
    .modifiers = kb_modifiers, .repeat_info = kb_repeat_info};

/* ---------- pointer: the power buttons ---------- */

/* 0-2: a power button; 10 + i: person i. */
static int hit(struct output *o, double x, double y) {
    if (!o || A.tr != TR_NONE) return -1;
    for (int i = 0; i < 3; i++) {
        struct rect r = o->power[i];
        if (x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h) return i;
    }
    for (int i = 0; i < A.npeople; i++) {
        struct rect r = o->people[i];
        if (x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h) return 10 + i;
    }
    return -1;
}

static struct output *output_for(struct wl_surface *s) {
    struct output *o;
    wl_list_for_each(o, &A.outputs, link) if (o->surface == s) return o;
    return NULL;
}

static void ptr_enter(void *d, struct wl_pointer *p, uint32_t serial, struct wl_surface *s, wl_fixed_t x, wl_fixed_t y) {
    A.pointer_output = output_for(s);
    A.px = wl_fixed_to_double(x);
    A.py = wl_fixed_to_double(y);
    if (A.cursor_theme && A.cursor_surface) {
        struct wl_cursor *c = wl_cursor_theme_get_cursor(A.cursor_theme, "left_ptr");
        if (!c) c = wl_cursor_theme_get_cursor(A.cursor_theme, "default");
        if (c && c->image_count) {
            struct wl_cursor_image *img = c->images[0];
            wl_surface_attach(A.cursor_surface, wl_cursor_image_get_buffer(img), 0, 0);
            wl_surface_damage_buffer(A.cursor_surface, 0, 0, (int32_t)img->width, (int32_t)img->height);
            wl_surface_commit(A.cursor_surface);
            wl_pointer_set_cursor(p, serial, A.cursor_surface, (int32_t)img->hotspot_x, (int32_t)img->hotspot_y);
        }
    }
}
static void ptr_leave(void *d, struct wl_pointer *p, uint32_t serial, struct wl_surface *s) {
    A.pointer_output = NULL;
    if (A.hover != -1) { A.hover = -1; A.redraw = 1; }
}
static void ptr_motion(void *d, struct wl_pointer *p, uint32_t t, wl_fixed_t x, wl_fixed_t y) {
    A.px = wl_fixed_to_double(x);
    A.py = wl_fixed_to_double(y);
    int h = hit(A.pointer_output, A.px, A.py);
    if (h != A.hover) { A.hover = h; A.redraw = 1; }
}

static void power_action(int which) {
    static const char *acts[3] = {"suspend", "reboot", "poweroff"};
    add_log("power: %s", acts[which]);
    pid_t pid = fork();
    if (pid == 0) {
        int fd = open("/dev/null", O_RDWR);
        if (fd >= 0) { dup2(fd, 0); dup2(fd, 1); dup2(fd, 2); }
        execlp("nexa-brokerd", "nexa-brokerd", "call", acts[which], (char *)NULL);
        _exit(127);
    }
    /* reaped in the main loop */
}

static void ptr_button(void *d, struct wl_pointer *p, uint32_t serial, uint32_t t, uint32_t button, uint32_t state) {
    if (button != BTN_LEFT || state != WL_POINTER_BUTTON_STATE_PRESSED || A.unlock_at > 0) return;
    if (A.tr == TR_AWAY && A.sw_pid <= 0) { start_arrival(1); return; }
    int h = hit(A.pointer_output, A.px, A.py);
    if (h < 0) { A.armed = -1; A.redraw = 1; return; }
    if (h >= 10) { begin_switch(h - 10); A.redraw = 1; return; }
    double now = now_s();
    if (A.armed == h && now - A.armed_at < 4) {
        A.armed = -1;
        power_action(h);
    } else {
        A.armed = h;
        A.armed_at = now;
    }
    A.redraw = 1;
}
static void ptr_axis(void *d, struct wl_pointer *p, uint32_t t, uint32_t a, wl_fixed_t v) {}
static void ptr_frame(void *d, struct wl_pointer *p) {}
static void ptr_axis_source(void *d, struct wl_pointer *p, uint32_t s) {}
static void ptr_axis_stop(void *d, struct wl_pointer *p, uint32_t t, uint32_t a) {}
static void ptr_axis_discrete(void *d, struct wl_pointer *p, uint32_t a, int32_t v) {}
static const struct wl_pointer_listener pointer_listener = {
    .enter = ptr_enter, .leave = ptr_leave, .motion = ptr_motion, .button = ptr_button, .axis = ptr_axis,
    .frame = ptr_frame, .axis_source = ptr_axis_source, .axis_stop = ptr_axis_stop, .axis_discrete = ptr_axis_discrete};

static void seat_caps(void *d, struct wl_seat *s, uint32_t caps) {
    if ((caps & WL_SEAT_CAPABILITY_KEYBOARD) && !A.keyboard) {
        A.keyboard = wl_seat_get_keyboard(s);
        wl_keyboard_add_listener(A.keyboard, &keyboard_listener, NULL);
    } else if (!(caps & WL_SEAT_CAPABILITY_KEYBOARD) && A.keyboard) {
        wl_keyboard_release(A.keyboard);
        A.keyboard = NULL;
    }
    if ((caps & WL_SEAT_CAPABILITY_POINTER) && !A.pointer) {
        A.pointer = wl_seat_get_pointer(s);
        wl_pointer_add_listener(A.pointer, &pointer_listener, NULL);
    } else if (!(caps & WL_SEAT_CAPABILITY_POINTER) && A.pointer) {
        wl_pointer_release(A.pointer);
        A.pointer = NULL;
    }
}
static void seat_name(void *d, struct wl_seat *s, const char *n) {}
static const struct wl_seat_listener seat_listener = {.capabilities = seat_caps, .name = seat_name};

/* ---------- registry and the lock ---------- */

static void reg_global(void *d, struct wl_registry *reg, uint32_t name, const char *iface, uint32_t ver) {
    if (!strcmp(iface, wl_compositor_interface.name)) {
        A.compositor = wl_registry_bind(reg, name, &wl_compositor_interface, ver < 4 ? ver : 4);
    } else if (!strcmp(iface, wl_shm_interface.name)) {
        A.shm = wl_registry_bind(reg, name, &wl_shm_interface, 1);
    } else if (!strcmp(iface, wl_seat_interface.name) && !A.seat) {
        A.seat = wl_registry_bind(reg, name, &wl_seat_interface, ver < 5 ? ver : 5);
        wl_seat_add_listener(A.seat, &seat_listener, NULL);
    } else if (!strcmp(iface, zwlr_screencopy_manager_v1_interface.name)) {
        A.screencopy_version = ver < 3 ? ver : 3;
        A.screencopy = wl_registry_bind(reg, name, &zwlr_screencopy_manager_v1_interface, A.screencopy_version);
    } else if (!strcmp(iface, ext_session_lock_manager_v1_interface.name)) {
        A.lock_manager = wl_registry_bind(reg, name, &ext_session_lock_manager_v1_interface, 1);
    } else if (!strcmp(iface, wl_output_interface.name)) {
        struct output *o = calloc(1, sizeof *o);
        if (!o) return;
        o->name = name;
        o->scale = 1;
        o->wl = wl_registry_bind(reg, name, &wl_output_interface, ver < 4 ? ver : 4);
        wl_output_add_listener(o->wl, &output_listener, o);
        wl_list_insert(&A.outputs, &o->link);
        create_lock_surface(o);  /* a screen plugged in while locked */
    }
}
static void reg_remove(void *d, struct wl_registry *reg, uint32_t name) {
    struct output *o, *tmp;
    wl_list_for_each_safe(o, tmp, &A.outputs, link) if (o->name == name) destroy_output(o);
}
static const struct wl_registry_listener registry_listener = {.global = reg_global, .global_remove = reg_remove};

static void lock_locked(void *d, struct ext_session_lock_v1 *l) {
    A.locked = 1;
    add_log(A.boot ? "session started, locked" : "session locked (ext-session-lock-v1)");
    if (A.switch_to[0]) A.tr_at = now_s();     /* the hand-over starts now */
    else if (A.boot) start_arrival(0);
    char c = 'L';
    if (A.ready_fd >= 0) { if (write(A.ready_fd, &c, 1) < 0) {} close(A.ready_fd); A.ready_fd = -1; }
    if (A.watch_fd >= 0) { if (write(A.watch_fd, &c, 1) < 0) {} close(A.watch_fd); A.watch_fd = -1; }
}
static void lock_finished(void *d, struct ext_session_lock_v1 *l) {
    if (!A.locked) {
        fprintf(stderr, "nexa-locker: the compositor refused the lock (another lock screen running?)\n");
        A.exit_code = 2;
    } else {
        A.exit_code = 0;
    }
    ext_session_lock_v1_destroy(A.lock);
    A.lock = NULL;
    A.running = 0;
}
static const struct ext_session_lock_v1_listener lock_listener = {.locked = lock_locked, .finished = lock_finished};

/* ---------- setup ---------- */

static void who_am_i(void) {
    struct passwd *pw = getpwuid(getuid());
    snprintf(A.user, sizeof A.user, "%s", pw ? pw->pw_name : "user");
    const char *g = pw && pw->pw_gecos ? pw->pw_gecos : "";
    size_t n = strcspn(g, ",");
    if (n) snprintf(A.display_name, sizeof A.display_name, "%.*s", (int)n, g);
    else {
        snprintf(A.display_name, sizeof A.display_name, "%s", A.user);
        A.display_name[0] = (char)toupper((unsigned char)A.display_name[0]);
    }
    if (gethostname(A.host, sizeof A.host) != 0) snprintf(A.host, sizeof A.host, "nexa");
    A.host[sizeof A.host - 1] = 0;
    /* No password on the live USB (the installer writes login-required). */
    const char *env = getenv("NEXA_LOCKER_LIVE");
    A.live = env ? atoi(env) : access("/etc/nexa/login-required", F_OK) != 0 && getuid() == 1000;
    A.pam_service = access("/etc/pam.d/nexa-lock", F_OK) == 0 || access("/usr/lib/pam.d/nexa-lock", F_OK) == 0
                        ? "nexa-lock" : "swaylock";
}

static int preview(int argc, char **argv) {
    if (argc < 5) return 1;
    int W = atoi(argv[2]), H = atoi(argv[3]);
    const char *state = argc > 5 ? argv[5] : "idle";
    A.preview = 1;
    A.preview_now = 1000;
    A.start_at = -100;
    A.wrong_at = A.typed_at = -100;
    load_theme();
    who_am_i();
    A.live = !strcmp(state, "live");
    A.boot = !strcmp(state, "boot");
    snprintf(A.layout, sizeof A.layout, "US");
    read_telemetry();
    usleep(200000);
    read_telemetry();
    add_log("session locked (ext-session-lock-v1)");
    if (!strcmp(state, "typing")) { A.pw_chars = 9; A.typed_at = 999.9; A.ripples[0] = (struct ripple){999.55, RP_KEY}; }
    if (!strcmp(state, "checking")) { A.checking = 1; add_log("auth: checking password (pam nexa-lock)"); }
    if (!strcmp(state, "wrong")) {
        A.fails = 2;
        A.wrong_at = 999.7;
        A.ripples[0] = (struct ripple){999.5, RP_WRONG};
        add_log("auth: denied (1)");
        add_log("auth: denied (2)");
    }
    if (!strcmp(state, "caps")) A.caps = 1;
    if (!strcmp(state, "intro")) A.start_at = 999.6;
    if (!strcmp(state, "people") || !strncmp(state, "depart:", 7) || !strncmp(state, "arrive:", 7)) {
        A.npeople = 2;
        A.people[0] = (struct person){"ama", "Ama Mensah", 1};
        A.people[1] = (struct person){"kofi", "Kofi", 0};
        if (!strcmp(state, "people")) A.focus = 0;
    }
    if (!strncmp(state, "depart:", 7)) {
        begin_departure("ama", "Ama Mensah");
        A.tr_at = A.preview_now - atof(state + 7) * DEPART_S;
    } else if (!strncmp(state, "arrive:", 7)) {
        A.tr = TR_ARRIVE;
        A.tr_back = 1;
        A.tr_at = A.preview_now - atof(state + 7) * ARRIVE_S;
    }
    cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, W, H);
    cairo_t *cr = cairo_create(s);
    cairo_surface_t *bg = make_background(W, H, 1), *scene = NULL;
    render_frame(cr, NULL, bg, &scene, NULL, W, H, 1);
    if (scene) cairo_surface_destroy(scene);
    cairo_destroy(cr);
    int rc = cairo_surface_write_to_png(s, argv[4]) == CAIRO_STATUS_SUCCESS ? 0 : 1;
    cairo_surface_destroy(bg);
    cairo_surface_destroy(s);
    return rc;
}

static void arm_tick(void) {
    struct itimerspec its = {0};
    if (animating()) {
        its.it_value.tv_nsec = (A.tr != TR_NONE ? 16 : 33) * 1000000L;
    } else {
        struct timespec rt;
        clock_gettime(CLOCK_REALTIME, &rt);
        long ns = 1000000000L - rt.tv_nsec + 5000000L;
        its.it_value.tv_sec = ns / 1000000000L;
        its.it_value.tv_nsec = ns % 1000000000L;
    }
    timerfd_settime(A.timer_fd, 0, &its, NULL);
}

static int run_locker(void) {
    who_am_i();
    load_theme();
    A.pw = mmap(NULL, MAX_PW, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (A.pw == MAP_FAILED) return 1;
    mlock(A.pw, MAX_PW);
    madvise(A.pw, MAX_PW, MADV_DONTDUMP);
    wl_list_init(&A.outputs);

    A.display = wl_display_connect(NULL);
    if (!A.display) { fprintf(stderr, "nexa-locker: no Wayland display\n"); return 2; }
    A.xkb = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    struct wl_registry *reg = wl_display_get_registry(A.display);
    wl_registry_add_listener(reg, &registry_listener, NULL);
    wl_display_roundtrip(A.display);
    if (!A.compositor || !A.shm || !A.lock_manager) {
        fprintf(stderr, "nexa-locker: the compositor has no ext-session-lock-v1\n");
        return 2;
    }
    int cursor_scale = 1;
    wl_display_roundtrip(A.display);  /* output scales */
    struct output *o;
    wl_list_for_each(o, &A.outputs, link) if (o->scale > cursor_scale) cursor_scale = o->scale;
    A.cursor_theme = wl_cursor_theme_load(getenv("XCURSOR_THEME"), 24 * cursor_scale, A.shm);
    A.cursor_surface = wl_compositor_create_surface(A.compositor);
    if (cursor_scale > 1) wl_surface_set_buffer_scale(A.cursor_surface, cursor_scale);

    /* Coming back on screen: nexa-sessiond sends SIGUSR2. */
    sigset_t usr2;
    sigemptyset(&usr2);
    sigaddset(&usr2, SIGUSR2);
    sigprocmask(SIG_BLOCK, &usr2, NULL);
    signal(SIGUSR2, SIG_DFL);
    A.sig_fd = signalfd(-1, &usr2, SFD_CLOEXEC | SFD_NONBLOCK);

    load_people();
    if (A.switch_to[0]) {
        snapshot_desktop();
        const char *display = A.switch_to;
        for (int i = 0; i < A.npeople; i++) if (!strcmp(A.people[i].name, A.switch_to)) display = A.people[i].display;
        begin_departure(A.switch_to, display);
        A.tr_at = 1e12;  /* held on the snapshot until the lock is confirmed */
    } else if (A.boot) {
        A.tr = TR_AWAY;  /* dark until locked, then the arrival */
    }

    A.lock = ext_session_lock_manager_v1_lock(A.lock_manager);
    ext_session_lock_v1_add_listener(A.lock, &lock_listener, NULL);
    wl_list_for_each(o, &A.outputs, link) create_lock_surface(o);

    A.timer_fd = timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC | TFD_NONBLOCK);
    A.repeat_fd = timerfd_create(CLOCK_MONOTONIC, TFD_CLOEXEC | TFD_NONBLOCK);
    A.start_at = now_s();
    A.wrong_at = A.typed_at = A.armed_at = -100;
    read_telemetry();
    A.running = 1;
    A.exit_code = 1;
    time_t last_sec = 0;

    while (A.running) {
        while (wl_display_prepare_read(A.display) != 0) wl_display_dispatch_pending(A.display);
        wl_display_flush(A.display);
        arm_tick();
        struct pollfd fds[6] = {
            {.fd = wl_display_get_fd(A.display), .events = POLLIN},
            {.fd = A.timer_fd, .events = POLLIN},
            {.fd = A.repeat_fd, .events = POLLIN},
            {.fd = A.sig_fd, .events = POLLIN},
            {.fd = A.sw_fd, .events = POLLIN},
            {.fd = A.pam_fd, .events = POLLIN},
        };
        int n = poll(fds, 6, -1);  /* fds of -1 are skipped */
        if (n < 0 && errno != EINTR) { wl_display_cancel_read(A.display); break; }
        if (n > 0 && (fds[0].revents & POLLIN)) {
            if (wl_display_read_events(A.display) < 0) break;
        } else {
            wl_display_cancel_read(A.display);
        }
        if (n > 0 && (fds[0].revents & (POLLERR | POLLHUP))) break;
        if (wl_display_dispatch_pending(A.display) < 0) break;

        uint64_t ticks;
        if (n > 0 && (fds[1].revents & POLLIN)) {
            if (read(A.timer_fd, &ticks, sizeof ticks) < 0) {}
            A.redraw = 1;
        }
        if (n > 0 && (fds[2].revents & POLLIN)) {
            if (read(A.repeat_fd, &ticks, sizeof ticks) < 0) {}
            if (A.repeat_key) handle_key(A.repeat_sym, A.repeat_utf8);
        }
        if (n > 0 && A.pam_fd >= 0 && (fds[5].revents & (POLLIN | POLLHUP))) pam_done();
        if (n > 0 && A.sig_fd >= 0 && (fds[3].revents & POLLIN)) {
            struct signalfd_siginfo si;
            while (read(A.sig_fd, &si, sizeof si) == sizeof si) {}
            if (A.tr == TR_AWAY || A.tr == TR_NONE) start_arrival(1);
        }
        if (n > 0 && A.sw_fd >= 0 && (fds[4].revents & (POLLIN | POLLHUP))) switch_done();
        if (A.tr == TR_DEPART && now_s() - A.tr_at >= DEPART_S && A.sw_pid <= 0) {
            render_all();
            send_switch();
        }
        if (A.tr == TR_ARRIVE && now_s() - A.tr_at >= ARRIVE_S) { A.tr = TR_NONE; A.redraw = 1; }
        /* Power-button helpers. SIGCHLD stays default: pam_unix waits for
         * its own unix_chkpwd child, which an ignored SIGCHLD would break. */
        while (waitpid(-1, NULL, WNOHANG) > 0) {}

        time_t s = time(NULL);
        if (s != last_sec) { last_sec = s; read_telemetry(); A.redraw = 1; }
        if (A.unlock_at > 0 && now_s() >= A.unlock_at) {
            render_all();
            ext_session_lock_v1_unlock_and_destroy(A.lock);
            A.lock = NULL;
            wl_display_roundtrip(A.display);
            A.exit_code = 0;
            break;
        }
        if (A.redraw || animating()) render_all();
    }
    wipe_password();
    if (A.lock) {
        /* Leaving without an unlock: the compositor keeps the screen locked. */
        ext_session_lock_v1_destroy(A.lock);
        wl_display_flush(A.display);
    }
    return A.exit_code;
}

int main(int argc, char **argv) {
    if (argc > 1 && !strcmp(argv[1], "--preview")) return preview(argc, argv);
    int daemonize = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--daemonize")) daemonize = 1;
        else if (!strcmp(argv[i], "--boot")) A.boot = 1;
        else if (!strcmp(argv[i], "--switch-to") && i + 1 < argc) snprintf(A.switch_to, sizeof A.switch_to, "%s", argv[++i]);
        else {
            fprintf(stderr, "usage: nexa-locker [--daemonize] [--boot] [--switch-to NAME] | --preview W H FILE.png [STATE]\n");
            return 1;
        }
    }
    if (A.switch_to[0]) {
        /* Only someone nexa-sessiond would switch to: otherwise say why and
         * lock nothing. */
        who_am_i();
        load_people();
        int found = 0;
        for (int i = 0; i < A.npeople; i++) found |= !strcmp(A.people[i].name, A.switch_to);
        if (!found) {
            char out[256];
            if (session_request("people\n", out, sizeof out) < 0)
                fprintf(stderr, "nexa-locker: switching isn't available (nexa-sessiond isn't running)\n");
            else if (!strcmp(A.switch_to, A.user))
                fprintf(stderr, "nexa-locker: that's you\n");
            else
                fprintf(stderr, "nexa-locker: %s can't use this computer's desktop\n", A.switch_to);
            return 3;
        }
    }
    prctl(PR_SET_DUMPABLE, 0);
    signal(SIGPIPE, SIG_IGN);
    signal(SIGINT, SIG_IGN);
    signal(SIGHUP, SIG_IGN);
    signal(SIGCHLD, SIG_DFL);
    signal(SIGUSR2, SIG_IGN);  /* the watchdog ignores it; the lock screen takes it (signalfd) */
    if (!daemonize) return run_locker();

    /* The caller waits until the screen is locked; a watchdog stays with the
     * lock screen and, if it dies while locked, hands the lock to swaylock. */
    int ready[2], watch[2];
    if (pipe2(ready, O_CLOEXEC) < 0 || pipe2(watch, O_CLOEXEC) < 0) return 1;
    pid_t pid = fork();
    if (pid < 0) return 1;
    if (pid > 0) {
        close(ready[1]);
        close(watch[0]);
        close(watch[1]);
        char c = 0;
        ssize_t r;
        do r = read(ready[0], &c, 1); while (r < 0 && errno == EINTR);
        return r == 1 && c == 'L' ? 0 : 2;
    }
    close(ready[0]);
    setsid();
    pid_t locker = fork();
    if (locker < 0) _exit(1);
    if (locker == 0) {
        close(watch[0]);
        A.ready_fd = ready[1];
        A.watch_fd = watch[1];
        _exit(run_locker());
    }
    close(ready[1]);
    close(watch[1]);
    char c = 0;
    int was_locked = read(watch[0], &c, 1) == 1 && c == 'L';
    close(watch[0]);
    int status = 0;
    while (waitpid(locker, &status, 0) < 0 && errno == EINTR) {}
    if (WIFEXITED(status) && (WEXITSTATUS(status) == 0 || WEXITSTATUS(status) == 2)) _exit(WEXITSTATUS(status));
    if (!was_locked) _exit(1);
    /* Died while locked: the compositor keeps the session locked, and
     * swaylock takes the lock over. For a moment the compositor still
     * counts the dead client's lock and refuses a new one, so retry. */
    openlog("nexa-locker", 0, LOG_USER);
    syslog(LOG_ERR, "the lock screen ended unexpectedly (status %d); swaylock takes over the lock", status);
    for (int attempt = 0; attempt < 40; attempt++) {
        struct timespec t0, t1;
        clock_gettime(CLOCK_MONOTONIC, &t0);
        pid_t sl = fork();
        if (sl == 0) {
            execlp("swaylock", "swaylock", (char *)NULL);
            _exit(127);
        }
        if (sl < 0) break;
        int st = 0;
        while (waitpid(sl, &st, 0) < 0 && errno == EINTR) {}
        clock_gettime(CLOCK_MONOTONIC, &t1);
        if (WIFEXITED(st) && WEXITSTATUS(st) == 0) _exit(0);      /* unlocked through swaylock */
        if (WIFEXITED(st) && WEXITSTATUS(st) == 127) break;        /* no swaylock */
        if (t1.tv_sec - t0.tv_sec > 5) attempt = 0;                /* it had the lock, then died */
        usleep(250000);
    }
    syslog(LOG_ERR, "swaylock couldn't take the lock over; the session stays locked");
    _exit(1);
}
