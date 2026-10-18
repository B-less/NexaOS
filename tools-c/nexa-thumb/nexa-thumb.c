/*
 * nexa-thumb: thumbnails for the Files app, as the freedesktop.org
 * Thumbnail Managing Standard describes them (shared with other apps):
 * ~/.cache/thumbnails/{normal,large,x-large}/<md5 of the file's URI>.png,
 * carrying Thumb::URI and Thumb::MTime so a changed file gets a new one.
 *
 *   nexa-thumb [--size 128|256|512] [--accent RRGGBB] [--folder] PATH...
 *   nexa-thumb --mark-failed PATH        (a file that made us hang/crash)
 *
 * Pictures through gdk-pixbuf, a frame from 10% into videos through
 * GStreamer, the first page of PDFs through poppler (loaded at run time).
 * --folder makes a folder's thumbnail: the folder shape with up to three
 * of the pictures inside peeking out (cached under thumbnails/nexa-folders).
 *
 * Output, one line each: "start\tPATH" before working on a file, then
 * "ok\tPATH\tTHUMBNAIL" or "none\tPATH". Files it can't read are recorded in
 * thumbnails/fail/ so they aren't tried again until they change.
 *
 * Decoding someone else's file is risky, so before touching any input the
 * process confines itself with Landlock (read: the system and the inputs'
 * folders; write: the thumbnail cache only) and caps its memory and CPU.
 */
#define _GNU_SOURCE
#include <cairo.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <gdk-pixbuf/gdk-pixbuf.h>
#include <gio/gio.h>
#include <gst/gst.h>
#include <gst/video/video.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#define MAX_PIXELS 100000000LL /* refuse decompression bombs */
#define SOFTWARE "NexaOS nexa-thumb"

/* ---------- Landlock (raw syscalls; no libc wrappers on musl) ---------- */

#define LL_EXECUTE (1ULL << 0)
#define LL_WRITE_FILE (1ULL << 1)
#define LL_READ_FILE (1ULL << 2)
#define LL_READ_DIR (1ULL << 3)
#define LL_REMOVE_DIR (1ULL << 4)
#define LL_REMOVE_FILE (1ULL << 5)
#define LL_MAKE_DIR (1ULL << 7)
#define LL_MAKE_REG (1ULL << 8)
#define LL_ALL_V1 ((1ULL << 13) - 1)
#define LL_REFER (1ULL << 13)
#define LL_TRUNCATE (1ULL << 14)
#define LL_READ (LL_READ_FILE | LL_READ_DIR)
#define LL_RW (LL_READ | LL_WRITE_FILE | LL_REMOVE_FILE | LL_REMOVE_DIR | LL_MAKE_DIR | LL_MAKE_REG | LL_REFER | LL_TRUNCATE)

struct ll_ruleset_attr { uint64_t handled_access_fs; };
struct ll_path_beneath { uint64_t allowed_access; int32_t parent_fd; } __attribute__((packed));

static int ll_add(int ruleset, const char *path, uint64_t access, uint64_t handled)
{
    struct stat st;
    if (stat(path, &st) != 0)
        return 0; /* missing paths are simply not allowed */
    int fd = open(path, O_PATH | O_CLOEXEC);
    if (fd < 0)
        return -1;
    struct ll_path_beneath rule = { access & handled, fd };
    if (!S_ISDIR(st.st_mode))
        rule.allowed_access &= LL_EXECUTE | LL_WRITE_FILE | LL_READ_FILE | LL_TRUNCATE;
    int rc = syscall(445 /* landlock_add_rule */, ruleset, 1 /* PATH_BENEATH */, &rule, 0);
    close(fd);
    return rc;
}

