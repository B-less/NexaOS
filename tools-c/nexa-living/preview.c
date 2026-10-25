/* nexa-living-preview: one frame of a style to a PNG, or a timing run.
 *   nexa-living-preview STYLE W H T FILE.png [QUALITY]
 *   nexa-living-preview --bench STYLE W H FRAMES QUALITY   (ms per frame) */
#define _GNU_SOURCE
#include "styles.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static struct region R;

static void base(cairo_t *cr, int w, int h)
{
    cairo_pattern_t *g = cairo_pattern_create_linear(0, 0, w, h);
    cairo_pattern_add_color_stop_rgb(g, 0, 0.031, 0.047, 0.118);
    cairo_pattern_add_color_stop_rgb(g, 1, 0.07, 0.06, 0.16);
    cairo_set_source(cr, g);
    cairo_paint(cr);
    cairo_pattern_destroy(g);
}

static struct params P(double t, int q)
{
    return (struct params){.t = t, .speed = 1, .bright = 1, .density = 1, .level = 0.35, .focus = 1, .px = -1e9, .py = -1e9,
                           .reaction = 0.6, .quality = q, .a1 = {0.24, 0.55, 1.0}, .a2 = {0.54, 0.36, 1.0}, .a3 = {0.92, 0.95, 1.0}};
}

int main(int argc, char **argv)
{
    if (argc >= 7 && !strcmp(argv[1], "--bench")) {
        int st = style_by_name(argv[2]), w = atoi(argv[3]), h = atoi(argv[4]), n = atoi(argv[5]), q = atoi(argv[6]);
        if (st < 0) return 2;
        /* As the renderer does it: the base is made once and copied. */
        cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, w, h);
        cairo_surface_t *bs = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, w, h);
        cairo_t *cb = cairo_create(bs);
        base(cb, w, h);
        cairo_destroy(cb);
        cairo_t *cr = cairo_create(s);
        region_prepare(&R, st, w, h, 1, 7);
        struct params p0 = P(0, q);
        style_draw(cr, &R, &p0);  /* caches made before timing */
        struct timespec a, b;
        clock_gettime(CLOCK_MONOTONIC, &a);
        for (int i = 0; i < n; i++) {
            struct params p = P(i / 30.0, q);
            cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
            cairo_set_source_surface(cr, bs, 0, 0);
            cairo_paint(cr);
            cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
            style_draw(cr, &R, &p);
        }
        cairo_surface_flush(s);
        clock_gettime(CLOCK_MONOTONIC, &b);
        printf("%s %.2f\n", argv[2], ((b.tv_sec - a.tv_sec) * 1e3 + (b.tv_nsec - a.tv_nsec) / 1e6) / n);
        return 0;
    }
    if (argc < 6) return 2;
    int st = style_by_name(argv[1]), w = atoi(argv[2]), h = atoi(argv[3]), q = argc > 6 ? atoi(argv[6]) : 2;
    double t = atof(argv[4]);
    if (st < 0) return 2;
    cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, w, h);
    cairo_t *cr = cairo_create(s);
    region_prepare(&R, st, w, h, 1, 7);
    /* run up to t so styles with state (particles) look settled */
    for (double tt = t - 3; tt <= t; tt += 1 / 30.0) {
        struct params p = P(tt, q);
        base(cr, w, h);
        style_draw(cr, &R, &p);
    }
    return cairo_surface_write_to_png(s, argv[5]) == CAIRO_STATUS_SUCCESS ? 0 : 1;
}
