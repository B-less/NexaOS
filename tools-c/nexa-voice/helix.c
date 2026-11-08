/* The voice helix and voiceprint. See helix.h. */
#include "helix.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

/* ---------- voiceprint ---------- */

void vp_init(vp_state *v, int rate)
{
    memset(v, 0, sizeof *v);
    v->hop = rate > 100 ? rate / 100 : 1;
    v->since = 1000;
}

int vp_feed(vp_state *v, const int16_t *s, size_t n, vp_frame *out, int cap)
{
    int made = 0;
    for (size_t i = 0; i < n; i++) {
        double x = s[i] / 32768.0;
        v->sum2 += x * x;
        if ((s[i] >= 0) != (v->last >= 0)) v->crossings++;
        v->last = s[i];
        if (++v->fill < v->hop) continue;
        double rms = sqrt(v->sum2 / v->hop);
        double db = 20 * log10(rms + 1e-9);
        double level = (db + 52) / 40;             /* -52 dB silent .. -12 dB loud */
        level = level < 0 ? 0 : level > 1 ? 1 : level;
        double zcr = (double)v->crossings / v->hop;
        double bright = zcr * 4;
        bright = bright > 1 ? 1 : bright;
        /* a syllable: loud enough, clearly above the recent average, rising,
         * and not straight after the last one */
        double onset = 0;
        v->since++;
        if (level > 0.3 && level - v->slow > 0.1 && level > v->prev && v->since > 11) {
            onset = level;
            v->since = 0;
        }
        v->slow = v->slow * 0.9 + level * 0.1;
        v->prev = level;
        if (made < cap) out[made] = (vp_frame){(float)level, (float)onset, level > 0.15 ? (float)bright : 0};
        made++;
        v->fill = 0;
        v->sum2 = 0;
        v->crossings = 0;
    }
    return made < cap ? made : cap;
}

/* ---------- the helix ---------- */

static double rnd(helix *h)
{
    h->rng = h->rng * 1103515245u + 12345u;
    return ((h->rng >> 8) & 0xFFFF) / 65536.0;
}

void helix_init(helix *h, double accent_hue, unsigned seed)
{
    memset(h, 0, sizeof *h);
    h->rng = seed ? seed : 1;
    h->accent_hue = accent_hue;
    h->hue = accent_hue;
    h->animations = 1;
}

void helix_pop(helix *h, double strength, double bright)
{
    h->pops_total++;
    double u = 0.2 + rnd(h) * 0.6;
    int slot = 0;
    double oldest = -1;
    for (int i = 0; i < HELIX_POPS; i++) {
        if (!h->pop[i].used) { slot = i; break; }
        if (h->pop[i].age > oldest) { oldest = h->pop[i].age; slot = i; }
    }
    h->pop[slot].u = u;
    h->pop[slot].s = strength;
    h->pop[slot].age = 0;
    h->pop[slot].used = 1;
    h->pitch += (bright * 2 - h->pitch) * 0.6;
    if (!h->animations) return;
    for (int k = 0, made = 0; k < HELIX_SPARKS && made < 7; k++) {
        if (h->spark[k].used) continue;
        double up = rnd(h) < 0.5 ? -1 : 1;
        h->spark[k].u = u;
        h->spark[k].y = 0;
        h->spark[k].vx = (rnd(h) - 0.5) * 90;
        h->spark[k].vy = up * (50 + rnd(h) * 130);
        h->spark[k].life = 0;
        h->spark[k].max = 0.6 + rnd(h) * 0.5;
        h->spark[k].hue = h->hue + (up > 0 ? 150 : 0) + u * 70;
        h->spark[k].used = 1;
        made++;
    }
}

