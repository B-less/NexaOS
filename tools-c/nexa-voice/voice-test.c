/* Unit tests for helix.c: voiceprint and the helix's motion and drawing. */
#include "helix.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails, checks;

#define CHECK(cond, what) do { checks++; if (cond) printf("PASS %s\n", what); \
    else { fails++; printf("FAIL %s (line %d)\n", what, __LINE__); } } while (0)

#define RATE 16000

/* "speech": n syllables, each a 180 ms tone burst, 120 ms apart */
static int16_t *syllables(int n, double amp, double freq, size_t *len)
{
    *len = (size_t)(RATE * (0.3 + n * 0.30));
    int16_t *s = calloc(*len, sizeof *s);
    for (int k = 0; k < n; k++) {
        size_t start = (size_t)(RATE * (0.15 + k * 0.30)), dur = (size_t)(RATE * 0.18);
        for (size_t i = 0; i < dur; i++) {
            double env = sin(M_PI * i / dur);
            s[start + i] = (int16_t)(amp * 32767 * env * sin(2 * M_PI * freq * i / RATE));
        }
    }
    return s;
}

static void voiceprint(void)
{
    vp_state v;
    vp_frame f[400];
    vp_init(&v, RATE);
    int16_t quiet[RATE] = {0};
    int n = vp_feed(&v, quiet, RATE, f, 400);
    int all_zero = 1;
    for (int i = 0; i < n; i++) all_zero &= f[i].level == 0 && f[i].onset == 0;
    CHECK(n == 100 && all_zero, "a second of silence: 100 quiet frames");

    size_t len;
    int16_t *s = syllables(5, 0.5, 220, &len);
    vp_init(&v, RATE);
    n = vp_feed(&v, s, len, f, 400);
    int onsets = 0;
    float peak = 0;
    for (int i = 0; i < n; i++) {
        onsets += f[i].onset > 0;
        if (f[i].level > peak) peak = f[i].level;
    }
    CHECK(onsets == 5, "five syllables, five beats");
    CHECK(peak > 0.7, "a loud voice reads loud");
    free(s);

    /* fed in odd sizes: the same frames */
    s = syllables(3, 0.5, 220, &len);
    vp_frame a[200], b[200];
    vp_init(&v, RATE);
    int na = vp_feed(&v, s, len, a, 200);
    vp_init(&v, RATE);
    int nb = 0;
    for (size_t off = 0; off < len; off += 137) nb += vp_feed(&v, s + off, len - off < 137 ? len - off : 137, b + nb, 200 - nb);
    CHECK(na == nb && !memcmp(a, b, na * sizeof *a), "the same frames however the samples arrive");
    free(s);

    int16_t *lo = syllables(2, 0.5, 150, &len);
    int16_t *hi = syllables(2, 0.5, 2500, &len);
    vp_frame fl[100], fh[100];
    vp_init(&v, RATE);
    int nl = vp_feed(&v, lo, len, fl, 100);
    vp_init(&v, RATE);
    vp_feed(&v, hi, len, fh, 100);
    double bl = 0, bh = 0;
    for (int i = 0; i < nl; i++) { bl += fl[i].bright; bh += fh[i].bright; }
    CHECK(bh > bl * 3, "a high voice is brighter than a low one");
    free(lo);
    free(hi);

    vp_init(&v, RATE);
    CHECK(vp_feed(&v, quiet, RATE, f, 10) == 10, "never writes past the space given");
}

static double opaque_share(cairo_surface_t *s)
{
    cairo_surface_flush(s);
    int w = cairo_image_surface_get_width(s), h = cairo_image_surface_get_height(s), st = cairo_image_surface_get_stride(s);
    unsigned char *d = cairo_image_surface_get_data(s);
    long n = 0;
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++) n += d[y * st + x * 4 + 3] > 40;
    return (double)n / (w * h);
}

/* rows from the top that have drawing in them: how tall the helix is */
static int drawn_height(cairo_surface_t *s)
{
    cairo_surface_flush(s);
    int w = cairo_image_surface_get_width(s), h = cairo_image_surface_get_height(s), st = cairo_image_surface_get_stride(s);
    unsigned char *d = cairo_image_surface_get_data(s);
    int top = -1, bottom = -1;
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++)
            if (d[y * st + x * 4 + 3] > 40) { if (top < 0) top = y; bottom = y; break; }
    return top < 0 ? 0 : bottom - top;
}