/* Returns the ABI used, 0 without Landlock, -1 on error. */
static int confine(char **read_paths, int n_read, char **write_paths, int n_write)
{
    long abi = syscall(444 /* landlock_create_ruleset */, NULL, 0, 1U /* VERSION */);
    if (abi < 1)
        return 0;
    uint64_t handled = LL_ALL_V1 | (abi >= 2 ? LL_REFER : 0) | (abi >= 3 ? LL_TRUNCATE : 0);
    struct ll_ruleset_attr attr = { handled };
    int rs = syscall(444, &attr, sizeof attr, 0);
    if (rs < 0)
        return -1;
    /* gdk-pixbuf decodes through glycin, which lists / to build its own
     * sandbox: folder names may be listed anywhere, file contents only read
     * where the rules below say. */
    if (ll_add(rs, "/", LL_READ_DIR, handled) < 0)
        goto fail;
    const char *system[] = { "/usr", "/lib", "/etc", "/proc", "/sys" };
    for (size_t i = 0; i < G_N_ELEMENTS(system); i++)
        if (ll_add(rs, system[i], LL_READ | LL_EXECUTE, handled) < 0)
            goto fail;
    if (ll_add(rs, "/dev/null", LL_READ_FILE | LL_WRITE_FILE, handled) < 0
        || ll_add(rs, "/dev/urandom", LL_READ_FILE, handled) < 0
        || ll_add(rs, "/dev/shm", LL_RW, handled) < 0)
        goto fail;
    for (int i = 0; i < n_read; i++)
        if (ll_add(rs, read_paths[i], LL_READ, handled) < 0)
            goto fail;
    for (int i = 0; i < n_write; i++)
        if (ll_add(rs, write_paths[i], LL_RW, handled) < 0)
            goto fail;
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 || syscall(446 /* restrict_self */, rs, 0) != 0)
        goto fail;
    close(rs);
    return (int)abi;
fail:
    close(rs);
    return -1;
}

/* ---------- pixbuf <-> cairo (no GDK needed) ---------- */

static cairo_surface_t *surface_from_pixbuf(GdkPixbuf *p)
{
    int w = gdk_pixbuf_get_width(p), h = gdk_pixbuf_get_height(p);
    int n = gdk_pixbuf_get_n_channels(p), rs = gdk_pixbuf_get_rowstride(p);
    gboolean alpha = gdk_pixbuf_get_has_alpha(p);
    const guchar *src = gdk_pixbuf_read_pixels(p);
    cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, w, h);
    unsigned char *dst = cairo_image_surface_get_data(s);
    int ds = cairo_image_surface_get_stride(s);
    cairo_surface_flush(s);
    for (int y = 0; y < h; y++) {
        const guchar *sp = src + (size_t)y * rs;
        uint32_t *dp = (uint32_t *)(dst + (size_t)y * ds);
        for (int x = 0; x < w; x++, sp += n) {
            unsigned a = alpha ? sp[3] : 255;
            unsigned r = sp[0] * a / 255, g = sp[1] * a / 255, b = sp[2] * a / 255;
            dp[x] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }
    cairo_surface_mark_dirty(s);
    return s;
}

static GdkPixbuf *pixbuf_from_surface(cairo_surface_t *s)
{
    cairo_surface_flush(s);
    int w = cairo_image_surface_get_width(s), h = cairo_image_surface_get_height(s);
    int ss = cairo_image_surface_get_stride(s);
    const unsigned char *src = cairo_image_surface_get_data(s);
    GdkPixbuf *p = gdk_pixbuf_new(GDK_COLORSPACE_RGB, TRUE, 8, w, h);
    guchar *dst = gdk_pixbuf_get_pixels(p);
    int ds = gdk_pixbuf_get_rowstride(p);
    for (int y = 0; y < h; y++) {
        const uint32_t *sp = (const uint32_t *)(src + (size_t)y * ss);
        guchar *dp = dst + (size_t)y * ds;
        for (int x = 0; x < w; x++, dp += 4) {
            uint32_t v = sp[x];
            unsigned a = v >> 24, r = (v >> 16) & 255, g = (v >> 8) & 255, b = v & 255;
            if (a) {
                r = r * 255 / a; g = g * 255 / a; b = b * 255 / a;
            }
            dp[0] = r; dp[1] = g; dp[2] = b; dp[3] = a;
        }
    }
    return p;
}

