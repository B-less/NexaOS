/* Unit tests for Photos' engine parts: EXIF and picture sizes (with
 * damaged and random input), dates, orientation, the library scan and its
 * index, albums and Deleted, and decoding with libjpeg (make test). */
#include <glib/gstdio.h>
#include <jpeglib.h>
#include <math.h>
#include <stdio.h>
#include <string.h>
#include <utime.h>

#include "../src/exif.h"
#include "../src/library.h"
#include "../src/store.h"
#include "../src/thumbs.h"

static int fails, checks;

static void check(int ok, const char *what)
{
    checks++;
    if (!ok) fails++;
    printf("%s %s\n", ok ? "PASS" : "FAIL", what);
}

/* ---- building test files ---- */

typedef struct {
    GByteArray *b;
    int le;
} Buf;

static void u16(Buf *t, unsigned v)
{
    guint8 x[2] = {t->le ? v & 255 : v >> 8, t->le ? v >> 8 : v & 255};
    g_byte_array_append(t->b, x, 2);
}

static void u32(Buf *t, unsigned long v)
{
    guint8 x[4];
    for (int i = 0; i < 4; i++) x[t->le ? i : 3 - i] = (v >> (8 * i)) & 255;
    g_byte_array_append(t->b, x, 4);
}

static void entry(Buf *t, unsigned tag, unsigned kind, unsigned long count, unsigned long val)
{
    u16(t, tag);
    u16(t, kind);
    u32(t, count);
    if (kind == 3 && count == 1) {
        u16(t, (unsigned)val);
        u16(t, 0);
    } else {
        u32(t, val);
    }
}

/* A TIFF block: IFD0 (make, model, orientation, EXIF, GPS), a sub-IFD
 * (exposure 1/250, f/2.8, ISO 200, 35 mm, date) and a GPS IFD. */
static GByteArray *tiff(int le, int orientation)
{
    Buf t = {g_byte_array_new(), le};
    g_byte_array_append(t.b, (const guint8 *)(le ? "II*\0" : "MM\0*"), 4);
    u32(&t, 8);
    unsigned ifd0 = 8, n0 = 5, data0 = ifd0 + 2 + n0 * 12 + 4;
    const char make[] = "NexaCam", model[] = "Hex 1";
    unsigned sub = data0 + sizeof make + sizeof model, nsub = 5, rats = sub + 2 + nsub * 12 + 4;
    const char date[] = "2026:10:31 18:04:05";
    unsigned gps = rats + 24 + sizeof date, ngps = 4, gdata = gps + 2 + ngps * 12 + 4;
    u16(&t, n0);
    entry(&t, 0x010f, 2, sizeof make, data0);
    entry(&t, 0x0110, 2, sizeof model, data0 + sizeof make);
    entry(&t, 0x0112, 3, 1, (unsigned long)orientation);
    entry(&t, 0x8769, 4, 1, sub);
    entry(&t, 0x8825, 4, 1, gps);
    u32(&t, 0);
    g_byte_array_append(t.b, (const guint8 *)make, sizeof make);
    g_byte_array_append(t.b, (const guint8 *)model, sizeof model);
    u16(&t, nsub);
    entry(&t, 0x829a, 5, 1, rats);
    entry(&t, 0x829d, 5, 1, rats + 8);
    entry(&t, 0x8827, 3, 1, 200);
    entry(&t, 0x920a, 5, 1, rats + 16);
    entry(&t, 0x9003, 2, sizeof date, rats + 24);
    u32(&t, 0);
    unsigned long r[] = {1, 250, 28, 10, 35, 1};
    for (int i = 0; i < 6; i++) u32(&t, r[i]);
    g_byte_array_append(t.b, (const guint8 *)date, sizeof date);
    u16(&t, ngps);
    entry(&t, 1, 2, 2, 'N');      /* "N\0" fits in the entry */
    entry(&t, 2, 5, 3, gdata);
    entry(&t, 3, 2, 2, 'W');
    entry(&t, 4, 5, 3, gdata + 24);
    u32(&t, 0);
    unsigned long lat[] = {5, 1, 36, 1, 1332, 100}, lon[] = {0, 1, 11, 1, 1320, 100};
    for (int i = 0; i < 6; i++) u32(&t, lat[i]);
    for (int i = 0; i < 6; i++) u32(&t, lon[i]);
    return t.b;
}

