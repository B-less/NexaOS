/* The ten living-background styles. See styles.h.
 *
 * Every style keeps its cost bounded: a fixed number of items made once
 * (scaled by density and quality), drawn with a handful of cairo paths per
 * frame (items of one brightness share a path), no text rendering, no
 * allocation. Positions come from the time, so a dropped frame just skips
 * ahead. The pointer, when given, nudges or lights what's near it. */
#include "styles.h"

#include <math.h>
#include <string.h>

const char *const style_names[ST_COUNT] = {
    "circuit", "quantum", "neural", "holo-grid", "rain", "nebula", "pulse", "orbital", "geometry", "signature"};
const char *const style_titles[ST_COUNT] = {
    "Cyber Circuit", "Quantum Flow", "Neural Network", "Holographic Grid", "Digital Rain",
    "Cosmic Nebula", "Energy Pulse", "Orbital Field", "Living Geometry", "Nexa Signature"};

int style_by_name(const char *name)
{
    for (int i = 0; i < ST_COUNT; i++)
        if (!strcmp(name, style_names[i])) return i;
    return -1;
}

/* ---------- helpers ---------- */

static unsigned rng_next(unsigned *s)
{
    unsigned x = *s ? *s : 0x9e3779b9u;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    return *s = x;
}
static double rnd(unsigned *s) { return (rng_next(s) & 0xffffff) / (double)0x1000000; }
static double clampd(double v, double lo, double hi) { return v < lo ? lo : v > hi ? hi : v; }
static double frac(double v) { return v - floor(v); }
static rgb mix(rgb a, rgb b, double t) { return (rgb){a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t}; }
static void src(cairo_t *cr, rgb c, double a) { cairo_set_source_rgba(cr, c.r, c.g, c.b, clampd(a, 0, 1)); }

/* How many items: base x density x the quality's factor. */
static int count(double base, double density, int quality, int max)
{
    static const double qf[3] = {0.45, 0.75, 1.0};
    int n = (int)(base * density * qf[quality < 0 ? 0 : quality > 2 ? 2 : quality]);
    return n < 1 ? 1 : n > max ? max : n;
}

/* A point at fraction s (0..1) along polyline i. */
static void along(const struct region *r, int i, double s, double *x, double *y)
{
    double want = s * r->len[i];
    for (int j = r->start[i]; j < r->start[i + 1] - 1; j++) {
        double dx = r->px[j + 1] - r->px[j], dy = r->py[j + 1] - r->py[j], d = hypot(dx, dy);
        if (want <= d || j == r->start[i + 1] - 2) {
            double f = d > 0 ? clampd(want / d, 0, 1) : 0;
            *x = r->px[j] + dx * f;
            *y = r->py[j] + dy * f;
            return;
        }
        want -= d;
    }
    *x = r->px[r->start[i]];
    *y = r->py[r->start[i]];
}

static void measure(struct region *r, int i)
{
    double L = 0;
    for (int j = r->start[i]; j < r->start[i + 1] - 1; j++) L += hypot(r->px[j + 1] - r->px[j], r->py[j + 1] - r->py[j]);
    r->len[i] = (float)L;
}

/* How near the pointer is (1 on it, 0 beyond `reach`). */
static double near(const struct params *p, double x, double y, double reach)
{
    if (p->px < -1e4 || p->reaction <= 0) return 0;
    double d = hypot(x - p->px, y - p->py);
    return d >= reach ? 0 : (1 - d / reach) * p->reaction;
}

/* A point of light: a soft halo and a bright core (two plain circles: a
 * radial gradient per dot cost ten times as much in software). */
static void glow_dot(cairo_t *cr, double x, double y, double rad, rgb c, double a)
{
    cairo_arc(cr, x, y, rad * 0.75, 0, 2 * M_PI);
    src(cr, c, a * 0.18);
    cairo_fill(cr);
    cairo_arc(cr, x, y, rad * 0.32, 0, 2 * M_PI);
    src(cr, mix(c, (rgb){1, 1, 1}, 0.35), a);
    cairo_fill(cr);
}

static void hex(cairo_t *cr, double cx, double cy, double rad, double rot)
{
    cairo_new_sub_path(cr);
    for (int i = 0; i < 6; i++) {
        double a = rot - M_PI / 2 + i * M_PI / 3;
        cairo_line_to(cr, cx + rad * cos(a), cy + rad * sin(a));
    }
    cairo_close_path(cr);
}

