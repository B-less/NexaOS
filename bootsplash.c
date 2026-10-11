/* NexaOS boot animation, drawn straight to /dev/fb0.
 *
 * One process (--daemon) owns the screen for the whole boot. The initramfs
 * starts it in the first second; it keeps running across switch_root (from
 * memory, with the initramfs's /dev and /sys still open to it) and is driven
 * by short-lived clients over an abstract unix socket -- abstract, so the
 * same name works from the initramfs and from the real root, which share no
 * filesystem.
 *
 *   bootsplash --daemon              run the animation until told to quit
 *   bootsplash <percent> [status]    update progress and the status line
 *   bootsplash --quit                fade out, give the console back, and
 *                                    return once the screen is free
 *   bootsplash --ping                exit 0 if a daemon is running
 *   bootsplash --text                put the console back in text mode
 *   bootsplash --preview W H T PCT FILE.ppm [status]
 *                                    render the frame at T seconds (testing)
 *
 * The sequence: the hexagon draws itself, the core ring and dot pop in, a
 * beam from the core writes N-E-X-A, then the hexagon shrinks into the
 * loader under the wordmark -- a comet chases its edges while the ring
 * fills with boot progress.
 */
#include <errno.h>
#include <fcntl.h>
#include <linux/fb.h>
#include <linux/kd.h>
#include <math.h>
#include <poll.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

#include "font8x8.h"

#define SOCK_NAME "nexa-splash"
#define PI 3.14159265358979f

/* Timeline, in seconds from daemon start. */
#define T_FADE_IN   0.35f
#define T_HEX_0     0.30f
#define T_HEX_1     1.60f
#define T_CORE_0    1.60f
#define T_CORE_1    2.25f
#define T_WRITE_0   2.25f
#define T_WRITE_1   4.05f
#define T_BEAM_OFF  4.35f
#define T_MOVE_0    4.35f
#define T_MOVE_1    5.25f
#define T_FADE_OUT  0.60f
#define T_MAX_LIFE  180.0f  /* never hold the screen longer than this */

typedef struct { float r, g, b; } RGB;

static const RGB BLUE      = {122, 162, 247};
static const RGB BLUE_TOP  = {143, 180, 255};
static const RGB BLUE_LOW  = {106, 143, 224};
static const RGB HOT       = {236, 243, 255};
static const RGB STATUS_C  = {150, 160, 182};

typedef struct {
    int w, h;
    uint8_t *px;            /* w*h*3, RGB */
    uint8_t *bg;            /* precomputed background */
} Canvas;

static inline float clampf(float v, float lo, float hi) { return v < lo ? lo : v > hi ? hi : v; }
static inline float mixf(float a, float b, float t) { return a + (b - a) * t; }
static inline RGB mixc(RGB a, RGB b, float t) { return (RGB){mixf(a.r, b.r, t), mixf(a.g, b.g, t), mixf(a.b, b.b, t)}; }
static inline float ease_out(float t) { t = clampf(t, 0, 1); return 1 - (1 - t) * (1 - t) * (1 - t); }
static inline float ease_in_out(float t) { t = clampf(t, 0, 1); return t < .5f ? 4 * t * t * t : 1 - powf(-2 * t + 2, 3) / 2; }
static inline float ease_back(float t) {
    t = clampf(t, 0, 1);
    const float c1 = 1.70158f, c3 = c1 + 1;
    return 1 + c3 * powf(t - 1, 3) + c1 * powf(t - 1, 2);
}
static inline float phase(float t, float a, float b) { return clampf((t - a) / (b - a), 0, 1); }

/* ---- pixels ---------------------------------------------------------- */

static inline void blend(Canvas *c, int x, int y, RGB col, float a) {
    if (x < 0 || y < 0 || x >= c->w || y >= c->h || a <= 0) return;
    if (a > 1) a = 1;
    uint8_t *p = c->px + ((size_t)y * c->w + x) * 3;
    p[0] = (uint8_t)(p[0] + (col.r - p[0]) * a);
    p[1] = (uint8_t)(p[1] + (col.g - p[1]) * a);
    p[2] = (uint8_t)(p[2] + (col.b - p[2]) * a);
}

static inline void add(Canvas *c, int x, int y, RGB col, float a) {
    if (x < 0 || y < 0 || x >= c->w || y >= c->h || a <= 0.002f) return;
    uint8_t *p = c->px + ((size_t)y * c->w + x) * 3;
    float v[3] = {p[0] + col.r * a, p[1] + col.g * a, p[2] + col.b * a};
    for (int i = 0; i < 3; i++) p[i] = v[i] > 255 ? 255 : (uint8_t)v[i];
}