/* FFD8, APP1 Exif, SOF0 (w x h), EOI */
static GByteArray *jpeg_header(GByteArray *exif, int w, int h)
{
    GByteArray *j = g_byte_array_new();
    g_byte_array_append(j, (const guint8 *)"\xff\xd8", 2);
    if (exif) {
        unsigned len = exif->len + 8;
        guint8 m[4] = {0xff, 0xe1, len >> 8, len & 255};
        g_byte_array_append(j, m, 4);
        g_byte_array_append(j, (const guint8 *)"Exif\0\0", 6);
        g_byte_array_append(j, exif->data, exif->len);
    }
    guint8 sof[] = {0xff, 0xc0, 0, 17, 8, h >> 8, h & 255, w >> 8, w & 255, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1};
    g_byte_array_append(j, sof, sizeof sof);
    g_byte_array_append(j, (const guint8 *)"\xff\xd9", 2);
    return j;
}

/* A real JPEG: left half red, right half blue, with EXIF orientation o. */
static void write_jpeg(const char *path, int w, int h, int o)
{
    FILE *f = fopen(path, "wb");
    struct jpeg_compress_struct c;
    struct jpeg_error_mgr e;
    c.err = jpeg_std_error(&e);
    jpeg_create_compress(&c);
    jpeg_stdio_dest(&c, f);
    c.image_width = (JDIMENSION)w;
    c.image_height = (JDIMENSION)h;
    c.input_components = 3;
    c.in_color_space = JCS_RGB;
    jpeg_set_defaults(&c);
    jpeg_set_quality(&c, 92, TRUE);
    jpeg_start_compress(&c, TRUE);
    if (o) {
        GByteArray *t = tiff(1, o);
        GByteArray *m = g_byte_array_new();
        g_byte_array_append(m, (const guint8 *)"Exif\0\0", 6);
        g_byte_array_append(m, t->data, t->len);
        jpeg_write_marker(&c, JPEG_APP0 + 1, m->data, m->len);
        g_byte_array_free(m, TRUE);
        g_byte_array_free(t, TRUE);
    }
    guint8 *row = g_malloc((size_t)w * 3);
    for (int x = 0; x < w; x++) {
        int red = x < w / 2;
        row[x * 3] = red ? 230 : 20;
        row[x * 3 + 1] = 20;
        row[x * 3 + 2] = red ? 20 : 230;
    }
    while (c.next_scanline < c.image_height) jpeg_write_scanlines(&c, &row, 1);
    jpeg_finish_compress(&c);
    jpeg_destroy_compress(&c);
    fclose(f);
    g_free(row);
}

static int is_red(cairo_surface_t *s, int x, int y)
{
    cairo_surface_flush(s);
    guint32 p = *(guint32 *)(void *)(cairo_image_surface_get_data(s) + (size_t)y * cairo_image_surface_get_stride(s) + (size_t)x * 4);
    return ((p >> 16) & 255) > 150 && (p & 255) < 100;
}

static int is_blue(cairo_surface_t *s, int x, int y)
{
    cairo_surface_flush(s);
    guint32 p = *(guint32 *)(void *)(cairo_image_surface_get_data(s) + (size_t)y * cairo_image_surface_get_stride(s) + (size_t)x * 4);
    return (p & 255) > 150 && ((p >> 16) & 255) < 100;
}

static void write_file(const char *path, const void *data, size_t n)
{
    char *dir = g_path_get_dirname(path);
    g_mkdir_with_parents(dir, 0700);
    g_free(dir);
    g_file_set_contents(path, data, (gssize)n, NULL);
}

