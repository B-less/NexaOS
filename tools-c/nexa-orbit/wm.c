#include "wm.h"

#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

#include "json.h"

#define MINIMIZED "special:minimized"  /* Hyprland's stand-in for Sway's scratchpad */

int wm_kind(void)
{
    if (getenv("HYPRLAND_INSTANCE_SIGNATURE")) return WM_HYPRLAND;
    if (getenv("SWAYSOCK")) return WM_SWAY;
    return WM_NONE;
}

static int connect_unix(const char *path)
{
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    if (!path || strlen(path) >= sizeof a.sun_path) return -1;
    strcpy(a.sun_path, path);
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    if (connect(fd, (struct sockaddr *)&a, sizeof a) < 0) {
        close(fd);
        return -1;
    }
    return fd;
}

static int write_all(int fd, const void *p, size_t n)
{
    const char *c = p;
    while (n) {
        ssize_t r = write(fd, c, n);
        if (r <= 0) return -1;
        c += r;
        n -= (size_t)r;
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

/* ---------- Sway ---------- */

static char *sway_request(uint32_t type, const char *payload)
{
    int fd = connect_unix(getenv("SWAYSOCK"));
    if (fd < 0) return NULL;
    uint32_t hdr[2] = {(uint32_t)strlen(payload), type}, rh[2];
    char magic[6], *out = NULL;
    if (write_all(fd, "i3-ipc", 6) == 0 && write_all(fd, hdr, 8) == 0 && write_all(fd, payload, hdr[0]) == 0 &&
        read_all(fd, magic, 6) == 0 && read_all(fd, rh, 8) == 0 && rh[0] < (64u << 20)) {
        out = malloc(rh[0] + 1);
        if (out && read_all(fd, out, rh[0]) == 0) {
            out[rh[0]] = 0;
        } else {
            free(out);
            out = NULL;
        }
    }
    close(fd);
    return out;
}

static void lower_copy(char *dst, size_t cap, const char *src)
{
    size_t i = 0;
    for (; src && src[i] && i + 1 < cap; i++) dst[i] = (char)tolower((unsigned char)src[i]);
    dst[i] = 0;
}

static void sway_walk(const struct jdoc *d, int n, int in_scratch, int ws, struct wm_win *out, int max, int *count)
{
    in_scratch = in_scratch || !strcmp(json_str(d, n, "name", ""), "__i3_scratch");
    if (!strcmp(json_str(d, n, "type", ""), "workspace")) ws = (int)json_num(d, n, "num", 0);
    if (in_scratch || ws < 0) ws = 0;
    const char *app = json_str(d, n, "app_id", NULL);
    if (!app || !*app) app = json_str(d, json_get(d, n, "window_properties"), "class", NULL);
    int pid = (int)json_num(d, n, "pid", 0);
    if (app && *app && pid > 0 && *count < max) {
        struct wm_win *w = &out[(*count)++];
        memset(w, 0, sizeof *w);
        lower_copy(w->app_id, sizeof w->app_id, app);
        w->id = (unsigned long)json_num(d, n, "id", 0);
        w->pid = pid;
        w->minimized = in_scratch;
        w->focused = (int)json_num(d, n, "focused", 0);
        w->urgent = (int)json_num(d, n, "urgent", 0);
        w->rank = w->focused ? 0 : 1;
        w->ws = ws;
    }
    static const char *const kids[2] = {"nodes", "floating_nodes"};
    for (int k = 0; k < 2; k++) {
        int a = json_get(d, n, kids[k]);
        if (a < 0) continue;
        for (int c = d->n[a].child; c >= 0; c = d->n[c].next) sway_walk(d, c, in_scratch, ws, out, max, count);
    }
}

/* ---------- Hyprland ---------- */

static char *hypr_request(const char *what)
{
    const char *sig = getenv("HYPRLAND_INSTANCE_SIGNATURE"), *run = getenv("XDG_RUNTIME_DIR");
    if (!sig || !run || strchr(sig, '/')) return NULL;
    char path[512];
    snprintf(path, sizeof path, "%s/hypr/%s/.socket.sock", run, sig);
    int fd = connect_unix(path);
    if (fd < 0) return NULL;
    if (write_all(fd, what, strlen(what)) < 0) {
        close(fd);
        return NULL;
    }
    size_t cap = 1 << 16, n = 0;
    char *b = malloc(cap);
    ssize_t r;
    while (b && (r = read(fd, b + n, cap - n - 1)) > 0) {
        n += (size_t)r;
        if (cap - n < 4096) {
            if (cap >= (32u << 20)) break;
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

static int hypr_ok(const char *what)
{
    char *r = hypr_request(what);
    int ok = r && strstr(r, "ok");
    free(r);
    return ok ? 0 : -1;
}

/* Hyprland only says a window wants attention as an event. */
static unsigned long urgent[16];
static int nurgent;

static void urgent_set(unsigned long a, int on)
{
    for (int i = 0; i < nurgent; i++)
        if (urgent[i] == a) {
            if (!on) urgent[i] = urgent[--nurgent];
            return;
        }
    if (on && nurgent < 16) urgent[nurgent++] = a;
}

static int is_urgent(unsigned long a)
{
    for (int i = 0; i < nurgent; i++)
        if (urgent[i] == a) return 1;
    return 0;
}

static int hypr_windows(struct wm_win *out, int max)
{
    char *cl = hypr_request("j/clients");
    struct jdoc d = {0};
    int count = -1;
    if (cl && json_parse(&d, cl) == 0 && d.n[0].type == J_ARR) {
        count = 0;
        for (int c = d.n[0].child; c >= 0 && count < max; c = d.n[c].next) {
            int pid = (int)json_num(&d, c, "pid", 0);
            const char *cls = json_str(&d, c, "class", "");
            if (pid <= 0 || !*cls) continue;
            struct wm_win *w = &out[count++];
            memset(w, 0, sizeof *w);
            lower_copy(w->app_id, sizeof w->app_id, cls);
            w->id = strtoul(json_str(&d, c, "address", "0x0"), NULL, 16);
            w->pid = pid;
            w->minimized = !strcmp(json_str(&d, json_get(&d, c, "workspace"), "name", ""), MINIMIZED);
            int ws = (int)json_num(&d, json_get(&d, c, "workspace"), "id", 0);
            w->ws = ws > 0 ? ws : 0;
            w->rank = (int)json_num(&d, c, "focusHistoryID", 99);
            w->focused = w->rank == 0;
            w->urgent = is_urgent(w->id);
        }
    }
    json_free(&d);
    free(cl);
    return count;
}

/* ---------- both ---------- */

int wm_windows(struct wm_win *out, int max)
{
    if (wm_kind() == WM_HYPRLAND) return hypr_windows(out, max);
    if (wm_kind() != WM_SWAY) return -1;
    char *tree = sway_request(4, "");
    struct jdoc d = {0};
    int count = -1;
    if (tree && json_parse(&d, tree) == 0) {
        count = 0;
        sway_walk(&d, 0, 0, 0, out, max, &count);
    }
    json_free(&d);
    free(tree);
    return count;
}

int wm_focus(const struct wm_win *w)
{
    char cmd[200];
    if (wm_kind() == WM_SWAY) {
        snprintf(cmd, sizeof cmd, w->minimized ? "[con_id=%lu] scratchpad show, focus" : "[con_id=%lu] focus", w->id);
        char *r = sway_request(0, cmd);
        int ok = r && strstr(r, "\"success\": true");
        free(r);
        return ok ? 0 : -1;
    }
    if (wm_kind() == WM_HYPRLAND) {
        if (w->minimized) {
            /* Silently: a plain movetoworkspace moved the focus (and the
             * pointer) before the window was raised. */
            char *aw = hypr_request("j/activeworkspace");
            struct jdoc d = {0};
            int ws = 1;
            if (aw && json_parse(&d, aw) == 0) ws = (int)json_num(&d, 0, "id", 1);
            json_free(&d);
            free(aw);
            snprintf(cmd, sizeof cmd, "dispatch movetoworkspacesilent %d,address:0x%lx", ws, w->id);
            hypr_ok(cmd);
        }
        snprintf(cmd, sizeof cmd, "dispatch focuswindow address:0x%lx", w->id);
        int r = hypr_ok(cmd);
        /* Every window floats; focusing one doesn't raise it. */
        snprintf(cmd, sizeof cmd, "dispatch alterzorder top,address:0x%lx", w->id);
        hypr_ok(cmd);
        urgent_set(w->id, 0);
        return r;
    }
    return -1;
}

int wm_close(const struct wm_win *w)
{
    char cmd[120];
    if (wm_kind() == WM_SWAY) {
        snprintf(cmd, sizeof cmd, "[con_id=%lu] kill", w->id);
        char *r = sway_request(0, cmd);
        free(r);
        return r ? 0 : -1;
    }
    if (wm_kind() == WM_HYPRLAND) {
        snprintf(cmd, sizeof cmd, "dispatch closewindow address:0x%lx", w->id);
        return hypr_ok(cmd);
    }
    return -1;
}

int wm_event_fd(void)
{
    if (wm_kind() == WM_HYPRLAND) {
        const char *sig = getenv("HYPRLAND_INSTANCE_SIGNATURE"), *run = getenv("XDG_RUNTIME_DIR");
        if (!run || strchr(sig, '/')) return -1;
        char path[512];
        snprintf(path, sizeof path, "%s/hypr/%s/.socket2.sock", run, sig);
        return connect_unix(path);
    }
    if (wm_kind() == WM_SWAY) {
        int fd = connect_unix(getenv("SWAYSOCK"));
        const char *sub = "[\"window\",\"workspace\"]";
        uint32_t hdr[2] = {(uint32_t)strlen(sub), 2};
        if (fd >= 0 && (write_all(fd, "i3-ipc", 6) < 0 || write_all(fd, hdr, 8) < 0 || write_all(fd, sub, hdr[0]) < 0)) {
            close(fd);
            fd = -1;
        }
        return fd;
    }
    return -1;
}

static char linebuf[1024];
static size_t linelen;
static int saw_workspace;

static void hypr_line(const char *l)
{
    if (!strncmp(l, "urgent>>", 8)) urgent_set(strtoul(l + 8, NULL, 16), 1);
    else if (!strncmp(l, "activewindowv2>>", 16)) urgent_set(strtoul(l + 16, NULL, 16), 0);
    else if (!strncmp(l, "closewindow>>", 13)) urgent_set(strtoul(l + 13, NULL, 16), 0);
    else if (!strncmp(l, "workspace>>", 11) || !strncmp(l, "workspacev2>>", 13) || !strncmp(l, "focusedmon>>", 12) ||
             !strncmp(l, "focusedmonv2>>", 14))
        saw_workspace = 1;
}

/* Sway's stream is i3-ipc frames: "i3-ipc", length, type, payload. Only
 * the type matters here (0x80000000: a workspace event). */
static unsigned char sw_hdr[14];
static size_t sw_hlen;
static uint32_t sw_left;

static void sway_bytes(const unsigned char *p, size_t n)
{
    while (n) {
        if (sw_left) {
            size_t k = n < sw_left ? n : sw_left;
            sw_left -= (uint32_t)k;
            p += k;
            n -= k;
            continue;
        }
        sw_hdr[sw_hlen++] = *p++;
        n--;
        if (sw_hlen == sizeof sw_hdr) {
            uint32_t len, type;
            memcpy(&len, sw_hdr + 6, 4);
            memcpy(&type, sw_hdr + 10, 4);
            if (type == 0x80000000u) saw_workspace = 1;
            sw_left = len;
            sw_hlen = 0;
        }
    }
}

int wm_event_read(int fd)
{
    char tmp[8192];
    ssize_t r = read(fd, tmp, sizeof tmp);
    if (r <= 0) {
        sw_hlen = sw_left = 0;
        linelen = 0;
        return -1;
    }
    saw_workspace = 0;
    if (wm_kind() == WM_HYPRLAND) {
        for (ssize_t i = 0; i < r; i++) {
            if (tmp[i] == '\n') {
                linebuf[linelen] = 0;
                hypr_line(linebuf);
                linelen = 0;
            } else if (linelen + 1 < sizeof linebuf) {
                linebuf[linelen++] = tmp[i];
            }
        }
    } else {
        sway_bytes((const unsigned char *)tmp, (size_t)r);
    }
    return saw_workspace ? 2 : 1;
}

/* ---------- Strata ---------- */

int wm_workspace(void)
{
    int num = 0, answered = 0;
    struct jdoc d = {0};
    if (wm_kind() == WM_SWAY) {
        char *r = sway_request(1, "");
        if (r && json_parse(&d, r) == 0 && d.n[0].type == J_ARR) {
            answered = 1;
            for (int c = d.n[0].child; c >= 0; c = d.n[c].next)
                if (json_num(&d, c, "focused", 0)) num = (int)json_num(&d, c, "num", 0);
        }
        free(r);
    } else if (wm_kind() == WM_HYPRLAND) {
        char *r = hypr_request("j/activeworkspace");
        if (r && json_parse(&d, r) == 0 && d.n[0].type == J_OBJ) {
            answered = 1;
            num = (int)json_num(&d, 0, "id", 0);
        }
        free(r);
    }
    json_free(&d);
    if (!answered) return -1;
    return num > 0 ? num : 0;  /* Sway's named workspaces are -1, Hyprland's special ones negative */
}

static int sway_ok(const char *cmd)
{
    char *r = sway_request(0, cmd);
    int ok = r && strstr(r, "\"success\": true");
    free(r);
    return ok ? 0 : -1;
}

int wm_workspace_go(int num)
{
    char cmd[64];
    if (num < 1 || num > 99) return -1;
    if (wm_kind() == WM_SWAY) {
        snprintf(cmd, sizeof cmd, "workspace number %d", num);
        return sway_ok(cmd);
    }
    if (wm_kind() == WM_HYPRLAND) {
        snprintf(cmd, sizeof cmd, "dispatch workspace %d", num);
        return hypr_ok(cmd);
    }
    return -1;
}

int wm_workspace_carry(int num)
{
    char cmd[96];
    if (num < 1 || num > 99) return -1;
    if (wm_kind() == WM_SWAY) {
        /* No window in front (an empty layer): just go. */
        snprintf(cmd, sizeof cmd, "move container to workspace number %d", num);
        sway_ok(cmd);
        return wm_workspace_go(num);
    }
    if (wm_kind() == WM_HYPRLAND) {
        snprintf(cmd, sizeof cmd, "dispatch movetoworkspace %d", num);  /* follows the window */
        return hypr_ok(cmd);
    }
    return -1;
}
