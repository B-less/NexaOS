#include "layers.h"

#include <ctype.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int strata_target(int cur, int step)
{
    if (cur < 1 || cur > STRATA_LAYERS) cur = 1;
    int t = cur + step;
    if (t < 1) t = 1;
    if (t > STRATA_LAYERS) t = STRATA_LAYERS;
    return t;
}

int strata_step(const char *s)
{
    if (!s || !*s) return 0;
    const char *p = s;
    if (*p == '+' || *p == '-') p++;
    if (!*p || strlen(p) > 2) return 0;
    for (const char *q = p; *q; q++)
        if (!isdigit((unsigned char)*q)) return 0;
    int v = atoi(p);
    return *s == '-' ? -v : v;
}

static double hue_part(double p, double q, double t)
{
    if (t < 0) t += 1;
    if (t > 1) t -= 1;
    if (t < 1.0 / 6) return p + (q - p) * 6 * t;
    if (t < 0.5) return q;
    if (t < 2.0 / 3) return p + (q - p) * (2.0 / 3 - t) * 6;
    return p;
}

strata_rgb strata_tint(strata_rgb a, int n)
{
    if (n < 1) n = 1;
    double mx = fmax(a.r, fmax(a.g, a.b)), mn = fmin(a.r, fmin(a.g, a.b));
    double l = (mx + mn) / 2, h = 0, s = 0, d = mx - mn;
    if (d < 1e-9) return a;  /* grey: no hue to turn */
    s = l > 0.5 ? d / (2 - mx - mn) : d / (mx + mn);
    if (mx == a.r) h = (a.g - a.b) / d + (a.g < a.b ? 6 : 0);
    else if (mx == a.g) h = (a.b - a.r) / d + 2;
    else h = (a.r - a.g) / d + 4;
    h /= 6;
    h = fmod(h + (n - 1) * STRATA_HUE_STEP / 360.0, 1.0);
    double q = l < 0.5 ? l * (1 + s) : l + s - l * s, p = 2 * l - q;
    return (strata_rgb){hue_part(p, q, h + 1.0 / 3), hue_part(p, q, h), hue_part(p, q, h - 1.0 / 3)};
}

void strata_conf_parse(struct strata_conf *c, const char *text)
{
    c->show = 1;
    c->tint = STRATA_TINT;
    if (!text) return;
    const char *line = text;
    while (*line) {
        const char *end = strchr(line, '\n');
        size_t n = end ? (size_t)(end - line) : strlen(line);
        char buf[128];
        if (n < sizeof buf && *line != '#') {
            memcpy(buf, line, n);
            buf[n] = 0;
            char *eq = strchr(buf, '=');
            if (eq) {
                *eq = 0;
                const char *k = buf, *v = eq + 1;
                if (!strcmp(k, "show")) {
                    c->show = strcmp(v, "no") != 0;
                } else if (!strcmp(k, "tint")) {
                    char *e;
                    double t = strtod(v, &e);
                    if (e != v && isfinite(t)) c->tint = fmin(STRATA_TINT_MAX, fmax(0, t));
                }
            }
        }
        if (!end) break;
        line = end + 1;
    }
}

struct strata_sheet strata_sheet_at(int i, double pos)
{
    double z = i - pos;
    struct strata_sheet s;
    if (z >= 0) {  /* the current layer and the deeper ones: up, smaller, fainter */
        s.scale = pow(0.84, z);
        s.rise = 30 * z;
        s.alpha = fmax(0.25, 1 - 0.17 * z);
    } else {       /* layers dived past: fall towards you and fade */
        s.scale = 1 - 0.3 * z;
        s.rise = 46 * z;
        s.alpha = fmax(0, 1 + 1.25 * z);
    }
    return s;
}

double strata_ease(double t)
{
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    double u = 1 - t;
    return 1 - u * u * u;
}
