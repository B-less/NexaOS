/* nexa-display: screens for Settings > Display, over the compositor's
 * wlr-output-management protocol (Sway and Hyprland both have it).
 *
 *   nexa-display list          every screen as JSON: name, make, model,
 *                              enabled, the mode in use and all modes,
 *                              scale, rotation, position, an "id"
 *   nexa-display apply [--test] < {"heads": [{"name", "enabled", "width",
 *                              "height", "refresh" (mHz), "scale",
 *                              "transform", "x", "y"}]}
 *                              changes those screens (missing fields and
 *                              screens keep what they have); --test only
 *                              asks the compositor whether it would work
 *   nexa-display apply-saved   applies ~/.config/nexa/displays.json (what
 *                              Settings saved) to the screens connected
 *                              now; run at the start of each session
 *
 * Every reply is one JSON line: the list, or {"ok": true} /
 * {"ok": false, "error": "..."}. The compositor checks each change and
 * refuses what the screen can't do; nothing is written here. */
#define _GNU_SOURCE
#include <errno.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wayland-client.h>
#include "wlr-output-management-unstable-v1-client-protocol.h"
#include "../nexa-living/json.h"

struct mode {
    struct zwlr_output_mode_v1 *obj;
    int width, height, refresh;
    bool preferred;
    struct wl_list link;
};

struct head {
    struct zwlr_output_head_v1 *obj;
    char name[64], description[256], make[64], model[128], serial[64];
    int phys_w, phys_h, x, y, transform;
    bool enabled;
    double scale;
    struct mode *current;
    struct wl_list modes;
    struct wl_list link;
};

static struct zwlr_output_manager_v1 *manager;
static struct wl_list heads;
static uint32_t serial;
static bool done_received;
static int result; /* 0 waiting, 1 succeeded, -1 failed, -2 cancelled */

static const char *TRANSFORMS[] = {"normal", "90", "180", "270", "flipped", "flipped-90", "flipped-180", "flipped-270"};

static void copy(char *dst, size_t n, const char *src) {
    snprintf(dst, n, "%s", src ? src : "");
}

/* ---- mode events ---- */
static void mode_size(void *data, struct zwlr_output_mode_v1 *m, int32_t w, int32_t h) {
    struct mode *mo = data;
    mo->width = w;
    mo->height = h;
}
static void mode_refresh(void *data, struct zwlr_output_mode_v1 *m, int32_t r) { ((struct mode *)data)->refresh = r; }
static void mode_preferred(void *data, struct zwlr_output_mode_v1 *m) { ((struct mode *)data)->preferred = true; }
static void mode_finished(void *data, struct zwlr_output_mode_v1 *m) {}
static const struct zwlr_output_mode_v1_listener mode_listener = {mode_size, mode_refresh, mode_preferred, mode_finished};

/* ---- head events ---- */
static void head_name(void *d, struct zwlr_output_head_v1 *h, const char *s) { copy(((struct head *)d)->name, 64, s); }
static void head_description(void *d, struct zwlr_output_head_v1 *h, const char *s) { copy(((struct head *)d)->description, 256, s); }
static void head_physical_size(void *d, struct zwlr_output_head_v1 *h, int32_t w, int32_t ht) {
    ((struct head *)d)->phys_w = w;
    ((struct head *)d)->phys_h = ht;
}
static void head_mode(void *d, struct zwlr_output_head_v1 *h, struct zwlr_output_mode_v1 *m) {
    struct head *he = d;
    struct mode *mo = calloc(1, sizeof *mo);
    if (!mo) return;
    mo->obj = m;
    wl_list_insert(he->modes.prev, &mo->link);
    zwlr_output_mode_v1_add_listener(m, &mode_listener, mo);
}
static void head_enabled(void *d, struct zwlr_output_head_v1 *h, int32_t e) { ((struct head *)d)->enabled = e; }
static void head_current_mode(void *d, struct zwlr_output_head_v1 *h, struct zwlr_output_mode_v1 *m) {
    struct head *he = d;
    struct mode *mo;
    wl_list_for_each(mo, &he->modes, link) if (mo->obj == m) he->current = mo;
}
static void head_position(void *d, struct zwlr_output_head_v1 *h, int32_t x, int32_t y) {
    ((struct head *)d)->x = x;
    ((struct head *)d)->y = y;
}
static void head_transform(void *d, struct zwlr_output_head_v1 *h, int32_t t) { ((struct head *)d)->transform = t; }
static void head_scale(void *d, struct zwlr_output_head_v1 *h, wl_fixed_t s) { ((struct head *)d)->scale = wl_fixed_to_double(s); }
static void head_finished(void *d, struct zwlr_output_head_v1 *h) {}
static void head_make(void *d, struct zwlr_output_head_v1 *h, const char *s) { copy(((struct head *)d)->make, 64, s); }
static void head_model(void *d, struct zwlr_output_head_v1 *h, const char *s) { copy(((struct head *)d)->model, 128, s); }
static void head_serial(void *d, struct zwlr_output_head_v1 *h, const char *s) { copy(((struct head *)d)->serial, 64, s); }
static void head_adaptive_sync(void *d, struct zwlr_output_head_v1 *h, uint32_t s) {}
static const struct zwlr_output_head_v1_listener head_listener = {
    head_name, head_description, head_physical_size, head_mode, head_enabled, head_current_mode, head_position,
    head_transform, head_scale, head_finished, head_make, head_model, head_serial, head_adaptive_sync,
};