static cairo_surface_t *render(const helix *h)
{
    cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, 520, 132);
    cairo_t *cr = cairo_create(s);
    helix_draw(h, cr, 520, 132);
    cairo_destroy(cr);
    return s;
}

static void motion(void)
{
    helix h;
    helix_init(&h, 210, 7);
    for (int i = 0; i < 60; i++) helix_step(&h, 1 / 60.0, 0, 0, 0);
    double rest = h.env;
    cairo_surface_t *s0 = render(&h);
    int h0 = drawn_height(s0);
    CHECK(rest > 0.01 && rest < 0.12, "at rest it breathes, small");
    CHECK(opaque_share(s0) > 0.01, "something is drawn");

    h.mode = HELIX_SPEAKING;
    double rot0 = h.rot, hue0 = h.hue;
    for (int i = 0; i < 30; i++) helix_step(&h, 1 / 60.0, 0.9, i % 10 == 0 ? 0.9 : 0, 0.5);
    CHECK(h.env > 0.7, "a loud voice swells it");
    CHECK(h.pops_total == 3, "each syllable pops it");
    int sparks = 0;
    for (int i = 0; i < HELIX_SPARKS; i++) sparks += h.spark[i].used;
    CHECK(sparks >= 14, "and throws sparks");
    CHECK(h.rot - rot0 > 0.5, "it turns faster while speaking");
    CHECK(fabs(h.hue - hue0) > 10, "the colours move with the voice");
    cairo_surface_t *s1 = render(&h);
    CHECK(drawn_height(s1) > h0 * 2, "speaking, it is much taller than at rest");
    cairo_surface_destroy(s0);
    cairo_surface_destroy(s1);

    for (int i = 0; i < 180; i++) helix_step(&h, 1 / 60.0, 0, 0, 0);
    sparks = 0;
    int pops = 0;
    for (int i = 0; i < HELIX_SPARKS; i++) sparks += h.spark[i].used;
    for (int i = 0; i < HELIX_POPS; i++) pops += h.pop[i].used;
    CHECK(sparks == 0 && pops == 0 && h.env < 0.1, "quiet again: it settles");

    double r0, g0, b0, r1, g1, b1;
    helix_colour(&h, 0.5, 0, &r0, &g0, &b0);
    helix_colour(&h, 0.5, 1, &r1, &g1, &b1);
    CHECK(fabs(r0 - r1) + fabs(g0 - g1) + fabs(b0 - b1) > 0.3, "the two strands have different colours");

    helix calm;
    helix_init(&calm, 120, 1);
    calm.palette = HELIX_CALM;
    for (int i = 0; i < 600; i++) helix_step(&calm, 1 / 60.0, 0, 0, 0);
    CHECK(fabs(calm.hue - 120) < 30, "calm stays near the theme's accent");

    helix still;
    helix_init(&still, 210, 3);
    still.animations = 0;
    still.mode = HELIX_SPEAKING;
    for (int i = 0; i < 60; i++) helix_step(&still, 1 / 60.0, 0.8, i % 15 == 0 ? 0.8 : 0, 0.3);
    sparks = 0;
    for (int i = 0; i < HELIX_SPARKS; i++) sparks += still.spark[i].used;
    CHECK(still.rot == 0 && sparks == 0, "reduce animations: no turning, no sparks");
    CHECK(still.env > 0.5 && still.pops_total == 4, "but it still swells and pops with the voice");

    helix think;
    helix_init(&think, 210, 5);
    think.mode = HELIX_THINKING;
    helix_step(&think, 1.0, 0, 0, 0);
    helix_init(&h, 210, 5);
    helix_step(&h, 1.0, 0, 0, 0);
    CHECK(think.rot > h.rot * 3, "thinking spins fast");

    helix big;
    helix_init(&big, 210, 9);
    helix_step(&big, 5.0, 0, 0, 0);
    CHECK(big.t <= 0.1 + 1e-9, "a long pause is one short step, not a jump");
}

int main(void)
{
    voiceprint();
    motion();
    printf("%d/%d passed\n", checks - fails, checks);
    return fails != 0;
}