/* ---- shapes ---------------------------------------------------------- */

typedef struct {
    float x[16], y[16], len[16];  /* len[i] = arc length up to point i */
    int n;
} Poly;

static void poly_add(Poly *p, float x, float y) {
    p->x[p->n] = x;
    p->y[p->n] = y;
    p->len[p->n] = p->n ? p->len[p->n - 1] + hypotf(x - p->x[p->n - 1], y - p->y[p->n - 1]) : 0;
    p->n++;
}

static float poly_total(const Poly *p) { return p->n ? p->len[p->n - 1] : 0; }

static void poly_point(const Poly *p, float s, float *x, float *y) {
    for (int i = 1; i < p->n; i++) {
        if (s <= p->len[i] || i == p->n - 1) {
            float seg = p->len[i] - p->len[i - 1];
            float t = seg > 0 ? clampf((s - p->len[i - 1]) / seg, 0, 1) : 0;
            *x = mixf(p->x[i - 1], p->x[i], t);
            *y = mixf(p->y[i - 1], p->y[i], t);
            return;
        }
    }
    *x = p->x[0];
    *y = p->y[0];
}

/* Style for a stroked polyline. Colour comes from a vertical gradient
 * (top..low over y0..y1) and, behind a moving pen, cools from HOT. */
typedef struct {
    float width, alpha;
    float glow, glow_alpha;
    RGB top, low;
    float y0, y1;
    float pen, hot_len;     /* hot_len > 0: colour by distance behind pen */
} Stroke;

/* Draw the first `upto` length of the polyline, anti-aliased, with round
 * caps and an optional glow; one pass per pixel over the shared bbox so
 * joints don't double up. */
static void draw_poly(Canvas *c, const Poly *p, float upto, const Stroke *st) {
    if (p->n < 2 || upto <= 0) return;
    Poly q = {.n = 0};
    poly_add(&q, p->x[0], p->y[0]);
    for (int i = 1; i < p->n; i++) {
        if (p->len[i] <= upto) { poly_add(&q, p->x[i], p->y[i]); continue; }
        float x, y;
        poly_point(p, upto, &x, &y);
        poly_add(&q, x, y);
        break;
    }
    if (q.n < 2) return;

    float pad = st->width / 2 + st->glow * 2.2f + 2;
    float minx = q.x[0], maxx = q.x[0], miny = q.y[0], maxy = q.y[0];
    for (int i = 1; i < q.n; i++) {
        minx = fminf(minx, q.x[i]); maxx = fmaxf(maxx, q.x[i]);
        miny = fminf(miny, q.y[i]); maxy = fmaxf(maxy, q.y[i]);
    }
    int x0 = (int)(minx - pad), x1 = (int)(maxx + pad), y0 = (int)(miny - pad), y1 = (int)(maxy + pad);
    if (x0 < 0) x0 = 0;
    if (y0 < 0) y0 = 0;
    if (x1 >= c->w) x1 = c->w - 1;
    if (y1 >= c->h) y1 = c->h - 1;
    float hw = st->width / 2;

    for (int y = y0; y <= y1; y++) {
        for (int x = x0; x <= x1; x++) {
            float fx = x + .5f, fy = y + .5f, best = 1e9f, at = 0;
            for (int i = 1; i < q.n; i++) {
                float ax = q.x[i - 1], ay = q.y[i - 1], dx = q.x[i] - ax, dy = q.y[i] - ay;
                float l2 = dx * dx + dy * dy;
                float t = l2 > 0 ? clampf(((fx - ax) * dx + (fy - ay) * dy) / l2, 0, 1) : 0;
                float ex = ax + t * dx - fx, ey = ay + t * dy - fy, d = ex * ex + ey * ey;
                if (d < best) { best = d; at = q.len[i - 1] + t * (q.len[i] - q.len[i - 1]); }
            }
            float d = sqrtf(best);
            if (d > hw + st->glow * 2.2f + 1) continue;
            float gy = st->y1 > st->y0 ? clampf((fy - st->y0) / (st->y1 - st->y0), 0, 1) : 0;
            RGB col = mixc(st->top, st->low, gy);
            if (st->hot_len > 0) col = mixc(HOT, col, clampf((st->pen - at) / st->hot_len, 0, 1));
            float core = clampf(hw + .5f - d, 0, 1) * st->alpha;
            if (core > 0) blend(c, x, y, col, core);
            if (st->glow > 0 && d > hw * .5f) {
                float g = (d - hw * .5f) / st->glow;
                add(c, x, y, col, st->glow_alpha * expf(-g * g));
            }
        }
    }
}