static GdkPixbuf *fit(GdkPixbuf *p, int size)
{
    int w = gdk_pixbuf_get_width(p), h = gdk_pixbuf_get_height(p);
    if (w <= size && h <= size)
        return p;
    double k = (double)size / (w > h ? w : h);
    int nw = w * k < 1 ? 1 : (int)lround(w * k), nh = h * k < 1 ? 1 : (int)lround(h * k);
    GdkPixbuf *s = gdk_pixbuf_scale_simple(p, nw, nh, GDK_INTERP_BILINEAR);
    g_object_unref(p);
    return s;
}

/* ---------- the cache ---------- */

static char *cache_root(void)
{
    const char *x = g_getenv("XDG_CACHE_HOME");
    if (x && g_path_is_absolute(x))
        return g_build_filename(x, "thumbnails", NULL);
    return g_build_filename(g_get_home_dir(), ".cache", "thumbnails", NULL);
}

static int bucket_px(int size)
{
    return size <= 128 ? 128 : size <= 256 ? 256 : 512;
}

static const char *bucket_name(int size)
{
    return size <= 128 ? "normal" : size <= 256 ? "large" : "x-large";
}

static char *md5_of(const char *s)
{
    return g_compute_checksum_for_string(G_CHECKSUM_MD5, s, -1);
}

static gboolean fresh(const char *png, const char *mtime, const char *key_name)
{
    GdkPixbuf *p = gdk_pixbuf_new_from_file(png, NULL);
    if (!p)
        return FALSE;
    const char *v = gdk_pixbuf_get_option(p, key_name);
    gboolean ok = v && strcmp(v, mtime) == 0;
    g_object_unref(p);
    return ok;
}

static gboolean save(GdkPixbuf *p, const char *png, const char *uri, const char *mtime, const char *mime, const char *key)
{
    char *tmp = g_strconcat(png, ".nexa-tmp", NULL);
    gboolean ok = key ? gdk_pixbuf_save(p, tmp, "png", NULL, "tEXt::Nexa::Key", key, "tEXt::Software", SOFTWARE, NULL)
                      : gdk_pixbuf_save(p, tmp, "png", NULL, "tEXt::Thumb::URI", uri, "tEXt::Thumb::MTime", mtime,
                                        "tEXt::Thumb::Mimetype", mime, "tEXt::Software", SOFTWARE, NULL);
    if (ok) {
        chmod(tmp, 0600);
        ok = rename(tmp, png) == 0;
    }
    if (!ok)
        unlink(tmp);
    g_free(tmp);
    return ok;
}

static void mark_failed(const char *root, const char *uri, const char *mtime)
{
    char *dir = g_build_filename(root, "fail", "nexa-thumb", NULL);
    g_mkdir_with_parents(dir, 0700);
    char *md5 = md5_of(uri), *name = g_strconcat(md5, ".png", NULL), *png = g_build_filename(dir, name, NULL);
    GdkPixbuf *dot = gdk_pixbuf_new(GDK_COLORSPACE_RGB, TRUE, 8, 1, 1);
    gdk_pixbuf_fill(dot, 0);
    save(dot, png, uri, mtime, "application/octet-stream", NULL);
    g_object_unref(dot);
    g_free(dir); g_free(md5); g_free(name); g_free(png);
}

/* ---------- makers ---------- */

