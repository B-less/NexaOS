#include "model.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "json.h"

int parse_volume(const char *out, int *percent, int *muted)
{
    const char *p = out ? strstr(out, "Volume:") : NULL;
    double v;
    if (!p || sscanf(p + 7, " %lf", &v) != 1 || v < 0) return -1;
    *percent = (int)(v * 100 + 0.5);
    *muted = strstr(p, "[MUTED]") != NULL;
    return 0;
}

/* mako fields are {"type": "s", "data": value} (or the bare value). */
static int field(const struct jdoc *d, int e, const char *key)
{
    int f = json_get(d, e, key);
    if (f >= 0 && d->n[f].type == J_OBJ) return json_get(d, f, "data");
    return f;
}

void utf8_cut(char *s)
{
    size_t n = strlen(s), i = n;
    /* back to the start of the last character; drop it if it is cut short */
    while (i > 0 && ((unsigned char)s[i - 1] & 0xc0) == 0x80) i--;
    if (i == 0) return;
    unsigned char lead = (unsigned char)s[i - 1];
    size_t want = lead >= 0xf0 ? 4 : lead >= 0xe0 ? 3 : lead >= 0xc0 ? 2 : 1;
    if (n - (i - 1) < want) s[i - 1] = 0;
}

static void copy_field(const struct jdoc *d, int e, const char *key, char *out, size_t cap)
{
    int f = field(d, e, key);
    snprintf(out, cap, "%s", f >= 0 && d->n[f].type == J_STR ? d->n[f].str : "");
    utf8_cut(out);
}

static int cmp_newest(const void *a, const void *b) { return ((const struct note *)b)->id - ((const struct note *)a)->id; }

int parse_mako(const char *json, int cleared_up_to, struct note *notes, int count, int max)
{
    if (!json) return count;
    char *text = strdup(json);
    struct jdoc d = {0};
    if (!text || json_parse(&d, text) != 0) {
        json_free(&d);
        free(text);
        return count;
    }
    int list = json_get(&d, 0, "data");
    if (list >= 0 && d.n[list].type == J_ARR && d.n[list].child >= 0 && d.n[d.n[list].child].type == J_ARR)
        list = d.n[list].child;  /* "data": [[ ... ]] */
    for (int e = list >= 0 && d.n[list].type == J_ARR ? d.n[list].child : -1; e >= 0 && count < max; e = d.n[e].next) {
        if (d.n[e].type != J_OBJ) continue;
        int f = field(&d, e, "id"), id = f >= 0 && d.n[f].type == J_NUM ? (int)d.n[f].num : 0;
        if (id && id <= cleared_up_to) continue;
        int dup = 0;
        for (int i = 0; i < count && id; i++) dup |= notes[i].id == id;
        if (dup) continue;
        struct note *n = &notes[count++];
        n->id = id;
        copy_field(&d, e, "app-name", n->app, sizeof n->app);
        copy_field(&d, e, "summary", n->summary, sizeof n->summary);
        copy_field(&d, e, "body", n->body, sizeof n->body);
    }
    json_free(&d);
    free(text);
    qsort(notes, (size_t)count, sizeof *notes, cmp_newest);
    return count;
}

static long read_long(const char *dir, const char *name)
{
    char path[600], buf[64];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    FILE *f = fopen(path, "re");
    if (!f) return -1;
    long v = -1;
    if (fgets(buf, sizeof buf, f)) v = strtol(buf, NULL, 10);
    fclose(f);
    return v;
}

static void read_text(const char *dir, const char *name, char *out, size_t cap)
{
    char path[600];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    out[0] = 0;
    FILE *f = fopen(path, "re");
    if (!f) return;
    if (fgets(out, (int)cap, f)) out[strcspn(out, "\n")] = 0;
    fclose(f);
}

int battery_read(const char *root, struct battery *b)
{
    char dir[400];
    for (int i = 0; i < 4; i++) {
        snprintf(dir, sizeof dir, "%s/BAT%d", root, i);
        long cap = read_long(dir, "capacity");
        if (cap < 0) continue;
        char status[32];
        read_text(dir, "status", status, sizeof status);
        b->percent = (int)(cap > 100 ? 100 : cap);
        b->charging = !strcmp(status, "Charging");
        b->full = !strcmp(status, "Full");
        b->minutes = -1;
        /* energy (uWh / uW) or charge (uAh / uA): the same sum */
        long now = read_long(dir, "energy_now"), full = read_long(dir, "energy_full"), rate = read_long(dir, "power_now");
        if (now < 0 || rate < 0) {
            now = read_long(dir, "charge_now");
            full = read_long(dir, "charge_full");
            rate = read_long(dir, "current_now");
        }
        if (rate > 0 && now >= 0) {
            if (b->charging && full > now) b->minutes = (int)((full - now) * 60 / rate);
            else if (!b->charging && !b->full) b->minutes = (int)(now * 60 / rate);
            if (b->minutes > 48 * 60) b->minutes = -1;  /* a rate that small is a reading, not a forecast */
        }
        return 0;
    }
    return -1;
}