static void draw_glow(Canvas *c, float cx, float cy, float r, RGB col, float a) {
    if (a <= 0 || r <= 0) return;
    int x0 = (int)(cx - r * 2.5f), x1 = (int)(cx + r * 2.5f), y0 = (int)(cy - r * 2.5f), y1 = (int)(cy + r * 2.5f);
    for (int y = y0; y <= y1; y++)
        for (int x = x0; x <= x1; x++) {
            float d = hypotf(x + .5f - cx, y + .5f - cy) / r;
            add(c, x, y, col, a * expf(-d * d));
        }
}

static void draw_disc(Canvas *c, float cx, float cy, float r, RGB col, float a) {
    if (a <= 0 || r <= 0) return;
    for (int y = (int)(cy - r - 2); y <= (int)(cy + r + 2); y++)
        for (int x = (int)(cx - r - 2); x <= (int)(cx + r + 2); x++)
            blend(c, x, y, col, clampf(r + .5f - hypotf(x + .5f - cx, y + .5f - cy), 0, 1) * a);
}

/* Ring of radius r; arc in 0..1 draws clockwise from 12 o'clock. */
static void draw_ring(Canvas *c, float cx, float cy, float r, float width, RGB col, float a, float arc) {
    if (a <= 0 || r <= 0 || arc <= 0) return;
    float hw = width / 2, lim = arc * 2 * PI;
    for (int y = (int)(cy - r - hw - 2); y <= (int)(cy + r + hw + 2); y++)
        for (int x = (int)(cx - r - hw - 2); x <= (int)(cx + r + hw + 2); x++) {
            float dx = x + .5f - cx, dy = y + .5f - cy;
            float cov = clampf(hw + .5f - fabsf(hypotf(dx, dy) - r), 0, 1);
            if (cov <= 0) continue;
            if (arc < 1) {
                float ang = atan2f(dx, -dy);
                if (ang < 0) ang += 2 * PI;
                if (ang > lim) continue;
            }
            blend(c, x, y, col, cov * a);
        }
}

static void draw_text(Canvas *c, int x, int y, const char *s, int scale, RGB col, float a) {
    for (int i = 0; s[i]; i++) {
        unsigned char ch = (unsigned char)s[i];
        if (ch >= 128) continue;
        for (int row = 0; row < 8; row++)
            for (int bit = 0; bit < 8; bit++)
                if (font8x8_basic[ch][row] & (1 << bit))
                    for (int j = 0; j < scale; j++)
                        for (int k = 0; k < scale; k++)
                            blend(c, x + (i * 8 + bit) * scale + k, y + row * scale + j, col, a);
    }
}

/* ---- scene ----------------------------------------------------------- */

/* Background: the wallpaper's dark gradient with a soft blue glow. */
static void make_background(Canvas *c) {
    const RGB a = {26, 27, 30}, b = {33, 35, 40};
    float gx = c->w * .5f, gy = c->h * .42f, gr = c->h * .75f;
    for (int y = 0; y < c->h; y++)
        for (int x = 0; x < c->w; x++) {
            float t = ((float)x / c->w + (float)y / c->h) * .5f;
            RGB col = mixc(a, b, t);
            float d = hypotf(x - gx, y - gy) / gr;
            float glow = .10f * expf(-d * d * 2.2f);
            uint8_t *p = c->bg + ((size_t)y * c->w + x) * 3;
            p[0] = (uint8_t)clampf(col.r + BLUE.r * glow, 0, 255);
            p[1] = (uint8_t)clampf(col.g + BLUE.g * glow, 0, 255);
            p[2] = (uint8_t)clampf(col.b + BLUE.b * glow, 0, 255);
        }
}

/* Thin 1px anti-aliased-ish line for the constellation (cheap DDA). */
static void thin_line(Canvas *c, float x0, float y0, float x1, float y1, RGB col, float a) {
    float len = hypotf(x1 - x0, y1 - y0);
    int n = (int)len + 1;
    for (int i = 0; i <= n; i++) {
        float t = (float)i / n;
        add(c, (int)mixf(x0, x1, t), (int)mixf(y0, y1, t), col, a);
    }
}

