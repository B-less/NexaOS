#include "thumbs.h"

#include <gdk-pixbuf/gdk-pixbuf.h>
#include <glib/gstdio.h>
#include <jpeglib.h>
#include <math.h>
#include <setjmp.h>
#include <stdio.h>
#include <string.h>

#define HUGE_PIXELS 400000000.0  /* refused outright: no real photo is this big */

/* ---- orientation ---- */

cairo_surface_t *orient_surface(cairo_surface_t *src, int o)
{
    int w = cairo_image_surface_get_width(src), h = cairo_image_surface_get_height(src);
    if (o <= 1 || o > 8) return cairo_surface_reference(src);
    int W, H;
    orient_size(o, w, h, &W, &H);
    cairo_surface_t *out = cairo_image_surface_create(cairo_image_surface_get_format(src), W, H);
    if (cairo_surface_status(out)) return out;
    cairo_t *cr = cairo_create(out);
    cairo_matrix_t m;
    switch (o) {
    case 2: cairo_matrix_init(&m, -1, 0, 0, 1, W, 0); break;
    case 3: cairo_matrix_init(&m, -1, 0, 0, -1, W, H); break;
    case 4: cairo_matrix_init(&m, 1, 0, 0, -1, 0, H); break;
    case 5: cairo_matrix_init(&m, 0, 1, 1, 0, 0, 0); break;
    case 6: cairo_matrix_init(&m, 0, 1, -1, 0, W, 0); break;
    case 7: cairo_matrix_init(&m, 0, -1, -1, 0, W, H); break;
    default: cairo_matrix_init(&m, 0, -1, 1, 0, 0, H); break;  /* 8 */
    }
    cairo_set_matrix(cr, &m);
    cairo_set_source_surface(cr, src, 0, 0);
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    cairo_paint(cr);
    cairo_destroy(cr);
    return out;
}

/* How much to shrink a w x h picture (as stored) for the box (as shown). */
static double wanted_scale(int w, int h, int o, int box_w, int box_h, int cover)
{
    if (w <= 0 || h <= 0) return 1;
    double s = 1;
    if (box_w > 0 && box_h > 0) {
        int bw = box_w, bh = box_h;
        if (o >= 5) bw = box_h, bh = box_w;  /* the box as stored */
        s = cover ? fmax((double)bw / w, (double)bh / h) : fmin((double)bw / w, (double)bh / h);
    }
    double cap = sqrt((double)VIEW_MAX_PIXELS / ((double)w * h));
    return fmin(1, fmin(s, cap));
}

/* ---- JPEG, with libjpeg-turbo ---- */

typedef struct {
    struct jpeg_error_mgr pub;
    jmp_buf jump;
    char msg[JMSG_LENGTH_MAX];
} JErr;

static void jpeg_fail(j_common_ptr c)
{
    JErr *e = (JErr *)c->err;
    c->err->format_message(c, e->msg);
    longjmp(e->jump, 1);
}

static void jpeg_quiet(j_common_ptr c) { (void)c; }

/* NULL with *fallback set: not one libjpeg can do (CMYK): try gdk-pixbuf. */
static cairo_surface_t *jpeg_load(const char *path, double scale, int *fallback)
{
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    struct jpeg_decompress_struct cinfo;
    JErr jerr;
    cinfo.err = jpeg_std_error(&jerr.pub);
    jerr.pub.error_exit = jpeg_fail;
    jerr.pub.output_message = jpeg_quiet;
    cairo_surface_t *volatile surf = NULL;
    if (setjmp(jerr.jump)) {
        jpeg_destroy_decompress(&cinfo);
        fclose(f);
        /* What was decoded before damage is kept (libjpeg fills the rest). */
        if (surf) {
            cairo_surface_destroy(surf);
            surf = NULL;
        }
        *fallback = 1;
        return NULL;
    }
    jpeg_create_decompress(&cinfo);
    jpeg_stdio_src(&cinfo, f);
    jpeg_read_header(&cinfo, TRUE);
    if (cinfo.jpeg_color_space == JCS_CMYK || cinfo.jpeg_color_space == JCS_YCCK) {
        jpeg_destroy_decompress(&cinfo);
        fclose(f);
        *fallback = 1;
        return NULL;
    }
    double px = (double)cinfo.image_width * cinfo.image_height;
    if (px > HUGE_PIXELS * 64) longjmp(jerr.jump, 1);
    int denom = 1;
    for (int d = 8; d > 1; d /= 2)
        if (1.0 / d >= scale) { denom = d; break; }
    while (denom < 8 && px / ((double)denom * denom) > VIEW_MAX_PIXELS) denom *= 2;
    if (px / ((double)denom * denom) > VIEW_MAX_PIXELS * 1.5) {
        jpeg_destroy_decompress(&cinfo);
        fclose(f);
        return NULL;  /* too big even at 1/8 */
    }
    cinfo.scale_num = 1;
    cinfo.scale_denom = (unsigned)denom;
    cinfo.out_color_space = G_BYTE_ORDER == G_LITTLE_ENDIAN ? JCS_EXT_BGRX : JCS_EXT_XRGB;
    jpeg_start_decompress(&cinfo);
    int w = (int)cinfo.output_width, h = (int)cinfo.output_height;
    surf = cairo_image_surface_create(CAIRO_FORMAT_RGB24, w, h);
    if (cairo_surface_status(surf)) longjmp(jerr.jump, 1);
    unsigned char *data = cairo_image_surface_get_data(surf);
    int stride = cairo_image_surface_get_stride(surf);
    cairo_surface_flush(surf);
    while (cinfo.output_scanline < cinfo.output_height) {
        JSAMPROW row = data + (size_t)cinfo.output_scanline * (size_t)stride;
        if (jpeg_read_scanlines(&cinfo, &row, 1) != 1) break;
    }
    cairo_surface_mark_dirty(surf);
    jpeg_finish_decompress(&cinfo);
    jpeg_destroy_decompress(&cinfo);
    fclose(f);
    return surf;
}