/* ---- manager ---- */
static void manager_head(void *d, struct zwlr_output_manager_v1 *m, struct zwlr_output_head_v1 *h) {
    struct head *he = calloc(1, sizeof *he);
    if (!he) return;
    he->obj = h;
    he->scale = 1.0;
    wl_list_init(&he->modes);
    wl_list_insert(heads.prev, &he->link);
    zwlr_output_head_v1_add_listener(h, &head_listener, he);
}
static void manager_done(void *d, struct zwlr_output_manager_v1 *m, uint32_t s) {
    serial = s;
    done_received = true;
}
static void manager_finished(void *d, struct zwlr_output_manager_v1 *m) {}
static const struct zwlr_output_manager_v1_listener manager_listener = {manager_head, manager_done, manager_finished};

static void reg_global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t version) {
    if (strcmp(iface, zwlr_output_manager_v1_interface.name) == 0) {
        manager = wl_registry_bind(r, name, &zwlr_output_manager_v1_interface, version < 4 ? version : 4);
        zwlr_output_manager_v1_add_listener(manager, &manager_listener, NULL);
    }
}
static void reg_remove(void *d, struct wl_registry *r, uint32_t name) {}
static const struct wl_registry_listener reg_listener = {reg_global, reg_remove};

/* ---- configuration events ---- */
static void cfg_succeeded(void *d, struct zwlr_output_configuration_v1 *c) { result = 1; }
static void cfg_failed(void *d, struct zwlr_output_configuration_v1 *c) { result = -1; }
static void cfg_cancelled(void *d, struct zwlr_output_configuration_v1 *c) { result = -2; }
static const struct zwlr_output_configuration_v1_listener cfg_listener = {cfg_succeeded, cfg_failed, cfg_cancelled};

/* ---- output ---- */
static void json_string(FILE *f, const char *s) {
    fputc('"', f);
    for (; *s; s++) {
        unsigned char c = (unsigned char)*s;
        if (c == '"' || c == '\\') fprintf(f, "\\%c", c);
        else if (c < 0x20) fprintf(f, "\\u%04x", c);
        else fputc(c, f);
    }
    fputc('"', f);
}

/* Stable across sessions and ports: make, model and serial, else the name. */
static void head_id(const struct head *h, char *out, size_t n) {
    if (h->serial[0] || h->make[0] || h->model[0])
        snprintf(out, n, "%s|%s|%s", h->make, h->model, h->serial);
    else
        snprintf(out, n, "%s", h->name);
}