/* The parts of a style that never move, drawn once at full strength into
 * the region's cache; each frame copies it at the frame's brightness. */
static cairo_t *cache_begin(struct region *r)
{
    if (r->cache) return NULL;
    r->cache = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, r->w, r->h);
    return cairo_create(r->cache);
}

static void cache_paint(cairo_t *cr, struct region *r, double alpha)
{
    if (!r->cache) return;
    cairo_set_source_surface(cr, r->cache, 0, 0);
    cairo_paint_with_alpha(cr, clampd(alpha, 0, 1));
}

/* ---------- geometry, made once per region ---------- */

static void make_circuit(struct region *r, unsigned *s, double q)
{
    /* PCB-style traces: segments at 0/45/90 degrees on a grid, a pad at the end. */
    double g = 34;
    int n = count(46 * r->w * r->h / 1e6 + 10, r->density, (int)q, MAX_ITEMS), pts = 0;
    r->n = 0;
    for (int i = 0; i < n && pts < MAX_PTS - 12; i++) {
        double x = floor(rnd(s) * r->w / g) * g, y = floor(rnd(s) * r->h / g) * g;
        int dir = (int)(rnd(s) * 8), segs = 2 + (int)(rnd(s) * 5);
        r->start[r->n] = pts;
        r->px[pts] = (float)x;
        r->py[pts++] = (float)y;
        for (int k = 0; k < segs; k++) {
            if (rnd(s) < 0.5) dir = (dir + (rnd(s) < 0.5 ? 1 : 7)) % 8;
            if (dir % 2) dir = (dir + 1) % 8 == 0 ? 0 : dir;  /* mostly straight, some diagonals */
            double step = g * (1 + floor(rnd(s) * 3)), a = dir * M_PI / 4;
            x += cos(a) * step;
            y += sin(a) * step;
            r->px[pts] = (float)x;
            r->py[pts++] = (float)y;
        }
        r->start[r->n + 1] = pts;
        r->k[r->n] = (float)rnd(s);         /* phase */
        r->u[r->n] = (float)(0.08 + rnd(s) * 0.14);  /* pulse speed, lengths per second */
        measure(r, r->n);
        r->n++;
    }
}

static void make_points(struct region *r, unsigned *s, int n)
{
    r->n = n;
    for (int i = 0; i < n; i++) {
        r->x[i] = (float)(rnd(s) * r->w);
        r->y[i] = (float)(rnd(s) * r->h);
        r->u[i] = (float)rnd(s);
        r->v[i] = (float)rnd(s);
        r->k[i] = (float)rnd(s);
    }
}

static void make_signature(struct region *r, unsigned *s, double q)
{
    /* Flowing lines across the region: smooth curves sampled into polylines. */
    int n = count(9, r->density, (int)q, 24), pts = 0;
    r->n = 0;
    for (int i = 0; i < n && pts < MAX_PTS - 64; i++) {
        double y0 = (0.12 + 0.76 * (i + rnd(s) * 0.6) / n) * r->h, amp = (0.05 + rnd(s) * 0.12) * r->h;
        double f1 = 1.2 + rnd(s) * 1.6, f2 = 2.5 + rnd(s) * 2.0, ph = rnd(s) * 6.28;
        r->start[r->n] = pts;
        for (int k = 0; k <= 48; k++) {
            double xx = (k / 48.0) * (r->w + 80) - 40;
            double u = xx / r->w;
            r->px[pts] = (float)xx;
            r->py[pts++] = (float)(y0 + amp * sin(u * f1 * M_PI + ph) + amp * 0.35 * sin(u * f2 * M_PI + ph * 1.7));
        }
        r->start[r->n + 1] = pts;
        r->k[r->n] = (float)rnd(s);
        r->u[r->n] = (float)(0.05 + rnd(s) * 0.08);
        measure(r, r->n);
        r->n++;
    }
    /* nodes on the lines */
    r->m = count(18, r->density, (int)q, 64);
    for (int j = 0; j < r->m; j++) {
        int line = (int)(rnd(s) * r->n);
        double x, y;
        along(r, line, 0.05 + rnd(s) * 0.9, &x, &y);
        r->x[j] = (float)x;
        r->y[j] = (float)y;
        r->v[j] = (float)rnd(s);
    }
}

void region_free(struct region *r)
{
    if (r->cache) cairo_surface_destroy(r->cache);
    r->cache = NULL;
    r->n = 0;
}