/* ---- tests ---- */

static void test_exif(void)
{
    GByteArray *t = tiff(1, 6), *j = jpeg_header(t, 4000, 3000);
    PhotoInfo x;
    check(photo_info_parse(j->data, j->len, &x) == 0, "a JPEG is read");
    check(x.width == 4000 && x.height == 3000, "its size (from the frame header)");
    check(x.orientation == 6, "its orientation");
    check(!strcmp(x.make, "NexaCam") && !strcmp(x.model, "Hex 1"), "the camera");
    check(!strcmp(x.taken, "2026:10:31 18:04:05"), "when it was taken");
    check(!strcmp(x.exposure, "1/250 s") && !strcmp(x.fnumber, "f/2.8") && x.iso == 200 && !strcmp(x.focal, "35 mm"),
          "exposure, aperture, ISO, focal length");
    check(x.has_gps && fabs(x.lat - 5.6037) < 0.001 && fabs(x.lon + 0.187) < 0.001, "where (5.6037 N, 0.1870 W)");
    GByteArray *tb = tiff(0, 8);
    PhotoInfo y;
    check(exif_parse_tiff(tb->data, tb->len, &y) == 0 && y.orientation == 8 && !strcmp(y.model, "Hex 1"), "big-endian EXIF too");

    /* Every cut-short copy: no crash, never anything outside the bytes. */
    int ok = 1;
    for (guint n = 0; n <= j->len; n++) {
        guint8 *copy = g_memdup2(j->data, n);
        PhotoInfo z;
        photo_info_parse(copy ? copy : (guint8 *)"", n, &z);
        ok &= z.orientation >= 1 && z.orientation <= 8 && strlen(z.taken) < sizeof z.taken;
        g_free(copy);
    }
    check(ok, "every cut-short JPEG read safely");
    /* Random damage, 20000 times. */
    guint32 seed = 12345;
    ok = 1;
    for (int round = 0; round < 20000; round++) {
        guint8 *copy = g_memdup2(j->data, j->len);
        for (int k = 0; k < 1 + round % 8; k++) {
            seed = seed * 1103515245 + 12345;
            copy[(seed >> 8) % j->len] = (guint8)(seed >> 20);
        }
        PhotoInfo z;
        photo_info_parse(copy, j->len, &z);
        ok &= z.orientation >= 1 && z.orientation <= 8 && z.width >= 0 && z.height >= 0;
        g_free(copy);
    }
    check(ok, "20000 damaged copies read safely");
    /* An IFD pointing at itself and far outside the block. */
    guint8 loop[] = {'I', 'I', '*', 0, 8, 0, 0, 0, 2, 0, 0x69, 0x87, 4, 0, 1, 0, 0, 0, 8, 0, 0, 0,
                     0x0f, 0x01, 2, 0, 0xff, 0xff, 0xff, 0x7f, 0xf0, 0xff, 0xff, 0xff, 0, 0, 0, 0};
    PhotoInfo z;
    check(exif_parse_tiff(loop, sizeof loop, &z) == 0 && !z.make[0], "a looping, overlong EXIF is ignored");
    check(photo_info_parse((const guint8 *)"hello", 5, &z) != 0, "not a picture: refused");
    g_byte_array_free(t, TRUE);
    g_byte_array_free(j, TRUE);
    g_byte_array_free(tb, TRUE);

    guint8 png[] = {0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n', 0, 0, 0, 13, 'I', 'H', 'D', 'R', 0, 0, 7, 0x80, 0, 0, 4, 0x38, 8, 2, 0, 0, 0, 0, 0, 0, 0};
    check(photo_info_parse(png, sizeof png, &z) == 0 && z.width == 1920 && z.height == 1080, "PNG size");
    guint8 gif[] = {'G', 'I', 'F', '8', '9', 'a', 0x40, 1, 0xf0, 0, 0, 0, 0};
    check(photo_info_parse(gif, sizeof gif, &z) == 0 && z.width == 320 && z.height == 240, "GIF size");
    guint8 webp[] = {'R', 'I', 'F', 'F', 30, 0, 0, 0, 'W', 'E', 'B', 'P', 'V', 'P', '8', 'X', 10, 0, 0, 0,
                     0, 0, 0, 0, 0x7f, 2, 0, 0xdf, 1, 0};
    check(photo_info_parse(webp, sizeof webp, &z) == 0 && z.width == 640 && z.height == 480, "WebP size");
    guint8 vp8l[] = {'R', 'I', 'F', 'F', 30, 0, 0, 0, 'W', 'E', 'B', 'P', 'V', 'P', '8', 'L', 5, 0, 0, 0,
                     0x2f, 99, 0x80, 0, 0, 0};  /* width 100, height 3 */
    check(photo_info_parse(vp8l, sizeof vp8l, &z) == 0 && z.width == 100 && z.height == 3, "lossless WebP size");
}

static void test_dates(void)
{
    check(exif_time("1970:01:01 00:00:00") == 0 && exif_time("2026:10:31 18:04:05") == 1793469845LL, "EXIF dates");
    check(exif_time("") < 0 && exif_time("2026:13:01 00:00:00") < 0 && exif_time("0000:00:00 00:00:00") < 0 &&
              exif_time("2026:10:31 18:04:05x") < 0 && exif_time(NULL) < 0,
          "bad dates refused");
    int m = photo_month(1793469845LL);
    char *t = month_title(m);
    check(!strcmp(t, "October 2026"), "month title");
    g_free(t);
    Photo p = {.taken = 1793469845LL, .name = "beach.jpg", .rel = "Holiday/beach.jpg"};
    strcpy(p.info.model, "Hex 1");
    char *w = photo_when(&p);
    check(!strcmp(w, "Saturday 31 October 2026, 18:04"), "the date for people");
    g_free(w);
    check(photo_matches(&p, "holiday october", NULL) && photo_matches(&p, "HEX 2026", NULL) &&
              photo_matches(&p, "trip", "Trip Family") && !photo_matches(&p, "beach november", NULL) && photo_matches(&p, "", NULL),
          "search: every word, any field, albums too");
}

static void test_orientation(void)
{
    check(orient_turn(1, 1) == 6 && orient_turn(6, 1) == 3 && orient_turn(3, 1) == 8 && orient_turn(8, 1) == 1, "turning right");
    check(orient_turn(1, -1) == 8 && orient_turn(1, 2) == 3 && orient_turn(2, 1) == 7 && orient_turn(4, 4) == 4, "left, twice, mirrored");
    int sw, sh;
    orient_size(6, 400, 300, &sw, &sh);
    check(sw == 300 && sh == 400, "a turned picture's size");
    /* stored: [red blue] (2 x 1) */
    cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_RGB24, 2, 1);
    cairo_t *cr = cairo_create(s);
    cairo_set_source_rgb(cr, 1, 0, 0);
    cairo_rectangle(cr, 0, 0, 1, 1);
    cairo_fill(cr);
    cairo_set_source_rgb(cr, 0, 0, 1);
    cairo_rectangle(cr, 1, 0, 1, 1);
    cairo_fill(cr);
    cairo_destroy(cr);
    struct { int o, w, red_x, red_y; } want[] = {{1, 2, 0, 0}, {2, 2, 1, 0}, {3, 2, 1, 0}, {4, 2, 0, 0},
                                                  {5, 1, 0, 0}, {6, 1, 0, 0}, {7, 1, 0, 1}, {8, 1, 0, 1}};
    int ok = 1;
    for (int i = 0; i < 8; i++) {
        cairo_surface_t *o = orient_surface(s, want[i].o);
        ok &= cairo_image_surface_get_width(o) == want[i].w && is_red(o, want[i].red_x, want[i].red_y);
        cairo_surface_destroy(o);
    }
    check(ok, "all eight orientations drawn the right way up");
    cairo_surface_destroy(s);
}