static GdkPixbuf *picture(const char *path, int size)
{
    int w = 0, h = 0;
    if (!gdk_pixbuf_get_file_info(path, &w, &h) || (long long)w * h > MAX_PIXELS)
        return NULL;
    GdkPixbuf *p = (w > size || h > size) ? gdk_pixbuf_new_from_file_at_scale(path, size, size, TRUE, NULL)
                                          : gdk_pixbuf_new_from_file(path, NULL);
    if (!p)
        return NULL;
    GdkPixbuf *o = gdk_pixbuf_apply_embedded_orientation(p);
    g_object_unref(p);
    return fit(o, size);
}

/* Film-strip edges, so a video thumbnail reads as a video. */
static GdkPixbuf *film_strip(GdkPixbuf *p)
{
    cairo_surface_t *s = surface_from_pixbuf(p);
    int w = cairo_image_surface_get_width(s), h = cairo_image_surface_get_height(s);
    cairo_t *cr = cairo_create(s);
    double band = w / 14.0 < 4 ? 4 : w / 14.0, hole = band * 0.5;
    cairo_set_source_rgba(cr, 0.05, 0.06, 0.09, 0.92);
    cairo_rectangle(cr, 0, 0, band, h);
    cairo_rectangle(cr, w - band, 0, band, h);
    cairo_fill(cr);
    cairo_set_source_rgba(cr, 0.92, 0.94, 0.98, 0.85);
    for (double y = hole * 0.6; y + hole < h; y += hole * 1.9) {
        cairo_rectangle(cr, (band - hole) / 2, y, hole, hole * 0.8);
        cairo_rectangle(cr, w - band + (band - hole) / 2, y, hole, hole * 0.8);
    }
    cairo_fill(cr);
    cairo_destroy(cr);
    GdkPixbuf *out = pixbuf_from_surface(s);
    cairo_surface_destroy(s);
    g_object_unref(p);
    return out;
}

static void free_copy(guchar *pixels, gpointer data) { (void)data; g_free(pixels); }

static GdkPixbuf *video_frame(const char *uri, int size)
{
    GstElement *pb = gst_element_factory_make("playbin", NULL);
    if (!pb)
        return NULL;
    g_object_set(pb, "uri", uri, "video-sink", gst_element_factory_make("fakesink", NULL),
                 "audio-sink", gst_element_factory_make("fakesink", NULL), "flags", 1 /* video only */, NULL);
    GdkPixbuf *out = NULL;
    if (gst_element_set_state(pb, GST_STATE_PAUSED) == GST_STATE_CHANGE_FAILURE
        || gst_element_get_state(pb, NULL, NULL, 6 * GST_SECOND) != GST_STATE_CHANGE_SUCCESS)
        goto done;
    gint64 dur = 0;
    if (gst_element_query_duration(pb, GST_FORMAT_TIME, &dur) && dur > 0) {
        gint64 at = dur / 10 < 30 * GST_SECOND ? dur / 10 : 30 * GST_SECOND;
        gst_element_seek_simple(pb, GST_FORMAT_TIME, GST_SEEK_FLAG_FLUSH | GST_SEEK_FLAG_KEY_UNIT, at);
        gst_element_get_state(pb, NULL, NULL, 6 * GST_SECOND);
    }
    GstCaps *want = gst_caps_new_simple("video/x-raw", "format", G_TYPE_STRING, "RGB",
                                        "pixel-aspect-ratio", GST_TYPE_FRACTION, 1, 1, NULL);
    GstSample *sample = NULL;
    g_signal_emit_by_name(pb, "convert-sample", want, &sample);
    gst_caps_unref(want);
    if (sample) {
        GstVideoInfo vi;
        GstBuffer *buf = gst_sample_get_buffer(sample);
        GstMapInfo map;
        if (gst_video_info_from_caps(&vi, gst_sample_get_caps(sample)) && buf && gst_buffer_map(buf, &map, GST_MAP_READ)) {
            int w = GST_VIDEO_INFO_WIDTH(&vi), h = GST_VIDEO_INFO_HEIGHT(&vi), stride = GST_VIDEO_INFO_PLANE_STRIDE(&vi, 0);
            if (w > 0 && h > 0 && (gsize)stride * h <= map.size) {
                guchar *copy = g_memdup2(map.data, (gsize)stride * h);
                out = fit(gdk_pixbuf_new_from_data(copy, GDK_COLORSPACE_RGB, FALSE, 8, w, h, stride, free_copy, NULL), size);
            }
            gst_buffer_unmap(buf, &map);
        }
        gst_sample_unref(sample);
    }
done:
    gst_element_set_state(pb, GST_STATE_NULL);
    gst_object_unref(pb);
    return out ? film_strip(out) : NULL;
}