/* ---- everything else, with gdk-pixbuf ---- */

static cairo_surface_t *from_pixbuf(GdkPixbuf *pb)
{
    int w = gdk_pixbuf_get_width(pb), h = gdk_pixbuf_get_height(pb), n = gdk_pixbuf_get_n_channels(pb);
    int alpha = gdk_pixbuf_get_has_alpha(pb), rs = gdk_pixbuf_get_rowstride(pb);
    if (gdk_pixbuf_get_bits_per_sample(pb) != 8 || n < 3) return NULL;
    cairo_surface_t *s = cairo_image_surface_create(alpha ? CAIRO_FORMAT_ARGB32 : CAIRO_FORMAT_RGB24, w, h);
    if (cairo_surface_status(s)) {
        cairo_surface_destroy(s);
        return NULL;
    }
    const guint8 *src = gdk_pixbuf_read_pixels(pb);
    unsigned char *dst = cairo_image_surface_get_data(s);
    int ds = cairo_image_surface_get_stride(s);
    cairo_surface_flush(s);
    for (int y = 0; y < h; y++) {
        const guint8 *p = src + (size_t)y * rs;
        guint32 *q = (guint32 *)(void *)(dst + (size_t)y * ds);
        for (int x = 0; x < w; x++, p += n) {
            guint32 a = alpha ? p[3] : 255;
            guint32 r = p[0] * a / 255, g = p[1] * a / 255, b = p[2] * a / 255;
            q[x] = a << 24 | r << 16 | g << 8 | b;
        }
    }
    cairo_surface_mark_dirty(s);
    return s;
}

static cairo_surface_t *pixbuf_load(const Photo *p, double scale, char **why)
{
    GError *err = NULL;
    GdkPixbuf *pb;
    int w = p->info.width, h = p->info.height;
    if (w > 0 && h > 0 && scale < 0.999)
        pb = gdk_pixbuf_new_from_file_at_scale(p->path, MAX(1, (int)ceil(w * scale)), MAX(1, (int)ceil(h * scale)), TRUE, &err);
    else
        pb = gdk_pixbuf_new_from_file(p->path, &err);
    if (!pb) {
        if (why) *why = g_strdup("Photos can't read this picture. It may be damaged, or in a format Photos doesn't know.");
        g_clear_error(&err);
        return NULL;
    }
    cairo_surface_t *s = from_pixbuf(pb);
    g_object_unref(pb);
    if (!s && why) *why = g_strdup("Photos can't show this kind of picture.");
    return s;
}

/* ---- both ---- */

static cairo_surface_t *scale_to(cairo_surface_t *s, double k)
{
    int w = cairo_image_surface_get_width(s), h = cairo_image_surface_get_height(s);
    int W = MAX(1, (int)lround(w * k)), H = MAX(1, (int)lround(h * k));
    if (W == w && H == h) return cairo_surface_reference(s);
    cairo_surface_t *out = cairo_image_surface_create(cairo_image_surface_get_format(s), W, H);
    cairo_t *cr = cairo_create(out);
    cairo_scale(cr, (double)W / w, (double)H / h);
    cairo_set_source_surface(cr, s, 0, 0);
    cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_GOOD);
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    cairo_paint(cr);
    cairo_destroy(cr);
    return out;
}