void region_prepare(struct region *r, int style, int w, int h, double density, unsigned seed)
{
    if (r->style == style && r->w == w && r->h == h && fabs(r->density - density) < 1e-3 && r->seed == seed && r->n) return;
    region_free(r);
    memset(r, 0, sizeof *r);
    r->style = style;
    r->w = w;
    r->h = h;
    r->density = density;
    r->seed = seed;
    r->last_t = -1;
    unsigned s = seed * 2654435761u + (unsigned)style * 97 + 1;
    double area = w * (double)h / 1e6;  /* in megapixels */
    switch (style) {
    case ST_CIRCUIT: make_circuit(r, &s, 2); break;
    case ST_QUANTUM: make_points(r, &s, count(420 * area + 80, density, 2, MAX_ITEMS)); break;
    case ST_NEURAL: make_points(r, &s, count(40 * area + 14, density, 2, 90)); break;
    case ST_RAIN: make_points(r, &s, count(w / 16.0, density, 2, MAX_ITEMS)); break;
    case ST_NEBULA: make_points(r, &s, count(220 * area + 40, density, 2, MAX_ITEMS)); break;
    case ST_ORBITAL: make_points(r, &s, count(4 * area + 3, density, 2, 9)); break;
    case ST_GEOMETRY: make_points(r, &s, count(6 * area + 4, density, 2, 14)); break;
    case ST_SIGNATURE: make_signature(r, &s, 2); break;
    default: make_points(r, &s, 8); break;
    }
}

/* ---------- the styles ---------- */

static void draw_circuit(cairo_t *cr, struct region *r, const struct params *p)
{
    int n = count(r->n, 1, p->quality, r->n);
    double b = p->bright * p->focus;
    cairo_set_line_width(cr, 1.2);
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
    cairo_set_line_join(cr, CAIRO_LINE_JOIN_ROUND);
    /* the traces, faint, drawn once */
    cairo_t *c2 = cache_begin(r);
    if (c2) {
        cairo_set_line_width(c2, 1.2);
        cairo_set_line_join(c2, CAIRO_LINE_JOIN_ROUND);
        for (int i = 0; i < r->n; i++) {
            cairo_move_to(c2, r->px[r->start[i]], r->py[r->start[i]]);
            for (int j = r->start[i] + 1; j < r->start[i + 1]; j++) cairo_line_to(c2, r->px[j], r->py[j]);
        }
        src(c2, mix(p->a1, p->a2, 0.35), 0.16);
        cairo_stroke(c2);
        cairo_destroy(c2);
    }
    cache_paint(cr, r, b);
    /* pads */
    for (int i = 0; i < n; i++) {
        int e = r->start[i + 1] - 1;
        double pulse = 0.5 + 0.5 * sin(p->t * 1.7 * p->speed + r->k[i] * 6.28);
        cairo_rectangle(cr, r->px[e] - 2.5, r->py[e] - 2.5, 5, 5);
        src(cr, i % 3 ? p->a1 : p->a2, (0.18 + 0.3 * pulse + 0.5 * near(p, r->px[e], r->py[e], 140)) * b);
        cairo_fill(cr);
    }
    /* the pulses running along them, with a tail */
    cairo_set_line_width(cr, 2.0);
    for (int i = 0; i < n; i++) {
        double s0 = frac(p->t * p->speed * r->u[i] * (1 + p->level) + r->k[i]);
        double tail = fmin(0.18, 60 / fmax(r->len[i], 1));
        double x0, y0, x1, y1;
        along(r, i, fmax(0, s0 - tail), &x0, &y0);
        along(r, i, s0, &x1, &y1);
        rgb c = mix(p->a1, p->a2, frac(r->k[i] * 3));
        double a = (0.55 + 0.45 * near(p, x1, y1, 160)) * b;
        cairo_move_to(cr, x0, y0);
        cairo_line_to(cr, x1, y1);
        src(cr, c, a * 0.7);
        cairo_stroke(cr);
        if (p->quality > 0) glow_dot(cr, x1, y1, 7, c, a);
        else {
            cairo_arc(cr, x1, y1, 2, 0, 2 * M_PI);
            src(cr, c, a);
            cairo_fill(cr);
        }
    }
}