/* poppler-glib, loaded only when a PDF needs it. */
static GdkPixbuf *pdf_page(const char *uri, int size)
{
    static void *lib;
    static void *(*doc_new)(const char *, const char *, GError **);
    static void *(*get_page)(void *, int);
    static void (*page_size)(void *, double *, double *);
    static void (*render)(void *, cairo_t *);
    if (!lib) {
        lib = dlopen("libpoppler-glib.so.8", RTLD_NOW | RTLD_LOCAL);
        if (!lib)
            return NULL;
        doc_new = dlsym(lib, "poppler_document_new_from_file");
        get_page = dlsym(lib, "poppler_document_get_page");
        page_size = dlsym(lib, "poppler_page_get_size");
        render = dlsym(lib, "poppler_page_render");
    }
    if (!doc_new || !get_page || !page_size || !render)
        return NULL;
    void *doc = doc_new(uri, NULL, NULL);
    if (!doc)
        return NULL;
    void *page = get_page(doc, 0);
    GdkPixbuf *out = NULL;
    if (page) {
        double w = 0, h = 0;
        page_size(page, &w, &h);
        if (w > 0 && h > 0) {
            double k = size / (w > h ? w : h);
            cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, (int)ceil(w * k), (int)ceil(h * k));
            cairo_t *cr = cairo_create(s);
            cairo_set_source_rgb(cr, 1, 1, 1);
            cairo_paint(cr);
            cairo_scale(cr, k, k);
            render(page, cr);
            cairo_destroy(cr);
            out = pixbuf_from_surface(s);
            cairo_surface_destroy(s);
        }
        g_object_unref(page);
    }
    g_object_unref(doc);
    return out;
}

typedef enum { NONE, PICTURE, VIDEO, PDF } Kind;

static Kind kind_of(const char *path, char **mime_out)
{
    gboolean uncertain;
    char *ct = g_content_type_guess(path, NULL, 0, &uncertain);
    char *mime = g_content_type_get_mime_type(ct);
    g_free(ct);
    Kind k = NONE;
    if (mime) {
        if (g_str_has_prefix(mime, "image/"))
            k = PICTURE;
        else if (g_str_has_prefix(mime, "video/"))
            k = VIDEO;
        else if (strcmp(mime, "application/pdf") == 0)
            k = PDF;
    }
    *mime_out = mime ? mime : g_strdup("application/octet-stream");
    return k;
}

/* The thumbnail of one file: path of the PNG, or NULL. */
static char *thumbnail(const char *root, const char *path, int size)
{
    struct stat st;
    if (stat(path, &st) != 0 || !S_ISREG(st.st_mode) || g_str_has_prefix(path, root))
        return NULL;
    char *mime = NULL;
    Kind kind = kind_of(path, &mime);
    char *uri = g_filename_to_uri(path, NULL, NULL), *result = NULL;
    if (kind == NONE || !uri)
        goto out;
    char mtime[32];
    snprintf(mtime, sizeof mtime, "%lld", (long long)st.st_mtime);
    char *md5 = md5_of(uri), *name = g_strconcat(md5, ".png", NULL);
    char *png = g_build_filename(root, bucket_name(size), name, NULL);
    char *failed = g_build_filename(root, "fail", "nexa-thumb", name, NULL);
    if (fresh(png, mtime, "tEXt::Thumb::MTime")) {
        result = png;
        png = NULL;
    } else if (!fresh(failed, mtime, "tEXt::Thumb::MTime")) {
        int px = bucket_px(size);
        GdkPixbuf *p = kind == PICTURE ? picture(path, px) : kind == VIDEO ? video_frame(uri, px) : pdf_page(uri, px);
        if (p && save(p, png, uri, mtime, mime, NULL)) {
            result = png;
            png = NULL;
        } else if (!p) {
            mark_failed(root, uri, mtime);
        }
        if (p)
            g_object_unref(p);
    }
    g_free(md5); g_free(name); g_free(png); g_free(failed);
out:
    g_free(mime);
    g_free(uri);
    return result;
}