static char *tmp_dir(void)
{
    char *d = g_dir_make_tmp("photos-test-XXXXXX", NULL);
    return d;
}

static void test_library(void)
{
    char *root = tmp_dir(), *cache = tmp_dir();
    GByteArray *t = tiff(1, 1), *j = jpeg_header(t, 640, 480), *plain = jpeg_header(NULL, 800, 600);
    char *a = g_build_filename(root, "a.jpg", NULL), *b = g_build_filename(root, "Holiday", "b b's \"x\".jpg", NULL);
    char *hidden = g_build_filename(root, ".hidden", "c.jpg", NULL), *txt = g_build_filename(root, "notes.txt", NULL);
    char *fake = g_build_filename(root, "fake.jpg", NULL), *empty = g_build_filename(root, "empty.png", NULL);
    write_file(a, j->data, j->len);
    write_file(b, plain->data, plain->len);
    write_file(hidden, j->data, j->len);
    write_file(txt, "hi", 2);
    write_file(fake, "this is not a jpeg at all", 25);
    write_file(empty, "", 0);
    struct utimbuf old = {1600000000, 1600000000};  /* b: no EXIF date, a file from 2020 */
    utime(b, &old);
    char *index = g_build_filename(cache, "index", NULL);
    GPtrArray *ph = library_scan(root, index, NULL);
    check(ph->len == 2, "the scan finds the pictures (not hidden, text, fake or empty ones)");
    const Photo *first = ph->len ? ph->pdata[0] : NULL;
    check(first && first->exif_date && !strcmp(first->rel, "a.jpg") && first->info.width == 640, "with EXIF dates first: newest first");
    check(ph->len > 1 && !strcmp(((Photo *)ph->pdata[1])->rel, "Holiday/b b's \"x\".jpg"), "folders inside, odd names too");
    check(g_file_test(index, G_FILE_TEST_EXISTS), "the index is kept");
    g_ptr_array_free(ph, TRUE);
    /* The index is used: change a's bytes but not its time or size, and the
     * old size still comes back. */
    GByteArray *j2 = jpeg_header(t, 641, 480);
    struct stat st;
    g_stat(a, &st);
    FILE *f = fopen(a, "r+b");
    fwrite(j2->data, 1, j2->len, f);
    fclose(f);
    struct utimbuf same = {st.st_atime, st.st_mtime};
    utime(a, &same);
    GPtrArray *again = library_scan(root, index, NULL);
    gboolean from_index = FALSE;
    for (guint i = 0; i < again->len; i++) {
        Photo *p = again->pdata[i];
        if (!strcmp(p->rel, "a.jpg")) from_index = p->info.width == 640;
    }
    check(again->len == 2 && from_index, "a second scan reads the index");
    g_ptr_array_free(again, TRUE);
    gint cancel = 1;
    GPtrArray *none = library_scan(root, NULL, &cancel);
    check(none->len == 0, "a scan can be stopped");
    g_ptr_array_free(none, TRUE);
    g_byte_array_free(t, TRUE);
    g_byte_array_free(j, TRUE);
    g_byte_array_free(j2, TRUE);
    g_byte_array_free(plain, TRUE);
    g_free(a); g_free(b); g_free(hidden); g_free(txt); g_free(fake); g_free(empty); g_free(index);
    g_free(root); g_free(cache);
}