void helix_step(helix *h, double dt, double level, double onset, double bright)
{
    if (dt < 0) dt = 0;
    if (dt > 0.1) dt = 0.1;
    h->t += dt;
    double t = h->t, target = level;
    switch (h->mode) {
    case HELIX_IDLE: target = fmax(target, 0.05 + 0.035 * sin(t * 1.7)); break;
    case HELIX_THINKING: target = fmax(target, 0.16 + 0.05 * sin(t * 6)); break;
    case HELIX_LISTENING: target = fmax(target, 0.06 + 0.03 * sin(t * 3.1)); break;
    case HELIX_SPEAKING: break;
    }
    double rate = target > h->env ? 30 : 8;
    if (!h->animations) rate *= 0.5;
    h->env += (target - h->env) * fmin(1, dt * rate);
    if (h->env > h->env_peak) h->env_peak = h->env;
    if (onset > 0) helix_pop(h, onset, bright);
    h->pitch *= pow(0.4, dt);
    if (h->animations) {
        static const double speed[] = {0.6, 1.0, 4.2, 1.3};
        h->rot += (speed[h->mode] + h->env * 4) * dt;
    }
    double drift = h->animations ? 1 : 0.25;
    switch (h->palette) {
    case HELIX_VOICE: h->hue += dt * drift * (10 + h->env * 110 + h->pitch * 60); break;
    case HELIX_CYCLE: h->hue += dt * drift * 45; break;
    case HELIX_CALM: h->hue = h->accent_hue + 18 * sin(t * 0.4 * drift) + h->env * 25; break;
    case HELIX_EMBER: h->hue = 12 + 22 * sin(t * 0.5 * drift) + h->env * 30; break;
    }
    h->hue = fmod(h->hue, 360);
    for (int i = 0; i < HELIX_POPS; i++) {
        if (!h->pop[i].used) continue;
        h->pop[i].age += dt;
        if (h->pop[i].age > 1.2) h->pop[i].used = 0;
    }
    for (int i = 0; i < HELIX_SPARKS; i++) {
        if (!h->spark[i].used) continue;
        h->spark[i].life += dt;
        h->spark[i].y += h->spark[i].vy * dt;
        h->spark[i].u += h->spark[i].vx * dt / 500;
        h->spark[i].vy *= pow(0.97, dt * 60);
        if (h->spark[i].life >= h->spark[i].max) h->spark[i].used = 0;
    }
}

/* how much the pops lift the helix at u */
static double pop_at(const helix *h, double u)
{
    double s = 0;
    for (int i = 0; i < HELIX_POPS; i++) {
        if (!h->pop[i].used) continue;
        double a = h->pop[i].age;
        double k = a < 0.07 ? a / 0.07 : exp(-(a - 0.07) * 5);
        double d = (u - h->pop[i].u) / 0.08;
        s += h->pop[i].s * k * exp(-d * d);
    }
    return s;
}

static void hsl(double hdeg, double s, double l, double *r, double *g, double *b)
{
    double hh = fmod(fmod(hdeg, 360) + 360, 360) / 60;
    double c = (1 - fabs(2 * l - 1)) * s, x = c * (1 - fabs(fmod(hh, 2) - 1)), m = l - c / 2;
    double rr = 0, gg = 0, bb = 0;
    if (hh < 1) { rr = c; gg = x; }
    else if (hh < 2) { rr = x; gg = c; }
    else if (hh < 3) { gg = c; bb = x; }
    else if (hh < 4) { gg = x; bb = c; }
    else if (hh < 5) { rr = x; bb = c; }
    else { rr = c; bb = x; }
    *r = rr + m;
    *g = gg + m;
    *b = bb + m;
}

static void strand_hue(const helix *h, double u, int strand, double *hue, double *span)
{
    int narrow = h->palette == HELIX_CALM || h->palette == HELIX_EMBER;
    double opp = h->palette == HELIX_CALM ? 40 : h->palette == HELIX_EMBER ? 30 : 150;
    *span = narrow ? 40 : 80;
    *hue = h->hue + (strand ? opp : 0) + u * *span;
}

void helix_colour(const helix *h, double u, int strand, double *r, double *g, double *b)
{
    double hue, span;
    strand_hue(h, u, strand, &hue, &span);
    hsl(hue, 0.85, 0.62, r, g, b);
}

typedef struct { double x, y, z, u, pk; int strand; } node;
typedef struct { int kind; double z; int a, b; } item;   /* 0 rung, 1 segment, 2 node */

#define N 60

static int by_depth(const void *a, const void *b)
{
    double d = ((const item *)a)->z - ((const item *)b)->z;
    return d < 0 ? -1 : d > 0;
}

static void set(cairo_t *cr, const helix *h, double u, int strand, double light, double alpha)
{
    double hue, span, r, g, b;
    strand_hue(h, u, strand, &hue, &span);
    hsl(hue, 0.85, fmin(0.9, light), &r, &g, &b);
    cairo_set_source_rgba(cr, r, g, b, alpha);
}