#define STARS 44
static void draw_constellation(Canvas *c, float t, float s, float a) {
    if (a <= 0) return;
    float sx[STARS], sy[STARS];
    unsigned seed = 0x4e455841u;  /* "NEXA" */
    for (int i = 0; i < STARS; i++) {
        seed = seed * 1103515245u + 12345u; float px = (seed >> 8) / 16777216.0f;
        seed = seed * 1103515245u + 12345u; float py = (seed >> 8) / 16777216.0f;
        seed = seed * 1103515245u + 12345u; float ang = (seed >> 8) / 16777216.0f * 2 * PI;
        float speed = 6 * s;
        float W = c->w + 200 * s, H = c->h + 200 * s;
        sx[i] = fmodf(px * W + cosf(ang) * speed * t + W * 4, W) - 100 * s;
        sy[i] = fmodf(py * H + sinf(ang) * speed * t + H * 4, H) - 100 * s;
    }
    float link = 210 * s;
    for (int i = 0; i < STARS; i++)
        for (int j = i + 1; j < STARS; j++) {
            float d = hypotf(sx[i] - sx[j], sy[i] - sy[j]);
            if (d < link) thin_line(c, sx[i], sy[i], sx[j], sy[j], BLUE, a * .16f * (1 - d / link));
        }
    for (int i = 0; i < STARS; i++)
        draw_disc(c, sx[i], sy[i], 2.2f * s, BLUE, a * .45f);
}

/* The NEXA wordmark as single-line strokes, in letter-height units. */
typedef struct { float pts[4][2]; int n; float dx; } LetterStroke;
static const LetterStroke WORD[] = {
    {{{0, 1}, {0, 0}, {.78f, 1}, {.78f, 0}}, 4, 0},          /* N */
    {{{.62f, 0}, {0, 0}, {0, 1}, {.62f, 1}}, 4, 1.04f},      /* E */
    {{{0, .5f}, {.5f, .5f}}, 2, 1.04f},
    {{{0, 0}, {.80f, 1}}, 2, 1.92f},                         /* X */
    {{{.80f, 0}, {0, 1}}, 2, 1.92f},
    {{{0, 1}, {.42f, 0}, {.84f, 1}}, 3, 2.98f},               /* A */
    {{{.18f, .62f}, {.66f, .62f}}, 2, 2.98f},
};
#define WORD_STROKES ((int)(sizeof(WORD) / sizeof(WORD[0])))
#define WORD_W 3.82f

typedef struct {
    float progress, progress_shown;
    char status[96], prev_status[96];
    float status_at;
    float quit_at;          /* <0: not quitting; else time the fade-out starts */
} State;

static void hexagon(Poly *p, float cx, float cy, float r) {
    p->n = 0;
    for (int k = 0; k <= 6; k++) {
        float a = (-90 + 60 * k) * PI / 180;
        poly_add(p, cx + r * cosf(a), cy + r * sinf(a));
    }
}