static void rounded(cairo_t *cr, double x, double y, double w, double h, double r)
{
    cairo_new_sub_path(cr);
    cairo_arc(cr, x + w - r, y + r, r, -M_PI / 2, 0);
    cairo_arc(cr, x + w - r, y + h - r, r, 0, M_PI / 2);
    cairo_arc(cr, x + r, y + h - r, r, M_PI / 2, M_PI);
    cairo_arc(cr, x + r, y + r, r, M_PI, 3 * M_PI / 2);
    cairo_close_path(cr);
}

typedef struct { char *path; time_t mtime; } Pic;

static gint newest_first(gconstpointer a, gconstpointer b)
{
    const Pic *x = a, *y = b;
    return (y->mtime > x->mtime) - (y->mtime < x->mtime);
}

/* A folder's thumbnail: its shape, with up to three pictures inside. */
static char *folder_thumbnail(const char *root, const char *dir, int size, double ar, double ag, double ab)
{
    struct stat st;
    if (stat(dir, &st) != 0 || !S_ISDIR(st.st_mode))
        return NULL;
    GDir *d = g_dir_open(dir, 0, NULL);
    if (!d)
        return NULL;
    GArray *pics = g_array_new(FALSE, FALSE, sizeof(Pic));
    const char *name;
    while ((name = g_dir_read_name(d)) && pics->len < 400) {
        if (name[0] == '.')
            continue;
        char *p = g_build_filename(dir, name, NULL), *mime = NULL;
        struct stat ps;
        if (stat(p, &ps) == 0 && S_ISREG(ps.st_mode) && kind_of(p, &mime) == PICTURE) {
            Pic pic = { p, ps.st_mtime };
            g_array_append_val(pics, pic);
        } else {
            g_free(p);
        }
        g_free(mime);
    }
    g_dir_close(d);
    g_array_sort(pics, newest_first);
    int n = pics->len < 3 ? pics->len : 3;
    char *result = NULL;
    if (n == 0)
        goto out;
    GString *key = g_string_new(NULL);
    g_string_append_printf(key, "%lld %d %02x%02x%02x", (long long)st.st_mtime, size,
                           (int)(ar * 255), (int)(ag * 255), (int)(ab * 255));
    for (int i = 0; i < n; i++) {
        Pic *pic = &g_array_index(pics, Pic, i);
        g_string_append_printf(key, "|%s:%lld", pic->path, (long long)pic->mtime);
    }
    char *uri = g_filename_to_uri(dir, NULL, NULL), *md5 = md5_of(uri ? uri : dir);
    char *fname = g_strconcat(md5, ".png", NULL);
    char *bucket = g_build_filename(root, "nexa-folders", bucket_name(size), NULL);
    g_mkdir_with_parents(bucket, 0700);
    char *png = g_build_filename(bucket, fname, NULL);
    if (fresh(png, key->str, "tEXt::Nexa::Key")) {
        result = png;
        png = NULL;
        goto done;
    }
    int S = bucket_px(size);
    cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, S, S);
    cairo_t *cr = cairo_create(s);
    /* Back of the folder, with its tab. */
    cairo_set_source_rgb(cr, ar * 0.62, ag * 0.62, ab * 0.62);
    rounded(cr, S * 0.06, S * 0.10, S * 0.36, S * 0.16, S * 0.04);
    cairo_fill(cr);
    rounded(cr, S * 0.06, S * 0.16, S * 0.88, S * 0.72, S * 0.06);
    cairo_fill(cr);
    /* Pictures peeking out, newest in front. */
    static const double dx[3] = { 0.16, -0.16, 0.0 }, rot[3] = { 7.0, -7.0, 0.0 };
    for (int i = n - 1; i >= 0; i--) {
        char *thumb = thumbnail(root, g_array_index(pics, Pic, i).path, size);
        GdkPixbuf *p = thumb ? gdk_pixbuf_new_from_file(thumb, NULL) : NULL;
        g_free(thumb);
        if (!p)
            continue;
        cairo_surface_t *img = surface_from_pixbuf(p);
        double cw = S * 0.56, ch = S * 0.44, pw = gdk_pixbuf_get_width(p), ph = gdk_pixbuf_get_height(p);
        int slot = n == 1 ? 2 : n == 2 ? i : i;
        cairo_save(cr);
        cairo_translate(cr, S * (0.5 + dx[slot] * (n > 1)), S * 0.36);
        cairo_rotate(cr, rot[slot] * (n > 1) * M_PI / 180.0);
        cairo_set_source_rgba(cr, 0, 0, 0, 0.25);
        rounded(cr, -cw / 2 + S * 0.01, -ch / 2 + S * 0.015, cw, ch, S * 0.02);
        cairo_fill(cr);
        cairo_set_source_rgb(cr, 0.98, 0.98, 0.99);
        rounded(cr, -cw / 2, -ch / 2, cw, ch, S * 0.02);
        cairo_fill(cr);
        double iw = cw - S * 0.04, ih = ch - S * 0.04, k = fmax(iw / pw, ih / ph);
        rounded(cr, -iw / 2, -ih / 2, iw, ih, S * 0.012);
        cairo_clip(cr);
        cairo_scale(cr, k, k);
        cairo_set_source_surface(cr, img, -pw / 2, -ph / 2);
        cairo_paint(cr);
        cairo_restore(cr);
        cairo_surface_destroy(img);
        g_object_unref(p);
    }
    /* Front of the folder, with the Hexa hexagon. */
    cairo_pattern_t *g = cairo_pattern_create_linear(0, S * 0.46, 0, S * 0.94);
    cairo_pattern_add_color_stop_rgba(g, 0, fmin(1, ar * 1.12), fmin(1, ag * 1.12), fmin(1, ab * 1.12), 0.96);
    cairo_pattern_add_color_stop_rgba(g, 1, ar * 0.86, ag * 0.86, ab * 0.86, 0.96);
    cairo_set_source(cr, g);
    rounded(cr, S * 0.04, S * 0.46, S * 0.92, S * 0.46, S * 0.07);
    cairo_fill(cr);
    cairo_pattern_destroy(g);
    cairo_set_source_rgba(cr, 1, 1, 1, 0.35);
    cairo_set_line_width(cr, S * 0.018);
    for (int i = 0; i < 6; i++) {
        double a = M_PI / 6 + i * M_PI / 3;
        cairo_line_to(cr, S * 0.5 + S * 0.09 * cos(a), S * 0.69 + S * 0.09 * sin(a));
    }
    cairo_close_path(cr);
    cairo_stroke(cr);
    cairo_destroy(cr);
    GdkPixbuf *out = pixbuf_from_surface(s);
    cairo_surface_destroy(s);
    if (save(out, png, NULL, NULL, NULL, key->str)) {
        result = png;
        png = NULL;
    }
    g_object_unref(out);