static void draw_quantum(cairo_t *cr, struct region *r, const struct params *p)
{
    int n = count(r->n, 1, p->quality, r->n);
    double dt = r->last_t < 0 ? 0 : clampd(p->t - r->last_t, 0, 0.2);
    r->last_t = p->t;
    double b = p->bright * p->focus, v = 38 * p->speed * (1 + 0.8 * p->level);
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
    /* three brightness bands, one path each */
    for (int band = 0; band < 3; band++) {
        for (int i = band; i < n; i += 3) {
            double x = r->x[i], y = r->y[i];
            double ang = M_PI * (sin(x * 0.0065 + p->t * 0.11) + cos(y * 0.0075 - p->t * 0.09) + 0.4 * r->k[i]);
            double vx = cos(ang) * v, vy = sin(ang) * v;
            double pn = near(p, x, y, 130);
            if (pn > 0) {  /* the pointer pushes them gently away */
                double dx = x - p->px, dy = y - p->py, d = fmax(hypot(dx, dy), 1);
                vx += dx / d * 90 * pn;
                vy += dy / d * 90 * pn;
            }
            double nx = x + vx * dt, ny = y + vy * dt;
            r->u[i] += (float)(dt * 0.12);
            if (nx < -5 || ny < -5 || nx > r->w + 5 || ny > r->h + 5 || r->u[i] > 1) {
                nx = fmod(r->k[i] * 9973 + p->t * 37, r->w);
                ny = fmod(r->v[i] * 7919 + p->t * 53, r->h);
                r->u[i] = 0;
                x = nx;
                y = ny;
            }
            /* a short streak behind it, fading in with its life */
            double tail = 0.10 + 0.08 * band;
            cairo_move_to(cr, nx - vx * tail, ny - vy * tail);
            cairo_line_to(cr, nx, ny + 0.01);
            r->x[i] = (float)nx;
            r->y[i] = (float)ny;
        }
        cairo_set_line_width(cr, 1.4 + band * 0.5);
        src(cr, band == 0 ? p->a1 : band == 1 ? p->a2 : mix(p->a1, p->a3, 0.6), (0.38 + 0.2 * band) * b);
        cairo_stroke(cr);
    }
}

static void draw_neural(cairo_t *cr, struct region *r, const struct params *p)
{
    int n = r->n;
    double b = p->bright * p->focus, D = 150 + 40 * p->density;
    double X[90], Y[90];
    for (int i = 0; i < n; i++) {
        double w1 = 0.05 + r->u[i] * 0.08, w2 = 0.04 + r->v[i] * 0.07;
        X[i] = r->x[i] + 34 * sin(p->t * p->speed * w1 * 6.28 + r->k[i] * 9);
        Y[i] = r->y[i] + 28 * cos(p->t * p->speed * w2 * 6.28 + r->k[i] * 5);
    }
    /* edges in two strengths */
    cairo_set_line_width(cr, 1.0);
    for (int pass = 0; pass < 2; pass++) {
        for (int i = 0; i < n; i++)
            for (int j = i + 1; j < n; j++) {
                double d = hypot(X[i] - X[j], Y[i] - Y[j]);
                if (d > D || (d < D * 0.55) != (pass == 1)) continue;
                cairo_move_to(cr, X[i], Y[i]);
                cairo_line_to(cr, X[j], Y[j]);
            }
        src(cr, mix(p->a1, p->a2, 0.5), (pass ? 0.22 : 0.09) * b);
        cairo_stroke(cr);
    }
    /* signals: a few travel along edges, each picking a new edge now and then */
    int signals = p->quality == 0 ? 3 : p->quality == 1 ? 6 : 10;
    for (int s = 0; s < signals; s++) {
        double tt = p->t * p->speed * (0.5 + 0.4 * p->level) + s * 0.37;
        unsigned hsh = (unsigned)(floor(tt)) * 2654435761u + (unsigned)s * 40503u;
        int i = (int)(hsh % (unsigned)n), best = -1;
        double bd = 1e9;
        for (int j = 0; j < n; j++) {
            double d = hypot(X[i] - X[j], Y[i] - Y[j]);
            if (j != i && d < bd && d < D && ((hsh >> 8) + (unsigned)j) % 3) { bd = d; best = j; }
        }
        if (best < 0) continue;
        double f = frac(tt);
        glow_dot(cr, X[i] + (X[best] - X[i]) * f, Y[i] + (Y[best] - Y[i]) * f, 8, s % 2 ? p->a2 : p->a1, 0.8 * b);
    }
    /* nodes */
    for (int i = 0; i < n; i++) {
        double pulse = 0.5 + 0.5 * sin(p->t * 2.1 * p->speed + r->k[i] * 6.28);
        double lit = near(p, X[i], Y[i], 150);
        cairo_arc(cr, X[i], Y[i], 2.2 + 1.6 * pulse + 2 * lit, 0, 2 * M_PI);
        src(cr, mix(p->a1, p->a3, 0.3 + 0.5 * lit), (0.35 + 0.35 * pulse + 0.3 * lit) * b);
        cairo_fill(cr);
    }
}