static void test_store(void)
{
    char *data = tmp_dir(), *root = tmp_dir();
    Store *s = store_load(data);
    check(s->albums->len == 0 && !store_is_fav(s, "a.jpg"), "a new store is empty");
    store_set_fav(s, "a.jpg", TRUE);
    store_set_fav(s, "../etc/passwd", TRUE);
    store_set_turn(s, "a.jpg", 5);
    check(store_is_fav(s, "a.jpg") && !store_is_fav(s, "../etc/passwd") && store_turn(s, "a.jpg") == 1, "favourites and turns");
    check(store_name_problem(s, "  ", NULL) && store_name_problem(s, "favourites", NULL) &&
              store_name_problem(s, "a\nb", NULL) &&
              store_name_problem(s, "0123456789012345678901234567890123456789012345678901234567890", NULL),
          "bad album names refused, in words");
    Album *trip = store_album_new(s, " Trip ");
    check(!strcmp(trip->name, "Trip") && store_name_problem(s, "trip", NULL) && !store_name_problem(s, "trip", trip),
          "an album; the same name again is refused (except renaming itself)");
    album_add(trip, "a.jpg");
    album_add(trip, "a.jpg");
    album_add(trip, "/abs.jpg");
    album_add(trip, "Holiday/b.jpg");
    check(trip->items->len == 2, "photos go in once each");
    char *of = store_albums_of(s, "a.jpg");
    check(!strcmp(of, "Favourites Trip"), "which albums a photo is in");
    g_free(of);
    store_album_new(s, "Ünïcode [x] = y; z");
    check(store_save(s, NULL), "saved");
    store_free(s);
    s = store_load(data);
    Album *t2 = store_album_find(s, "trip");
    check(s->albums->len == 2 && t2 && t2->items->len == 2 && store_is_fav(s, "a.jpg") && store_turn(s, "a.jpg") == 1 &&
              store_album_find(s, "Ünïcode [x] = y; z"),
          "and read back, odd names too");

    /* Deleted */
    char *pa = g_build_filename(root, "a.jpg", NULL);
    write_file(pa, "picture", 7);
    GError *err = NULL;
    Gone *g = store_delete(s, root, "a.jpg", 1000, &err);
    check(g && !g_file_test(pa, G_FILE_TEST_EXISTS) && s->gone->len == 1, "delete: moved out of Pictures");
    char *gp = g ? store_gone_path(s, g) : NULL;
    check(gp && g_file_test(gp, G_FILE_TEST_EXISTS), "into Photos' Deleted folder");
    g_free(gp);
    write_file(pa, "another", 7);  /* a new a.jpg took its place */
    char *back = g ? store_restore(s, g, root, &err) : NULL;
    check(back && !strcmp(back, "a (2).jpg") && s->gone->len == 0, "put back, beside the new one");
    check(store_is_fav(s, "a (2).jpg") && !store_is_fav(s, "a.jpg") && album_has(store_album_find(s, "Trip"), "a (2).jpg"),
          "its favourite and albums follow it");
    g_free(back);
    g = store_delete(s, root, "a (2).jpg", 1000, &err);
    check(g && store_purge(s, 1000 + 29 * 86400) == 0 && s->gone->len == 1, "kept for 30 days");
    check(store_purge(s, 1000 + 31 * 86400) == 1 && s->gone->len == 0 && !store_is_fav(s, "a (2).jpg") &&
              !album_has(store_album_find(s, "Trip"), "a (2).jpg"),
          "then gone for good, out of its albums too");
    check(!store_delete(s, root, "../outside.jpg", 1, &err), "nothing outside Pictures is deleted");
    g_clear_error(&err);
    check(!store_delete(s, root, "missing.jpg", 1, &err) && err, "a missing photo: an error, not a crash");
    g_clear_error(&err);
    store_free(s);
    /* A damaged file: no crash, nothing bad read. */
    char *ini = g_build_filename(data, "library.ini", NULL);
    const char *hostile = "[album 1]\nname=\nitems=../x\n[deleted 1]\nfile=../../etc/passwd\nrel=a\nwhen=5\n"
                          "[favourites]\nitems=/etc;ok.jpg\n";
    write_file(ini, hostile, strlen(hostile));
    s = store_load(data);
    check(s->albums->len == 0 && s->gone->len == 0 && store_is_fav(s, "ok.jpg") && !store_is_fav(s, "/etc"),
          "a damaged or hostile library.ini is read safely");
    store_free(s);
    g_free(ini);
    g_free(pa);
    g_free(data);
    g_free(root);
}