void helix_draw(const helix *h, cairo_t *cr, double w, double hgt)
{
    node nd[2 * N];
    item it[2 * N + 2 * (N - 1) + N];
    int ni = 0;
    double m = w * 0.08, span = w - 2 * m, cy = hgt / 2;
    double twists = h->mode == HELIX_THINKING ? 4 : 3;
    double a0 = hgt * 0.06 + h->env * hgt * 0.30;
    double flutter = 1;
    if (h->mode == HELIX_SPEAKING && h->animations)
        flutter += (sin(h->t * 53) * 0.05 + sin(h->t * 31) * 0.04) * h->env;
    for (int i = 0; i < N; i++) {
        double u = (double)i / (N - 1), ph = u * twists * 2 * M_PI - h->rot;
        double taper = 0.25 + 0.75 * pow(sin(M_PI * u), 0.7);
        double pk = pop_at(h, u), amp = (a0 * taper + pk * hgt * 0.22) * flutter;
        double s = sin(ph), z = cos(ph), x = m + u * span;
        nd[i] = (node){x, cy + amp * s, z, u, pk, 0};
        nd[N + i] = (node){x, cy - amp * s, -z, u, pk, 1};
        if (i % 2 == 0) it[ni++] = (item){0, -1.2, i, N + i};
    }
    for (int i = 0; i < N - 1; i++) {
        it[ni++] = (item){1, (nd[i].z + nd[i + 1].z) / 2, i, i + 1};
        it[ni++] = (item){1, (nd[N + i].z + nd[N + i + 1].z) / 2, N + i, N + i + 1};
    }
    for (int i = 0; i < 2 * N; i++) it[ni++] = (item){2, nd[i].z + 0.01, i, i};
    qsort(it, ni, sizeof *it, by_depth);
    double light = 0.58 + h->env * 0.10;
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
    for (int k = 0; k < ni; k++) {
        const item *e = &it[k];
        if (e->kind == 0) {
            const node *a = &nd[e->a], *b = &nd[e->b];
            double al = 0.18 + 0.3 * h->env, mx = (a->x + b->x) / 2, my = (a->y + b->y) / 2;
            cairo_set_line_width(cr, 1.4);
            set(cr, h, a->u, 0, light, al);
            cairo_move_to(cr, a->x, a->y);
            cairo_line_to(cr, mx, my);
            cairo_stroke(cr);
            set(cr, h, b->u, 1, light, al);
            cairo_move_to(cr, mx, my);
            cairo_line_to(cr, b->x, b->y);
            cairo_stroke(cr);
        } else if (e->kind == 1) {
            const node *p = &nd[e->a], *q = &nd[e->b];
            double d = (e->z + 1) / 2;
            cairo_set_line_width(cr, 1.2 + 2.8 * d);
            set(cr, h, p->u, p->strand, light - 0.08 + d * 0.10, 0.25 + 0.75 * d);
            cairo_move_to(cr, p->x, p->y);
            cairo_line_to(cr, q->x, q->y);
            cairo_stroke(cr);
        } else {
            const node *p = &nd[e->a];
            double d = (p->z + 1) / 2, r = (1.6 + 2.8 * d) * (1 + p->pk * 1.3 + h->env * 0.35);
            if (d > 0.6 && (h->env > 0.25 || p->pk > 0.2)) {
                set(cr, h, p->u, p->strand, light + 0.08, 0.12 * d + 0.15 * fmin(1, p->pk));
                cairo_arc(cr, p->x, p->y, r * 2.6, 0, 2 * M_PI);
                cairo_fill(cr);
            }
            set(cr, h, p->u, p->strand, light + 0.06 * d, 0.35 + 0.65 * d);
            cairo_arc(cr, p->x, p->y, r, 0, 2 * M_PI);
            cairo_fill(cr);
        }
    }
    for (int i = 0; i < HELIX_SPARKS; i++) {
        if (!h->spark[i].used) continue;
        double a = 1 - h->spark[i].life / h->spark[i].max, r, g, b;
        if (a <= 0) continue;
        hsl(h->spark[i].hue, 0.9, 0.68, &r, &g, &b);
        cairo_set_source_rgba(cr, r, g, b, a * 0.9);
        cairo_arc(cr, m + h->spark[i].u * span, cy + h->spark[i].y, 1.2 + 2 * a, 0, 2 * M_PI);
        cairo_fill(cr);
    }
}