cairo_surface_t *picture_load(const Photo *p, int o, int box_w, int box_h, int cover, char **why)
{
    int w = p->info.width, h = p->info.height;
    if ((double)w * h > HUGE_PIXELS) {
        if (why) *why = g_strdup("This picture is too big for Photos to show.");
        return NULL;
    }
    double scale = wanted_scale(w, h, o, box_w, box_h, cover);
    cairo_surface_t *s = NULL;
    FILE *f = fopen(p->path, "rb");
    unsigned char head[16] = {0};
    size_t n = f ? fread(head, 1, sizeof head, f) : 0;
    if (f) fclose(f);
    if (!f) {
        if (why) *why = g_strdup("Photos can't open this picture any more. It may have been moved or deleted.");
        return NULL;
    }
    if (photo_kind(head, n) == KIND_JPEG) {
        int fallback = 0;
        s = jpeg_load(p->path, scale, &fallback);
        if (!s && !fallback) {
            if (why) *why = g_strdup("This picture is too big for Photos to show.");
            return NULL;
        }
    }
    if (!s) s = pixbuf_load(p, scale, why);
    if (!s) return NULL;
    cairo_surface_t *up = orient_surface(s, o);
    cairo_surface_destroy(s);
    if (cairo_surface_status(up)) {
        cairo_surface_destroy(up);
        if (why) *why = g_strdup("There isn't enough memory to show this picture.");
        return NULL;
    }
    /* Exactly the size asked for (libjpeg only shrinks by halves). */
    if (box_w > 0 && box_h > 0) {
        int W = cairo_image_surface_get_width(up), H = cairo_image_surface_get_height(up);
        double k = cover ? fmax((double)box_w / W, (double)box_h / H) : fmin((double)box_w / W, (double)box_h / H);
        if (k < 1) {
            cairo_surface_t *t = scale_to(up, k);
            cairo_surface_destroy(up);
            up = t;
        }
    }
    return up;
}

char *thumb_path(const Photo *p, int o, const char *cache_dir)
{
    char *key = g_strdup_printf("%s\n%" G_GINT64_FORMAT "\n%" G_GINT64_FORMAT "\n%d\n%d", p->path, p->mtime, p->size, o, THUMB_PX);
    char *sum = g_compute_checksum_for_string(G_CHECKSUM_SHA1, key, -1);
    char *name = g_strconcat(sum, ".png", NULL);
    char *path = g_build_filename(cache_dir, "thumbs", name, NULL);
    g_free(key);
    g_free(sum);
    g_free(name);
    return path;
}

cairo_surface_t *thumb_square(const Photo *p, int o, const char *cache_dir)
{
    char *path = cache_dir ? thumb_path(p, o, cache_dir) : NULL;
    if (path && g_file_test(path, G_FILE_TEST_IS_REGULAR)) {
        cairo_surface_t *s = cairo_image_surface_create_from_png(path);
        if (!cairo_surface_status(s) && cairo_image_surface_get_width(s) == THUMB_PX &&
            cairo_image_surface_get_height(s) == THUMB_PX) {
            g_free(path);
            return s;
        }
        cairo_surface_destroy(s);  /* damaged: make it again */
    }
    cairo_surface_t *pic = picture_load(p, o, THUMB_PX, THUMB_PX, 1, NULL);
    if (!pic) {
        g_free(path);
        return NULL;
    }
    cairo_surface_t *sq = cairo_image_surface_create(CAIRO_FORMAT_RGB24, THUMB_PX, THUMB_PX);
    cairo_t *cr = cairo_create(sq);
    int w = cairo_image_surface_get_width(pic), h = cairo_image_surface_get_height(pic);
    double k = fmax((double)THUMB_PX / w, (double)THUMB_PX / h);
    cairo_set_source_rgb(cr, 0.06, 0.08, 0.15);  /* under see-through pictures: inside media, not UI */
    cairo_paint(cr);
    cairo_translate(cr, (THUMB_PX - w * k) / 2, (THUMB_PX - h * k) / 2);
    cairo_scale(cr, k, k);
    cairo_set_source_surface(cr, pic, 0, 0);
    cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_GOOD);
    cairo_paint(cr);
    cairo_destroy(cr);
    cairo_surface_destroy(pic);
    if (path) {
        char *dir = g_path_get_dirname(path), *tmp = g_strdup_printf("%s.%p.part", path, (void *)g_thread_self());  /* threads may make the same one */
        g_mkdir_with_parents(dir, 0700);
        if (cairo_surface_write_to_png(sq, tmp) == CAIRO_STATUS_SUCCESS) g_rename(tmp, path);
        else g_unlink(tmp);
        g_free(dir);
        g_free(tmp);
    }
    g_free(path);
    return sq;
}