static void test_decode(void)
{
    char *root = tmp_dir(), *cache = tmp_dir();
    char *path = g_build_filename(root, "turned.jpg", NULL);
    write_jpeg(path, 640, 480, 6);
    Photo *p = photo_read(path, root);
    check(p && p->info.orientation == 6 && p->info.width == 640, "a real JPEG read");
    cairo_surface_t *fit = p ? picture_load(p, p->info.orientation, 100, 100, 0, NULL) : NULL;
    check(fit && cairo_image_surface_get_width(fit) == 75 && cairo_image_surface_get_height(fit) == 100,
          "shrunk to fit, turned upright (75 x 100)");
    check(fit && is_red(fit, 37, 10) && is_blue(fit, 37, 90), "the right way up (red on top)");
    if (fit) cairo_surface_destroy(fit);
    gint64 t0 = g_get_monotonic_time();
    cairo_surface_t *th = p ? thumb_square(p, p->info.orientation, cache) : NULL;
    gint64 t1 = g_get_monotonic_time();
    char *tp = p ? thumb_path(p, p->info.orientation, cache) : NULL;
    check(th && cairo_image_surface_get_width(th) == THUMB_PX && tp && g_file_test(tp, G_FILE_TEST_EXISTS), "a thumbnail, kept");
    cairo_surface_t *th2 = p ? thumb_square(p, p->info.orientation, cache) : NULL;
    check(th2 && is_red(th2, THUMB_PX / 2, 20) && is_blue(th2, THUMB_PX / 2, THUMB_PX - 20), "read back from the cache");
    printf("INFO thumbnail of a 640x480 JPEG: %.1f ms\n", (t1 - t0) / 1000.0);
    if (th) cairo_surface_destroy(th);
    if (th2) cairo_surface_destroy(th2);
    cairo_surface_t *turned = p ? picture_load(p, orient_turn(p->info.orientation, 1), 0, 0, 0, NULL) : NULL;
    check(turned && cairo_image_surface_get_width(turned) == 640 && is_blue(turned, 10, 240) && is_red(turned, 630, 240),
          "turned once more in Photos (blue on the left)");
    if (turned) cairo_surface_destroy(turned);
    /* Half a JPEG: shown (the rest grey) or refused, never a crash. */
    gchar *bytes;
    gsize n;
    g_file_get_contents(path, &bytes, &n, NULL);
    char *half = g_build_filename(root, "half.jpg", NULL);
    write_file(half, bytes, n / 2);
    Photo *hp = photo_read(half, root);
    char *why = NULL;
    cairo_surface_t *hs = hp ? picture_load(hp, 1, 0, 0, 0, &why) : NULL;
    check(hp && (hs || why), "a cut-short JPEG: shown or explained");
    if (hs) cairo_surface_destroy(hs);
    g_free(why);
    /* Garbage after a JPEG header. */
    memset(bytes + 200, 0x5a, n - 200);
    char *junk = g_build_filename(root, "junk.jpg", NULL);
    write_file(junk, bytes, n);
    Photo *jp = photo_read(junk, root);
    why = NULL;
    cairo_surface_t *js = jp ? picture_load(jp, 1, 0, 0, 0, &why) : NULL;
    check(!jp || js || why, "a damaged JPEG: shown or explained");
    if (js) cairo_surface_destroy(js);
    g_free(why);
    photo_free(jp);
    photo_free(hp);
    photo_free(p);
    g_free(bytes);
    g_free(junk);
    g_free(half);
    g_free(tp);
    g_free(path);
    g_free(root);
    g_free(cache);
}

int main(void)
{
    test_exif();
    test_dates();
    test_orientation();
    test_library();
    test_store();
    test_decode();
    printf("%d of %d checks passed\n", checks - fails, checks);
    return fails != 0;
}