static void render(Canvas *c, float t, State *st) {
    memcpy(c->px, c->bg, (size_t)c->w * c->h * 3);
    float s = fminf(c->w / 1920.0f, c->h / 1080.0f);
    float W = c->w, H = c->h;

    /* Layout: intro lockup (logo left of the word), then the word centred
     * with the logo shrunk into the loader below it. */
    float R0 = 70 * s, LH = 92 * s, hexw = R0 * 1.7320508f, gap = 46 * s;
    float group = hexw + gap + WORD_W * LH;
    float logo_x0 = W / 2 - group / 2 + hexw / 2, logo_y0 = H / 2;
    float word_x0 = W / 2 - group / 2 + hexw + gap, word_y0 = H / 2;
    float word_x1 = W / 2 - WORD_W * LH / 2, word_y1 = H / 2 - 52 * s;
    float logo_x1 = W / 2, logo_y1 = H / 2 + 118 * s, scale1 = .62f;

    float mv = ease_in_out(phase(t, T_MOVE_0, T_MOVE_1));
    /* The logo drops first, then glides across, so its path stays below
     * the wordmark instead of sweeping through the letters. */
    float drop = ease_out(phase(t, T_MOVE_0, T_MOVE_0 + .55f));
    float glide = ease_in_out(phase(t, T_MOVE_0 + .2f, T_MOVE_1));
    float lx = mixf(logo_x0, logo_x1, glide), ly = mixf(logo_y0, logo_y1, drop);
    float R = R0 * mixf(1, scale1, drop);
    float wx = mixf(word_x0, word_x1, mv), wy = mixf(word_y0, word_y1, mv);
    float ls = mixf(1, scale1, drop);  /* logo scale for strokes */

    draw_constellation(c, t, s, ease_out(phase(t, 0, 1.2f)));

    /* 1. The hexagon draws itself; corners flash as the pen reaches them. */
    Poly hex;
    hexagon(&hex, lx, ly, R);
    float per = poly_total(&hex), side = per / 6;
    float pen = ease_in_out(phase(t, T_HEX_0, T_HEX_1)) * per;
    int looping = t >= T_MOVE_0;
    Stroke hs = {.width = 6 * s * ls, .alpha = .95f, .glow = 10 * s * ls, .glow_alpha = .28f,
                 .top = BLUE, .low = BLUE, .y0 = 0, .y1 = 0};
    if (looping) hs.alpha = mixf(.95f, .38f, mv), hs.glow_alpha = mixf(.28f, .08f, mv);
    draw_poly(c, &hex, pen, &hs);
    if (t > T_HEX_0 && t < T_HEX_1 + .2f) {
        float px, py;
        poly_point(&hex, pen, &px, &py);
        float fade = 1 - phase(t, T_HEX_1, T_HEX_1 + .2f);
        draw_glow(c, px, py, 14 * s, HOT, .9f * fade);
        for (int k = 0; k <= 6; k++) {
            float since = (pen - k * side) / (side * .35f);
            if (since > 0 && since < 1) draw_glow(c, hex.x[k], hex.y[k], 18 * s, BLUE, (1 - since) * .8f);
        }
    }

    /* 2. Core ring and dot pop in, with a shockwave. */
    float ring_r = R * .42f, dot_r = R * .24f;
    float rp = ease_back(phase(t, T_CORE_0, T_CORE_0 + .5f));
    float dp = ease_back(phase(t, T_CORE_0 + .15f, T_CORE_1));
    float breathe = t > T_MOVE_1 ? 1 + .08f * sinf((t - T_MOVE_1) * 2 * PI / 1.6f) : 1;
    if (!looping || mv < 1) draw_ring(c, lx, ly, ring_r * rp, 2.8f * s * ls, BLUE, .5f * (1 - mv), 1);
    if (looping) {
        draw_ring(c, lx, ly, ring_r, 3.2f * s * ls * 1.6f, BLUE, .18f * mv, 1);
        draw_ring(c, lx, ly, ring_r, 3.2f * s * ls * 1.6f, BLUE_TOP, .95f * mv, st->progress_shown);
    }
    draw_disc(c, lx, ly, dot_r * dp * breathe, BLUE, 1);
    draw_glow(c, lx, ly, dot_r * 1.6f * dp, BLUE, .35f);
    float sw = phase(t, T_CORE_0 + .15f, T_CORE_0 + .85f);
    if (sw > 0 && sw < 1) draw_ring(c, lx, ly, mixf(dot_r, R * 2.2f, ease_out(sw)), 3 * s, BLUE, .4f * (1 - sw), 1);

    /* 3. A beam from the core writes N-E-X-A. Strokes are white-hot behind
     * the pen and cool to the wordmark's blue gradient. */
    float total = 0;
    Poly strokes[WORD_STROKES];
    for (int i = 0; i < WORD_STROKES; i++) {
        strokes[i].n = 0;
        for (int k = 0; k < WORD[i].n; k++)
            poly_add(&strokes[i], wx + (WORD[i].dx + WORD[i].pts[k][0]) * LH, wy - LH / 2 + WORD[i].pts[k][1] * LH);
        total += poly_total(&strokes[i]);
    }
    float wp = phase(t, T_WRITE_0, T_WRITE_1);
    float wpen = (wp < .08f ? wp * wp / .16f : wp > .92f ? 1 - (1 - wp) * (1 - wp) / .16f - .0f : wp - .04f) / .92f * total;
    if (wp >= 1) wpen = total;
    float done = 0, tipx = -1, tipy = -1;
    for (int i = 0; i < WORD_STROKES; i++) {
        float l = poly_total(&strokes[i]);
        float here = clampf(wpen - done, 0, l);
        Stroke ls_ = {.width = .14f * LH, .alpha = 1, .glow = 9 * s, .glow_alpha = .22f,
                      .top = BLUE_TOP, .low = BLUE_LOW, .y0 = wy - LH / 2, .y1 = wy + LH / 2,
                      .pen = here + (wpen - done - here), .hot_len = .9f * LH};
        if (here > 0) draw_poly(c, &strokes[i], here, &ls_);
        if (wpen > done && wpen <= done + l) poly_point(&strokes[i], here, &tipx, &tipy);
        done += l;
    }
    float beam = wp > 0 && wp < 1 ? 1 : 1 - phase(t, T_WRITE_1, T_BEAM_OFF);
    if (wp > 0 && beam > 0 && tipx >= 0) {
        float flick = .85f + .15f * sinf(t * 47);
        Poly b = {.n = 0};
        poly_add(&b, lx, ly);
        poly_add(&b, tipx, tipy);
        Stroke bs = {.width = 2.2f * s, .alpha = .9f * beam * flick, .glow = 7 * s, .glow_alpha = .45f * beam,
                     .top = HOT, .low = BLUE};
        bs.y0 = fminf(ly, tipy); bs.y1 = fmaxf(ly, tipy) + 1;
        draw_poly(c, &b, poly_total(&b), &bs);
        draw_glow(c, tipx, tipy, 13 * s, HOT, .95f * beam);
        draw_glow(c, lx, ly, 20 * s, BLUE, .5f * beam);
    }

    /* 4. Loader: a comet chases the hexagon's edges. */
    if (looping) {
        float lt = t - T_MOVE_0;
        float head = fmodf(lt / 1.4f, 1) * per, tail = per * .34f;
        int steps = (int)(tail / (1.2f * s)) + 1;
        for (int i = steps; i >= 0; i--) {
            float sp = head - tail * i / steps;
            while (sp < 0) sp += per;
            float px, py;
            poly_point(&hex, sp, &px, &py);
            float k = 1 - (float)i / steps;
            draw_disc(c, px, py, 3.4f * s * ls * (.4f + .6f * k), mixc(BLUE, HOT, k * k), k * k * mv);
        }
        float hx, hy;
        poly_point(&hex, head, &hx, &hy);
        draw_glow(c, hx, hy, 10 * s, HOT, .8f * mv);
    }

    /* Status line under the loader, cross-fading on change. */
    if (looping && st) {
        int sc = s >= 1.5f ? 3 : s >= .9f ? 2 : 1;
        int ty = (int)(logo_y1 + R0 * scale1 + 40 * s);
        float f = clampf((t - st->status_at) / .35f, 0, 1) * mv;
        if (st->prev_status[0] && f < 1)
            draw_text(c, (int)(W / 2 - strlen(st->prev_status) * 4 * sc), ty, st->prev_status, sc, STATUS_C, (1 - f) * .9f);
        if (st->status[0])
            draw_text(c, (int)(W / 2 - strlen(st->status) * 4 * sc), ty, st->status, sc, STATUS_C, f * .9f);
    }

    /* Fade in from black, fade out at the end. */
    float vis = ease_out(phase(t, 0, T_FADE_IN));
    if (st && st->quit_at >= 0) vis *= 1 - ease_in_out(phase(t, st->quit_at, st->quit_at + T_FADE_OUT));
    if (vis < 1) {
        size_t n = (size_t)c->w * c->h * 3;
        int m = (int)(vis * 256);
        for (size_t i = 0; i < n; i++) c->px[i] = (uint8_t)((c->px[i] * m) >> 8);
    }
}