static void print_list(void) {
    struct head *h;
    bool first = true;
    printf("[");
    wl_list_for_each(h, &heads, link) {
        char id[300];
        head_id(h, id, sizeof id);
        printf("%s{\"name\":", first ? "" : ",");
        json_string(stdout, h->name);
        printf(",\"id\":");
        json_string(stdout, id);
        printf(",\"description\":");
        json_string(stdout, h->description);
        printf(",\"make\":");
        json_string(stdout, h->make);
        printf(",\"model\":");
        json_string(stdout, h->model);
        printf(",\"enabled\":%s,\"x\":%d,\"y\":%d,\"scale\":%.4g,\"transform\":\"%s\",\"physical_mm\":[%d,%d]",
               h->enabled ? "true" : "false", h->x, h->y, h->scale,
               TRANSFORMS[h->transform >= 0 && h->transform < 8 ? h->transform : 0], h->phys_w, h->phys_h);
        if (h->current)
            printf(",\"mode\":{\"width\":%d,\"height\":%d,\"refresh\":%d}", h->current->width, h->current->height, h->current->refresh);
        printf(",\"modes\":[");
        struct mode *m;
        bool mf = true;
        wl_list_for_each(m, &h->modes, link) {
            printf("%s{\"width\":%d,\"height\":%d,\"refresh\":%d,\"preferred\":%s}", mf ? "" : ",", m->width, m->height,
                   m->refresh, m->preferred ? "true" : "false");
            mf = false;
        }
        printf("]}");
        first = false;
    }
    printf("]\n");
}

static int fail(const char *msg) {
    printf("{\"ok\":false,\"error\":");
    json_string(stdout, msg);
    printf("}\n");
    return 1;
}

static int transform_of(const char *s) {
    for (int i = 0; i < 8; i++)
        if (strcmp(s, TRANSFORMS[i]) == 0) return i;
    return -1;
}

/* The node (in doc d, array a) that configures head h, or -1. */
static int wanted_for(const struct jdoc *d, int arr, const struct head *h, bool by_id) {
    if (arr < 0) return -1;
    char id[300];
    head_id(h, id, sizeof id);
    for (int c = d->n[arr].child; c >= 0; c = d->n[c].next) {
        const char *key = json_str(d, c, by_id ? "id" : "name", "");
        if (strcmp(key, by_id ? id : h->name) == 0) return c;
    }
    return -1;
}

static int read_all(FILE *f, char **out) {
    size_t cap = 4096, len = 0;
    char *buf = malloc(cap);
    if (!buf) return -1;
    size_t k;
    while ((k = fread(buf + len, 1, cap - len - 1, f)) > 0) {
        len += k;
        if (len + 1 >= cap) {
            if (cap > (1 << 20)) break;
            char *nb = realloc(buf, cap *= 2);
            if (!nb) { free(buf); return -1; }
            buf = nb;
        }
    }
    buf[len] = 0;
    *out = buf;
    return 0;
}

/* Build and send one configuration: every head is configured (the protocol
 * wants all of them), from `arr` where given, else as it is now. */
static int configure(struct wl_display *dpy, const struct jdoc *d, int arr, bool by_id, bool test, int *matched) {
    struct zwlr_output_configuration_v1 *cfg = zwlr_output_manager_v1_create_configuration(manager, serial);
    zwlr_output_configuration_v1_add_listener(cfg, &cfg_listener, NULL);
    int enabled_count = 0;
    struct head *h;
    *matched = 0;
    wl_list_for_each(h, &heads, link) {
        int w = wanted_for(d, arr, h, by_id);
        if (w >= 0) (*matched)++;
        bool on = w >= 0 && json_get(d, w, "enabled") >= 0 ? json_num(d, w, "enabled", 1) != 0 : h->enabled;
        if (!on) {
            zwlr_output_configuration_v1_disable_head(cfg, h->obj);
            continue;
        }
        enabled_count++;
        struct zwlr_output_configuration_head_v1 *ch = zwlr_output_configuration_v1_enable_head(cfg, h->obj);
        int cw = h->current ? h->current->width : 0, chh = h->current ? h->current->height : 0;
        int cr = h->current ? h->current->refresh : 0;
        int width = w >= 0 ? (int)json_num(d, w, "width", cw) : cw;
        int height = w >= 0 ? (int)json_num(d, w, "height", chh) : chh;
        int refresh = w >= 0 ? (int)json_num(d, w, "refresh", cr) : cr;
        /* The listed mode nearest the refresh asked for, else a custom one. */
        struct mode *best = NULL, *m;
        wl_list_for_each(m, &h->modes, link) {
            if (m->width != width || m->height != height) continue;
            if (!best || abs(m->refresh - refresh) < abs(best->refresh - refresh)) best = m;
        }
        if (best) zwlr_output_configuration_head_v1_set_mode(ch, best->obj);
        else if (width > 0 && height > 0) zwlr_output_configuration_head_v1_set_custom_mode(ch, width, height, refresh > 0 ? refresh : 0);
        zwlr_output_configuration_head_v1_set_position(ch, w >= 0 ? (int)json_num(d, w, "x", h->x) : h->x,
                                                       w >= 0 ? (int)json_num(d, w, "y", h->y) : h->y);
        int t = h->transform;
        if (w >= 0) {
            const char *ts = json_str(d, w, "transform", NULL);
            if (ts) {
                t = transform_of(ts);
                if (t < 0) {
                    zwlr_output_configuration_v1_destroy(cfg);
                    return fail("unknown rotation");
                }
            }
        }
        zwlr_output_configuration_head_v1_set_transform(ch, t);
        double scale = w >= 0 ? json_num(d, w, "scale", h->scale) : h->scale;
        if (scale < 0.25 || scale > 4) {
            zwlr_output_configuration_v1_destroy(cfg);
            return fail("scale must be between 0.25 and 4");
        }
        zwlr_output_configuration_head_v1_set_scale(ch, wl_fixed_from_double(scale));
    }
    if (enabled_count == 0) {
        zwlr_output_configuration_v1_destroy(cfg);
        return fail("at least one screen has to stay on");
    }
    result = 0;
    if (test) zwlr_output_configuration_v1_test(cfg);
    else zwlr_output_configuration_v1_apply(cfg);
    while (result == 0 && wl_display_dispatch(dpy) != -1) {}
    zwlr_output_configuration_v1_destroy(cfg);
    if (result == 1) {
        printf("{\"ok\":true,\"tested\":%s}\n", test ? "true" : "false");
        return 0;
    }
    return fail(result == -2 ? "the screens changed meanwhile; try again" : "the compositor refused that setting for this screen");
}