done:
    g_free(uri); g_free(md5); g_free(fname); g_free(bucket); g_free(png);
    g_string_free(key, TRUE);
out:
    for (guint i = 0; i < pics->len; i++)
        g_free(g_array_index(pics, Pic, i).path);
    g_array_free(pics, TRUE);
    return result;
}

int main(int argc, char **argv)
{
    int size = 128, folder = 0, first = 1;
    double ar = 0.35, ag = 0.55, ab = 0.95;
    const char *failed_path = NULL;
    for (; first < argc && strncmp(argv[first], "--", 2) == 0; first++) {
        if (strcmp(argv[first], "--size") == 0 && first + 1 < argc)
            size = atoi(argv[++first]);
        else if (strcmp(argv[first], "--accent") == 0 && first + 1 < argc) {
            unsigned v = (unsigned)strtoul(argv[++first], NULL, 16);
            ar = ((v >> 16) & 255) / 255.0; ag = ((v >> 8) & 255) / 255.0; ab = (v & 255) / 255.0;
        } else if (strcmp(argv[first], "--folder") == 0)
            folder = 1;
        else if (strcmp(argv[first], "--mark-failed") == 0 && first + 1 < argc)
            failed_path = argv[++first];
        else if (strcmp(argv[first], "--") == 0) {
            first++;
            break;
        } else {
            fprintf(stderr, "usage: nexa-thumb [--size N] [--accent RRGGBB] [--folder] PATH... | --mark-failed PATH\n");
            return 2;
        }
    }
    if (size < 16 || size > 512)
        size = 128;
    char *root = cache_root();
    char *buckets[] = { "normal", "large", "x-large", "fail/nexa-thumb", "nexa-folders" };
    for (size_t i = 0; i < G_N_ELEMENTS(buckets); i++) {
        char *d = g_build_filename(root, buckets[i], NULL);
        g_mkdir_with_parents(d, 0700);
        g_free(d);
    }
    if (failed_path) {
        struct stat st;
        char *uri = g_filename_to_uri(failed_path, NULL, NULL), mtime[32];
        if (uri && stat(failed_path, &st) == 0) {
            snprintf(mtime, sizeof mtime, "%lld", (long long)st.st_mtime);
            mark_failed(root, uri, mtime);
        }
        g_free(uri);
        return 0;
    }

    /* GStreamer reads (and the first time writes) its plugin registry. */
    char *gst_cache = g_build_filename(g_get_user_cache_dir(), "gstreamer-1.0", NULL);
    g_mkdir_with_parents(gst_cache, 0700);
    g_setenv("GST_REGISTRY_FORK", "no", TRUE);
    gst_init(NULL, NULL);

    /* Read: the folders the inputs are in. Write: the cache only. */
    int n_in = argc - first;
    char **reads = g_new0(char *, n_in + 1);
    for (int i = 0; i < n_in; i++)
        reads[i] = folder ? g_strdup(argv[first + i]) : g_path_get_dirname(argv[first + i]);
    /* glycin (gdk-pixbuf's decoders) passes data to its loader processes
     * through temporary files. */
    char *writes[] = { root, gst_cache, (char *)g_get_tmp_dir() };
    struct rlimit mem = { 2ULL << 30, 2ULL << 30 }, cpu = { 120, 120 };
    setrlimit(RLIMIT_AS, &mem);
    setrlimit(RLIMIT_CPU, &cpu);
    if (confine(reads, n_in, writes, 3) < 0) {
        fprintf(stderr, "nexa-thumb: couldn't confine myself: %s\n", strerror(errno));
        return 1;
    }

    for (int i = first; i < argc; i++) {
        printf("start\t%s\n", argv[i]);
        fflush(stdout);
        char *t = folder ? folder_thumbnail(root, argv[i], size, ar, ag, ab) : thumbnail(root, argv[i], size);
        if (t)
            printf("ok\t%s\t%s\n", argv[i], t);
        else
            printf("none\t%s\n", argv[i]);
        fflush(stdout);
        g_free(t);
    }
    return 0;
}