/* ---- framebuffer ----------------------------------------------------- */

typedef struct {
    int fd;
    uint8_t *mem;
    size_t size;
    int stride, bpp;
    struct fb_var_screeninfo var;
    char name[32];
} FB;

static void fb_name(char *out, size_t n) {
    out[0] = 0;
    FILE *f = fopen("/sys/class/graphics/fb0/name", "r");
    if (!f) return;
    if (fgets(out, (int)n, f)) out[strcspn(out, "\n")] = 0;
    fclose(f);
}

static int fb_open(FB *fb) {
    memset(fb, 0, sizeof(*fb));
    fb->fd = open("/dev/fb0", O_RDWR | O_CLOEXEC);
    if (fb->fd < 0) return -1;
    struct fb_fix_screeninfo fix;
    if (ioctl(fb->fd, FBIOGET_FSCREENINFO, &fix) || ioctl(fb->fd, FBIOGET_VSCREENINFO, &fb->var)) {
        close(fb->fd);
        return -1;
    }
    fb->stride = fix.line_length;
    fb->bpp = fb->var.bits_per_pixel;
    fb->size = (size_t)fix.line_length * fb->var.yres;
    fb->mem = mmap(NULL, fb->size, PROT_READ | PROT_WRITE, MAP_SHARED, fb->fd, 0);
    if (fb->mem == MAP_FAILED || (fb->bpp != 32 && fb->bpp != 16)) {
        if (fb->mem != MAP_FAILED) munmap(fb->mem, fb->size);
        close(fb->fd);
        return -1;
    }
    fb_name(fb->name, sizeof(fb->name));
    return 0;
}

static void fb_close(FB *fb) {
    if (fb->fd <= 0) return;
    munmap(fb->mem, fb->size);
    close(fb->fd);
    fb->fd = 0;
}