static void draw_holo(cairo_t *cr, struct region *r, const struct params *p)
{
    double w = r->w, h = r->h, b = p->bright * p->focus, hor = h * 0.42, vx = w / 2;
    cairo_set_line_width(cr, 1.0);
    /* what stays put, drawn once: the floor's lines to the vanishing point,
     * the horizon's glow, the dots of the sky */
    cairo_t *c2 = cache_begin(r);
    if (c2) {
        cairo_set_line_width(c2, 1.0);
        int cols = 20;
        for (int i = -cols; i <= cols; i++) {
            double xb = vx + i * w / cols * 1.6;
            cairo_move_to(c2, vx + (xb - vx) * 0.02, hor);
            cairo_line_to(c2, xb, h);
        }
        src(c2, p->a1, 0.12);
        cairo_stroke(c2);
        cairo_pattern_t *hg = cairo_pattern_create_linear(0, hor - 30, 0, hor + 40);
        cairo_pattern_add_color_stop_rgba(hg, 0, p->a2.r, p->a2.g, p->a2.b, 0);
        cairo_pattern_add_color_stop_rgba(hg, 0.45, p->a2.r, p->a2.g, p->a2.b, 0.16);
        cairo_pattern_add_color_stop_rgba(hg, 1, p->a1.r, p->a1.g, p->a1.b, 0);
        cairo_set_source(c2, hg);
        cairo_rectangle(c2, 0, hor - 30, w, 70);
        cairo_fill(c2);
        cairo_pattern_destroy(hg);
        for (double y = 18; y < hor - 20; y += 36)
            for (double x = 18; x < w; x += 36) cairo_rectangle(c2, x, y, 1.4, 1.4);
        src(c2, p->a3, 0.10);
        cairo_fill(c2);
        cairo_destroy(c2);
    }
    cache_paint(cr, r, b);
    /* rows coming towards you */
    int rows = p->quality == 0 ? 8 : 14;
    double scroll = frac(p->t * p->speed * 0.25);
    for (int k = 0; k < rows; k++) {
        double z = (k + 1 - scroll) / rows;            /* 0 near the horizon .. 1 near */
        double y = hor + (h - hor) * z * z;
        cairo_move_to(cr, 0, y);
        cairo_line_to(cr, w, y);
        src(cr, mix(p->a2, p->a1, z), (0.04 + 0.22 * z) * b);
        cairo_stroke(cr);
    }
    /* a scan line sweeping down every 7 s */
    double sy = frac(p->t * p->speed / 7) * h;
    cairo_pattern_t *g = cairo_pattern_create_linear(0, sy - 26, 0, sy + 2);
    cairo_pattern_add_color_stop_rgba(g, 0, p->a1.r, p->a1.g, p->a1.b, 0);
    cairo_pattern_add_color_stop_rgba(g, 1, p->a1.r, p->a1.g, p->a1.b, 0.18 * b);
    cairo_set_source(cr, g);
    cairo_rectangle(cr, 0, sy - 26, w, 28);
    cairo_fill(cr);
    cairo_pattern_destroy(g);
    if (p->px > -1e4 && p->reaction > 0) glow_dot(cr, p->px, p->py, 120, p->a1, 0.10 * p->reaction * b);
}

/* 3x5 pixel glyphs: 0-9, A-F. */
static const unsigned short GLYPH[16] = {
    0x7b6f, 0x2c97, 0x73e7, 0x73cf, 0x5bc9, 0x79cf, 0x79ef, 0x7249, 0x7bef, 0x7bcf, 0x2bed, 0x6bae, 0x7927, 0x6b6e, 0x79a7, 0x79a4};

