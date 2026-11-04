#include "model.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Nexus is the mark at the top of the rail, so it isn't in this list.
 * foot has had two desktop ids (org.codeberg.dnkl.foot before 3.24); the
 * window's id is mapped by alias_of. */
static const char *const DEFAULTS[] = {"nexa-files", "nexa-browser", "foot", "nexa-settings", "nexa-store",
                                       "nexa-ai", "nexa-notes", "nexa-music"};

void conf_defaults(struct orbit_conf *c)
{
    memset(c, 0, sizeof *c);
    for (size_t i = 0; i < sizeof DEFAULTS / sizeof *DEFAULTS; i++)
        snprintf(c->pinned[c->npinned++], ORBIT_ID_LEN, "%s", DEFAULTS[i]);
}

int id_ok(const char *id)
{
    size_t n = id ? strlen(id) : 0;
    if (n == 0 || n >= ORBIT_ID_LEN || id[0] == '.' || id[0] == '-') return 0;
    for (size_t i = 0; i < n; i++) {
        char ch = id[i];
        if (!((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') || ch == '.' || ch == '_' ||
              ch == '-'))
            return 0;
    }
    return 1;
}

int conf_find(const struct orbit_conf *c, const char *id)
{
    for (int i = 0; i < c->npinned; i++)
        if (!strcmp(c->pinned[i], id)) return i;
    return -1;
}

int conf_pin(struct orbit_conf *c, const char *id)
{
    if (!id_ok(id) || c->npinned >= ORBIT_MAX_PINNED || conf_find(c, id) >= 0) return -1;
    snprintf(c->pinned[c->npinned++], ORBIT_ID_LEN, "%s", id);
    return 0;
}

int conf_unpin(struct orbit_conf *c, const char *id)
{
    int i = conf_find(c, id);
    if (i < 0) return -1;
    memmove(c->pinned[i], c->pinned[i + 1], (size_t)(c->npinned - i - 1) * ORBIT_ID_LEN);
    c->npinned--;
    return 0;
}

static char *trim(char *s)
{
    while (*s == ' ' || *s == '\t') s++;
    char *e = s + strlen(s);
    while (e > s && (e[-1] == ' ' || e[-1] == '\t' || e[-1] == '\r')) *--e = 0;
    return s;
}

void conf_parse(struct orbit_conf *c, const char *text)
{
    conf_defaults(c);
    if (!text) return;
    char *copy = strdup(text);
    if (!copy) return;
    char *save = NULL;
    for (char *line = strtok_r(copy, "\n", &save); line; line = strtok_r(NULL, "\n", &save)) {
        line = trim(line);
        char *eq = strchr(line, '=');
        if (line[0] == '#' || !eq) continue;
        *eq = 0;
        char *k = trim(line), *v = trim(eq + 1);
        if (!strcmp(k, "pinned")) {
            c->npinned = 0;
            char *s2 = NULL;
            for (char *id = strtok_r(v, ",", &s2); id; id = strtok_r(NULL, ",", &s2))
                conf_pin(c, trim(id));  /* bad or repeated ids are dropped */
        } else if (!strcmp(k, "fold")) {
            c->fold = !strcmp(v, "yes") || !strcmp(v, "true") || !strcmp(v, "1");
        }
    }
    free(copy);
}

int conf_format(const struct orbit_conf *c, char *out, size_t cap)
{
    size_t n = 0;
    int r = snprintf(out, cap, "# Orbit (nexa-orbit): pinned apps in order, and whether it folds away.\npinned=");
    if (r < 0 || (size_t)r >= cap) return -1;
    n = (size_t)r;
    for (int i = 0; i < c->npinned; i++) {
        r = snprintf(out + n, cap - n, "%s%s", i ? "," : "", c->pinned[i]);
        if (r < 0 || (size_t)r >= cap - n) return -1;
        n += (size_t)r;
    }
    r = snprintf(out + n, cap - n, "\nfold=%s\n", c->fold ? "yes" : "no");
    return r < 0 || (size_t)r >= cap - n ? -1 : 0;
}

int parse_stat(const char *line, int *ppid, unsigned long long *ticks)
{
    /* pid (comm) state ppid pgrp session tty tpgid flags minflt cminflt
     * majflt cmajflt utime stime ...: fields after the last ')'. */
    const char *p = line ? strrchr(line, ')') : NULL;
    if (!p) return -1;
    char state;
    int pp;
    unsigned long long ut, st;
    if (sscanf(p + 1, " %c %d %*d %*d %*d %*d %*u %*u %*u %*u %*u %llu %llu", &state, &pp, &ut, &st) != 4) return -1;
    *ppid = pp;
    *ticks = ut + st;
    return 0;
}

double cpu_percent(unsigned long long before, unsigned long long after, double dt, long clk_tck)
{
    if (after < before || dt <= 0 || clk_tck <= 0) return 0;
    return (double)(after - before) / (double)clk_tck / dt * 100.0;
}

int exec_strip(const char *exec, char *out, size_t cap)
{
    size_t n = 0;
    for (const char *p = exec; *p; p++) {
        if (*p == '%') {
            if (p[1] == '%') {
                p++;  /* "%%" is a literal '%' */
            } else {
                if (p[1]) p++;  /* a field code: dropped */
                continue;
            }
        }
        if (n + 1 >= cap) return -1;
        out[n++] = *p;
    }
    if (n >= cap) return -1;
    out[n] = 0;
    return 0;
}

const char *alias_of(const char *app_id)
{
    if (!strcmp(app_id, "foot")) return "org.codeberg.dnkl.foot";
    if (!strcmp(app_id, "org.codeberg.dnkl.foot")) return "foot";
    return NULL;
}