int main(int argc, char **argv) {
    const char *cmd = argc > 1 ? argv[1] : "list";
    bool test = argc > 2 && strcmp(argv[2], "--test") == 0;
    if (strcmp(cmd, "list") && strcmp(cmd, "apply") && strcmp(cmd, "apply-saved")) {
        fprintf(stderr, "usage: nexa-display list | apply [--test] < JSON | apply-saved\n");
        return 2;
    }
    wl_list_init(&heads);
    struct wl_display *dpy = wl_display_connect(NULL);
    if (!dpy) return fail("no Wayland desktop to talk to");
    struct wl_registry *reg = wl_display_get_registry(dpy);
    wl_registry_add_listener(reg, &reg_listener, NULL);
    wl_display_roundtrip(dpy);
    if (!manager) return fail("this desktop doesn't let apps change screens (no wlr-output-management)");
    while (!done_received && wl_display_dispatch(dpy) != -1) {}
    wl_display_roundtrip(dpy); /* every head's and mode's details */

    if (strcmp(cmd, "list") == 0) {
        print_list();
        return 0;
    }
    char *text = NULL;
    bool by_id = strcmp(cmd, "apply-saved") == 0;
    if (by_id) {
        const char *home = getenv("HOME");
        char path[512];
        snprintf(path, sizeof path, "%s/.config/nexa/displays.json", home ? home : "/root");
        FILE *f = fopen(path, "r");
        if (!f) {
            printf("{\"ok\":true,\"applied\":0}\n");
            return 0;
        }
        int rc = read_all(f, &text);
        fclose(f);
        if (rc) return fail("can't read the saved screens");
    } else if (read_all(stdin, &text)) {
        return fail("can't read the request");
    }
    struct jdoc d = {0};
    if (json_parse(&d, text) != 0 || d.count == 0 || d.n[0].type != J_OBJ) return fail("bad JSON");
    int arr = json_get(&d, 0, "heads");
    if (arr >= 0 && d.n[arr].type != J_ARR) return fail("'heads' must be a list");
    int matched = 0;
    if (by_id) {
        /* Nothing saved for these screens: leave them as the compositor set them. */
        struct head *h;
        wl_list_for_each(h, &heads, link) if (wanted_for(&d, arr, h, true) >= 0) matched++;
        if (matched == 0) {
            printf("{\"ok\":true,\"applied\":0}\n");
            return 0;
        }
    }
    int rc = configure(dpy, &d, arr, by_id, test, &matched);
    json_free(&d);
    free(text);
    wl_display_disconnect(dpy);
    return rc;
}