static void draw_rain(cairo_t *cr, struct region *r, const struct params *p)
{
    int n = count(r->n, 1, p->quality, r->n);
    double b = p->bright * p->focus, cell = 2.2, gh = 5 * cell + 4;
    /* four brightness bands, one path each: the head brightest */
    for (int band = 0; band < 4; band++) {
        for (int i = 0; i < n; i++) {
            if (r->k[i] < 0.35) continue;  /* not every column runs */
            double x = (i + 0.5) * r->w / n;
            int trail = 6 + (int)(r->u[i] * 10);
            double speed = (60 + r->v[i] * 90) * p->speed * (1 + 0.6 * p->level);
            double head = frac(p->t * speed / (r->h + trail * gh) + r->k[i]) * (r->h + trail * gh);
            for (int g = 0; g < trail; g++) {
                int bnd = g == 0 ? 0 : g < 3 ? 1 : g < trail * 0.6 ? 2 : 3;
                if (bnd != band) continue;
                double y = head - g * gh;
                if (y < -gh || y > r->h) continue;
                unsigned code = GLYPH[(unsigned)(i * 7 + g * 13 + (int)(p->t * (g == 0 ? 9 : 2))) & 15];
                for (int bit = 0; bit < 15; bit++)
                    if (code & (0x4000 >> bit)) cairo_rectangle(cr, x + (bit % 3) * cell, y + (bit / 3) * cell, cell - 0.4, cell - 0.4);
            }
        }
        static const double alpha[4] = {0.75, 0.42, 0.22, 0.09};
        src(cr, band == 0 ? p->a3 : band == 1 ? p->a1 : mix(p->a1, p->a2, 0.5), alpha[band] * b);
        cairo_fill(cr);
    }
}

static void draw_nebula(cairo_t *cr, struct region *r, const struct params *p)
{
    double w = r->w, h = r->h, b = p->bright * p->focus, t = p->t * p->speed;
    /* Soft clouds: painted once into a cache a little larger than the
     * region (gradients over the whole area are the expensive part), then
     * each frame only copied, drifting slowly by whole pixels. */
    const int M = 48;
    if (!r->cache) {
        r->cache = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, r->w + 2 * M, r->h + 2 * M);
        cairo_t *c2 = cairo_create(r->cache);
        unsigned sd = r->seed + 11;
        for (int i = 0; i < 6; i++) {
            double cx = (r->w + 2 * M) * (0.1 + 0.8 * rnd(&sd)), cy = (r->h + 2 * M) * (0.15 + 0.7 * rnd(&sd));
            double rad = fmax(w, h) * (0.25 + 0.2 * rnd(&sd));
            rgb c = i % 2 ? p->a2 : p->a1;
            cairo_pattern_t *g = cairo_pattern_create_radial(cx, cy, 0, cx, cy, rad);
            cairo_pattern_add_color_stop_rgba(g, 0, c.r, c.g, c.b, 0.15);
            cairo_pattern_add_color_stop_rgba(g, 0.5, c.r, c.g, c.b, 0.055);
            cairo_pattern_add_color_stop_rgba(g, 1, c.r, c.g, c.b, 0);
            cairo_set_source(c2, g);
            cairo_paint(c2);
            cairo_pattern_destroy(g);
        }
        cairo_destroy(c2);
    }
    double dx = round(M * sin(t * 0.021)) - M, dy = round(M * 0.7 * cos(t * 0.017)) - M;
    cairo_set_source_surface(cr, r->cache, dx, dy);
    cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_FAST);
    cairo_paint_with_alpha(cr, clampd(b * (0.85 + 0.15 * sin(t * 0.2)), 0, 1));
    /* stars, drifting a little, twinkling, in two brightness bands */
    int n = count(r->n, 1, p->quality, r->n);
    for (int band = 0; band < 2; band++) {
        for (int i = band; i < n; i += 2) {
            double x = frac((r->x[i] + t * (2 + 6 * r->u[i])) / w) * w, y = r->y[i];
            double tw = 0.5 + 0.5 * sin(t * (1 + 2 * r->v[i]) + r->k[i] * 6.28);
            double s = 0.6 + r->u[i] * 1.4 + 0.6 * tw;
            cairo_rectangle(cr, x, y, s, s);
        }
        src(cr, p->a3, (band ? 0.55 : 0.28) * b);
        cairo_fill(cr);
    }
}