static void fb_present(FB *fb, const Canvas *c) {
    const struct fb_var_screeninfo *v = &fb->var;
    int w = c->w < (int)v->xres ? c->w : (int)v->xres, h = c->h < (int)v->yres ? c->h : (int)v->yres;
    for (int y = 0; y < h; y++) {
        const uint8_t *s = c->px + (size_t)y * c->w * 3;
        uint8_t *d = fb->mem + (size_t)y * fb->stride;
        if (fb->bpp == 32) {
            uint32_t *o = (uint32_t *)d;
            for (int x = 0; x < w; x++, s += 3)
                o[x] = (uint32_t)(s[0] >> (8 - v->red.length)) << v->red.offset |
                       (uint32_t)(s[1] >> (8 - v->green.length)) << v->green.offset |
                       (uint32_t)(s[2] >> (8 - v->blue.length)) << v->blue.offset;
        } else {
            uint16_t *o = (uint16_t *)d;
            for (int x = 0; x < w; x++, s += 3)
                o[x] = (uint16_t)((s[0] >> (8 - v->red.length)) << v->red.offset |
                                  (s[1] >> (8 - v->green.length)) << v->green.offset |
                                  (s[2] >> (8 - v->blue.length)) << v->blue.offset);
        }
    }
}

static int canvas_init(Canvas *c, int w, int h) {
    c->w = w;
    c->h = h;
    c->px = malloc((size_t)w * h * 3);
    c->bg = malloc((size_t)w * h * 3);
    if (!c->px || !c->bg) return -1;
    make_background(c);
    return 0;
}

/* ---- console --------------------------------------------------------- */

static void console_mode(int mode) {
    int fd = open("/dev/tty0", O_RDWR | O_CLOEXEC);
    if (fd < 0) return;
    ioctl(fd, KDSETMODE, mode);
    close(fd);
}

/* ---- socket ---------------------------------------------------------- */

static socklen_t sock_addr(struct sockaddr_un *a) {
    memset(a, 0, sizeof(*a));
    a->sun_family = AF_UNIX;
    memcpy(a->sun_path + 1, SOCK_NAME, strlen(SOCK_NAME));
    return (socklen_t)(offsetof(struct sockaddr_un, sun_path) + 1 + strlen(SOCK_NAME));
}

static double now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

/* Send a message; if want_reply, wait up to timeout for "OK". Returns 0 on
 * success, -1 when no daemon is listening (not an error for callers). */