static void span(int minutes, char *out, size_t cap)
{
    if (minutes >= 60) snprintf(out, cap, "%d h %d min", minutes / 60, minutes % 60);
    else snprintf(out, cap, "%d min", minutes);
}

void battery_text(const struct battery *b, char *out, size_t cap)
{
    char s[32];
    if (b->full) snprintf(out, cap, "Full");
    else if (b->charging && b->minutes >= 0) span(b->minutes, s, sizeof s), snprintf(out, cap, "Full in %s", s);
    else if (b->charging) snprintf(out, cap, "Charging");
    else if (b->minutes >= 0) span(b->minutes, s, sizeof s), snprintf(out, cap, "%s left", s);
    else out[0] = 0;
}

int backlight_read(const char *root)
{
    char dir[400];
    static const char *const names[] = {"intel_backlight", "amdgpu_bl0", "amdgpu_bl1", "acpi_video0", "nv_backlight", "backlight"};
    for (size_t i = 0; i < sizeof names / sizeof *names; i++) {
        snprintf(dir, sizeof dir, "%s/%s", root, names[i]);
        long cur = read_long(dir, "brightness"), max = read_long(dir, "max_brightness");
        if (cur >= 0 && max > 0) return (int)((cur * 100 + max / 2) / max);
    }
    return -1;
}

int signal_percent(int raw)
{
    double v = 2 * (raw / 100.0 + 100);
    return v < 0 ? 0 : v > 100 ? 100 : (int)v;
}

int signal_bars(int percent) { return percent >= 75 ? 4 : percent >= 50 ? 3 : percent >= 25 ? 2 : percent > 0 ? 1 : 0; }

int tunnel_ok(const char *n)
{
    size_t len = n ? strlen(n) : 0;
    if (len == 0 || len > 15) return 0;
    for (size_t i = 0; i < len; i++) {
        char c = n[i];
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || c == '.' || c == '-')) return 0;
    }
    return 1;
}

int broker_request(char *out, size_t cap, const char *action, const char *name, int percent)
{
    for (const char *c = action; *c; c++)
        if (!((*c >= 'a' && *c <= 'z') || *c == '_')) return -1;
    if (name && !tunnel_ok(name)) return -1;
    int r;
    if (name) r = snprintf(out, cap, "{\"action\": \"%s\", \"args\": {\"name\": \"%s\"}}\n", action, name);
    else if (percent >= 0) r = snprintf(out, cap, "{\"action\": \"%s\", \"args\": {\"percent\": %d}}\n", action, percent > 100 ? 100 : percent);
    else r = snprintf(out, cap, "{\"action\": \"%s\", \"args\": {}}\n", action);
    return r < 0 || (size_t)r >= cap ? -1 : 0;
}

int broker_ok(const char *reply, char *err, size_t cap)
{
    char *text = reply ? strdup(reply) : NULL;
    struct jdoc d = {0};
    int ok = 0;
    if (err && cap) err[0] = 0;
    if (text && json_parse(&d, text) == 0) {
        ok = json_num(&d, 0, "ok", 0) != 0;
        if (!ok && err) snprintf(err, cap, "%s", json_str(&d, 0, "error", "the system service said no"));
    } else if (err) {
        snprintf(err, cap, "the system service gave no answer");
    }
    json_free(&d);
    free(text);
    return ok;
}

int parse_tunnels(const char *reply, struct tunnel *out, int max)
{
    char *text = reply ? strdup(reply) : NULL;
    struct jdoc d = {0};
    int n = -1;
    if (text && json_parse(&d, text) == 0 && json_num(&d, 0, "ok", 0)) {
        int list = json_get(&d, json_get(&d, 0, "result"), "tunnels");
        n = 0;
        for (int t = list >= 0 && d.n[list].type == J_ARR ? d.n[list].child : -1; t >= 0 && n < max; t = d.n[t].next) {
            const char *name = json_str(&d, t, "name", "");
            if (!tunnel_ok(name)) continue;
            snprintf(out[n].name, sizeof out[n].name, "%s", name);
            out[n++].active = json_num(&d, t, "active", 0) != 0;
        }
    }
    json_free(&d);
    free(text);
    return n;
}