static void draw_pulse(cairo_t *cr, struct region *r, const struct params *p)
{
    double w = r->w, h = r->h, b = p->bright * p->focus, t = p->t * p->speed;
    int waves = p->quality == 0 ? 3 : 5, steps = p->quality == 0 ? 24 : 48;
    double lv = p->level;
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
    for (int k = 0; k < waves; k++) {
        double y0 = h * (0.25 + 0.5 * k / fmax(waves - 1, 1));
        double amp = h * (0.05 + 0.08 * lv) * (1 + 0.3 * sin(t * 0.3 + k));
        double f = (1.5 + k * 0.4) * M_PI / w, sp = (0.6 + 0.25 * k) * (1 + 1.5 * lv);
        for (int i = 0; i <= steps; i++) {
            double x = w * i / steps;
            double y = y0 + amp * sin(x * f + t * sp + k) * sin(M_PI * i / steps);
            if (i == 0) cairo_move_to(cr, x, y);
            else cairo_line_to(cr, x, y);
        }
        rgb c = mix(p->a1, p->a2, k / (double)waves);
        cairo_set_line_width(cr, 1.5);
        src(cr, c, (0.16 + 0.14 * lv) * b);
        cairo_stroke(cr);
        /* the light travelling along it */
        double hx = frac(t * (0.08 + 0.05 * k) * (1 + lv) + k * 0.23) * w;
        for (int i = 0; i <= 10; i++) {
            double x = hx - 90 + 9 * i;
            double y = y0 + amp * sin(x * f + t * sp + k) * sin(M_PI * clampd(x / w, 0, 1));
            if (i == 0) cairo_move_to(cr, x, y);
            else cairo_line_to(cr, x, y);
        }
        cairo_set_line_width(cr, 2.4);
        src(cr, mix(c, p->a3, 0.4), (0.35 + 0.4 * lv) * b);
        cairo_stroke(cr);
    }
}

static void draw_orbital(cairo_t *cr, struct region *r, const struct params *p)
{
    double b = p->bright * p->focus, t = p->t * p->speed, m = fmin(r->w, r->h);
    int per = p->quality == 0 ? 3 : p->quality == 1 ? 4 : 5;
    /* the orbits, drawn once */
    cairo_t *c2 = cache_begin(r);
    if (c2) {
        for (int a = 0; a < r->n; a++) {
            double rx0 = m * (0.08 + 0.1 * r->u[a]);
            for (int e = 0; e < 5; e++) {
                double rx = rx0 * (1 + 0.45 * e), ry = rx * (0.35 + 0.25 * r->v[a]);
                cairo_save(c2);
                cairo_translate(c2, r->x[a], r->y[a]);
                cairo_rotate(c2, r->k[a] * M_PI);
                cairo_scale(c2, 1, ry / rx);
                cairo_new_sub_path(c2);
                cairo_arc(c2, 0, 0, rx, 0, 2 * M_PI);
                cairo_restore(c2);
            }
        }
        cairo_set_line_width(c2, 0.8);
        src(c2, p->a1, 0.08);
        cairo_stroke(c2);
        cairo_destroy(c2);
    }
    cache_paint(cr, r, b);
    for (int a = 0; a < r->n; a++) {
        double cx = r->x[a], cy = r->y[a], rx0 = m * (0.08 + 0.1 * r->u[a]), tilt = r->k[a] * M_PI;
        double lit = near(p, cx, cy, m * 0.4);
        for (int e = 0; e < per; e++) {
            double rx = rx0 * (1 + 0.45 * e), ry = rx * (0.35 + 0.25 * r->v[a]);
            double ang = t * (0.5 - 0.08 * e) * (e % 2 ? -1 : 1) + e * 1.9 + a;
            double ex = cos(ang) * rx, ey = sin(ang) * ry;
            double x = cx + ex * cos(tilt) - ey * sin(tilt), y = cy + ex * sin(tilt) + ey * cos(tilt);
            hex(cr, x, y, 3 + e * 0.4, ang);
            src(cr, e % 2 ? p->a2 : p->a1, (0.5 + 0.3 * lit) * b);
            cairo_fill(cr);
        }
        glow_dot(cr, cx, cy, 10 + 6 * lit, p->a3, (0.25 + 0.3 * lit) * b);
    }
}