static int client_send(const char *msg, int want_reply, int timeout_ms) {
    int fd = socket(AF_UNIX, SOCK_DGRAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    struct sockaddr_un self = {.sun_family = AF_UNIX};
    bind(fd, (struct sockaddr *)&self, sizeof(sa_family_t));  /* autobind, for replies */
    struct sockaddr_un a;
    socklen_t al = sock_addr(&a);
    int rc = sendto(fd, msg, strlen(msg), 0, (struct sockaddr *)&a, al) < 0 ? -1 : 0;
    if (rc == 0 && want_reply) {
        struct pollfd p = {.fd = fd, .events = POLLIN};
        char buf[8];
        rc = poll(&p, 1, timeout_ms) > 0 && recv(fd, buf, sizeof(buf), 0) > 0 ? 0 : -1;
    }
    close(fd);
    return rc;
}

static volatile sig_atomic_t got_signal;
static void on_signal(int sig) { (void)sig; got_signal = 1; }

static void set_status(State *st, const char *text, float t) {
    if (strncmp(st->status, text, sizeof(st->status) - 1) == 0) return;
    memcpy(st->prev_status, st->status, sizeof(st->status));
    snprintf(st->status, sizeof(st->status), "%.95s", text);
    st->status_at = t;
}

static int run_daemon(void) {
    int sock = socket(AF_UNIX, SOCK_DGRAM | SOCK_CLOEXEC, 0);
    struct sockaddr_un a;
    socklen_t al = sock_addr(&a);
    if (sock < 0 || bind(sock, (struct sockaddr *)&a, al) < 0) return 0;  /* one already running */

    FB fb;
    if (fb_open(&fb) != 0) return 0;  /* no framebuffer: boot shows text, that's fine */
    Canvas c;
    if (canvas_init(&c, fb.var.xres, fb.var.yres) != 0) return 1;

    signal(SIGTERM, on_signal);
    signal(SIGINT, on_signal);
    signal(SIGHUP, SIG_IGN);
    console_mode(KD_GRAPHICS);

    State st = {.progress = 0, .progress_shown = 0, .quit_at = -1};
    struct sockaddr_un waiters[8];
    socklen_t waiter_len[8];
    int nwait = 0;
    double t0 = now(), last = 0;
    int frame = 0;

    for (;;) {
        double t = now() - t0;
        if (got_signal && st.quit_at < 0) st.quit_at = (float)t;
        if (t > T_MAX_LIFE && st.quit_at < 0) st.quit_at = (float)t;

        char buf[128];
        struct sockaddr_un from;
        socklen_t fl = sizeof(from);
        ssize_t n;
        while ((n = recvfrom(sock, buf, sizeof(buf) - 1, MSG_DONTWAIT, (struct sockaddr *)&from, &fl)) > 0) {
            buf[n] = 0;
            if (buf[0] == 'P') st.progress = clampf(atoi(buf + 1) / 100.0f, 0, 1);
            else if (buf[0] == 'S') set_status(&st, buf + 2 <= buf + n ? buf + 2 : "", (float)t);
            else if (buf[0] == '?') sendto(sock, "OK", 2, 0, (struct sockaddr *)&from, fl);
            else if (buf[0] == 'Q') {
                if (nwait < 8) { waiters[nwait] = from; waiter_len[nwait] = fl; nwait++; }
                if (st.quit_at < 0) {
                    st.progress = 1;
                    /* Let the intro finish and the ring close before fading. */
                    float earliest = T_MOVE_1 + .45f;
                    st.quit_at = (float)(t > earliest ? t + .3 : earliest);
                }
            }
            fl = sizeof(from);
        }

        float dt = (float)(t - last);
        last = t;
        st.progress_shown += (st.progress - st.progress_shown) * clampf(dt * 5, 0, 1);

        /* A GPU driver can replace the firmware framebuffer mid-boot. */
        if (++frame % 30 == 0) {
            char name[32];
            fb_name(name, sizeof(name));
            if (strcmp(name, fb.name) != 0) {
                fb_close(&fb);
                if (fb_open(&fb) == 0 && ((int)fb.var.xres != c.w || (int)fb.var.yres != c.h)) {
                    free(c.px);
                    free(c.bg);
                    if (canvas_init(&c, fb.var.xres, fb.var.yres) != 0) break;
                }
            }
        }

        render(&c, (float)t, &st);
        if (fb.fd > 0) fb_present(&fb, &c);

        if (st.quit_at >= 0 && t >= st.quit_at + T_FADE_OUT) break;

        double next = t0 + (frame + 1) / 30.0;
        int wait = (int)((next - now()) * 1000);
        struct pollfd p = {.fd = sock, .events = POLLIN};
        poll(&p, 1, wait > 0 ? wait : 0);
    }

    console_mode(KD_TEXT);
    fb_close(&fb);
    for (int i = 0; i < nwait; i++) sendto(sock, "OK", 2, 0, (struct sockaddr *)&waiters[i], waiter_len[i]);
    close(sock);
    return 0;
}

static int run_preview(int argc, char **argv) {
    if (argc < 7) {
        fprintf(stderr, "usage: bootsplash --preview W H T PCT FILE.ppm [status]\n");
        return 2;
    }
    Canvas c;
    if (canvas_init(&c, atoi(argv[2]), atoi(argv[3])) != 0) return 1;
    float t = (float)atof(argv[4]);
    State st = {.progress = atoi(argv[5]) / 100.0f, .quit_at = -1};
    st.progress_shown = st.progress;
    snprintf(st.status, sizeof(st.status), "%s", argc > 7 ? argv[7] : "");
    st.status_at = -10;
    render(&c, t, &st);
    FILE *f = fopen(argv[6], "wb");
    if (!f) return 1;
    fprintf(f, "P6\n%d %d\n255\n", c.w, c.h);
    fwrite(c.px, 1, (size_t)c.w * c.h * 3, f);
    fclose(f);
    return 0;
}

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "--daemon") == 0) return run_daemon();
    if (argc > 1 && strcmp(argv[1], "--preview") == 0) return run_preview(argc, argv);
    if (argc > 1 && strcmp(argv[1], "--ping") == 0) return client_send("?", 1, 500) == 0 ? 0 : 1;
    if (argc > 1 && strcmp(argv[1], "--text") == 0) { console_mode(KD_TEXT); return 0; }
    if (argc > 1 && (strcmp(argv[1], "--quit") == 0 || strcmp(argv[1], "--clear") == 0)) {
        client_send("Q", 1, 10000);
        return 0;
    }
    if (argc > 1) {
        char msg[128];
        snprintf(msg, sizeof(msg), "P %d", atoi(argv[1]));
        client_send(msg, 0, 0);
        if (argc > 2) {
            snprintf(msg, sizeof(msg), "S %s", argv[2]);
            client_send(msg, 0, 0);
        }
        return 0;
    }
    fprintf(stderr, "usage: bootsplash --daemon | <percent> [status] | --quit | --ping | --text | --preview ...\n");
    return 2;
}