static void draw_geometry(cairo_t *cr, struct region *r, const struct params *p)
{
    double b = p->bright * p->focus, t = p->t * p->speed * (1 + 1.2 * p->level), m = fmin(r->w, r->h);
    cairo_set_line_join(cr, CAIRO_LINE_JOIN_ROUND);
    for (int i = 0; i < r->n; i++) {
        double cx = r->x[i] + m * 0.05 * sin(t * 0.07 + i), cy = r->y[i] + m * 0.05 * cos(t * 0.06 + i * 2);
        double R = m * (0.05 + 0.08 * r->u[i]), rot = t * (0.05 + 0.1 * r->v[i]) * (i % 2 ? -1 : 1);
        /* a hexagon breathing into a triangle and back */
        double morph = 0.5 + 0.5 * sin(t * 0.25 + r->k[i] * 6.28);
        for (int ring = 0; ring < (p->quality == 0 ? 1 : 2); ring++) {
            double rr = R * (1 + 0.45 * ring);
            for (int v = 0; v <= 6; v++) {
                double a = rot - M_PI / 2 + v * M_PI / 3, rad = rr * (v % 2 ? 1 - 0.5 * morph : 1);
                if (v == 0) cairo_move_to(cr, cx + rad * cos(a), cy + rad * sin(a));
                else cairo_line_to(cr, cx + rad * cos(a), cy + rad * sin(a));
            }
            cairo_close_path(cr);
            cairo_set_line_width(cr, ring ? 0.8 : 1.3);
            src(cr, ring ? p->a2 : p->a1, (ring ? 0.12 : 0.28) * b);
            cairo_stroke(cr);
        }
        cairo_arc(cr, cx, cy, 2, 0, 2 * M_PI);
        src(cr, p->a3, 0.4 * b);
        cairo_fill(cr);
    }
}

static void draw_signature(cairo_t *cr, struct region *r, const struct params *p)
{
    double b = p->bright * p->focus, t = p->t * p->speed;
    /* drawn once: a faint honeycomb around each node, and the flowing
     * lines, blue through to purple (a solid colour per third: a gradient
     * source is computed over the whole region in software) */
    cairo_t *c2 = cache_begin(r);
    if (c2) {
        cairo_set_line_width(c2, 0.8);
        for (int j = 0; j < r->m; j++) hex(c2, r->x[j], r->y[j], 18 + 6 * r->v[j], 0);
        src(c2, p->a1, 0.09);
        cairo_stroke(c2);
        cairo_set_line_width(c2, 1.4);
        cairo_set_line_join(c2, CAIRO_LINE_JOIN_ROUND);
        for (int band = 0; band < 3; band++) {
            for (int i = band; i < r->n; i += 3) {
                cairo_move_to(c2, r->px[r->start[i]], r->py[r->start[i]]);
                for (int k = r->start[i] + 1; k < r->start[i + 1]; k++) cairo_line_to(c2, r->px[k], r->py[k]);
            }
            src(c2, mix(p->a1, p->a2, band / 2.0), 0.2);
            cairo_stroke(c2);
        }
        cairo_destroy(c2);
    }
    cache_paint(cr, r, b);
    /* light running along each line */
    for (int i = 0; i < r->n; i++) {
        double s = frac(t * r->u[i] * (1 + p->level) + r->k[i]), x, y;
        along(r, i, s, &x, &y);
        glow_dot(cr, x, y, p->quality ? 10 : 5, mix(p->a1, p->a2, s), 0.7 * b);
    }
    /* nodes, pulsing, lit by the pointer */
    for (int j = 0; j < r->m; j++) {
        double pulse = 0.5 + 0.5 * sin(t * 1.4 + r->v[j] * 6.28), lit = near(p, r->x[j], r->y[j], 150);
        hex(cr, r->x[j], r->y[j], 3.2 + 1.5 * pulse + 2 * lit, 0);
        src(cr, mix(p->a1, p->a3, 0.4 + 0.4 * lit), (0.45 + 0.3 * pulse + 0.3 * lit) * b);
        cairo_fill(cr);
    }
}

void style_draw(cairo_t *cr, struct region *r, const struct params *p)
{
    cairo_save(cr);
    /* coarser curves and cheaper edges below High: invisible at this faintness */
    if (p->quality < 2) {
        cairo_set_antialias(cr, CAIRO_ANTIALIAS_FAST);
        cairo_set_tolerance(cr, 0.5);
    }
    switch (r->style) {
    case ST_CIRCUIT: draw_circuit(cr, r, p); break;
    case ST_QUANTUM: draw_quantum(cr, r, p); break;
    case ST_NEURAL: draw_neural(cr, r, p); break;
    case ST_HOLOGRID: draw_holo(cr, r, p); break;
    case ST_RAIN: draw_rain(cr, r, p); break;
    case ST_NEBULA: draw_nebula(cr, r, p); break;
    case ST_PULSE: draw_pulse(cr, r, p); break;
    case ST_ORBITAL: draw_orbital(cr, r, p); break;
    case ST_GEOMETRY: draw_geometry(cr, r, p); break;
    case ST_SIGNATURE: draw_signature(cr, r, p); break;
    }
    cairo_restore(cr);
}
