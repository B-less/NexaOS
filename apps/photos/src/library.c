#include "library.h"

#include <glib/gstdio.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>

#define INDEX_MAGIC "photos-index 1"

gboolean library_wanted_name(const char *name)
{
    static const char *ext[] = {".jpg", ".jpeg", ".jpe", ".png", ".webp", ".gif", ".tif", ".tiff", ".bmp", NULL};
    if (!name || name[0] == '.') return FALSE;
    char *lower = g_ascii_strdown(name, -1);
    gboolean yes = FALSE;
    for (int i = 0; ext[i] && !yes; i++) yes = g_str_has_suffix(lower, ext[i]);
    g_free(lower);
    return yes;
}

static gint64 wall_time(gint64 unix_time)
{
    GDateTime *d = g_date_time_new_from_unix_local(unix_time);
    if (!d) return unix_time;
    gint64 off = g_date_time_get_utc_offset(d) / G_USEC_PER_SEC;
    g_date_time_unref(d);
    return unix_time + off;
}

static void set_taken(Photo *p)
{
    long long t = exif_time(p->info.taken);
    p->exif_date = t >= 0;
    p->taken = t >= 0 ? t : wall_time(p->mtime);
}

static char *rel_of(const char *path, const char *root)
{
    if (!root) return NULL;
    size_t n = strlen(root);
    while (n > 1 && root[n - 1] == '/') n--;
    if (strncmp(path, root, n) || path[n] != '/' || !path[n + 1]) return NULL;
    return g_strdup(path + n + 1);
}

Photo *photo_read(const char *path, const char *root)
{
    struct stat st;
    if (stat(path, &st) != 0 || !S_ISREG(st.st_mode) || st.st_size < 8) return NULL;
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    /* Most of what we want is near the start: 128 KB, then up to 1 MB for a
     * JPEG whose size comes after big colour profiles. */
    size_t cap = 128 * 1024, n = 0;
    unsigned char *buf = g_malloc(cap);
    n = fread(buf, 1, cap, f);
    Photo *p = NULL;
    PhotoInfo info;
    int ok = photo_info_parse(buf, n, &info) == 0;
    if (ok && n == cap && !info.width && photo_kind(buf, n) == KIND_JPEG) {
        size_t more = 1024 * 1024;
        buf = g_realloc(buf, more);
        n += fread(buf + n, 1, more - n, f);
        ok = photo_info_parse(buf, n, &info) == 0;
    }
    fclose(f);
    g_free(buf);
    if (!ok) return NULL;
    p = g_new0(Photo, 1);
    p->path = g_strdup(path);
    p->rel = rel_of(path, root);
    p->name = g_path_get_basename(path);
    p->mtime = st.st_mtime;
    p->size = st.st_size;
    p->info = info;
    set_taken(p);
    return p;
}

void photo_free(Photo *p)
{
    if (!p) return;
    g_free(p->path);
    g_free(p->rel);
    g_free(p->name);
    g_free(p);
}

/* ---- the index ---- */

static void put_str(GString *s, const char *v)
{
    char *e = g_strescape(v ? v : "", NULL);
    g_string_append(s, e);
    g_string_append_c(s, '\t');
    g_free(e);
}

static void index_save(const char *index, GPtrArray *photos)
{
    GString *s = g_string_new(INDEX_MAGIC "\n");
    for (guint i = 0; i < photos->len; i++) {
        const Photo *p = photos->pdata[i];
        if (!p->rel) continue;
        const PhotoInfo *x = &p->info;
        put_str(s, p->rel);
        g_string_append_printf(s, "%" G_GINT64_FORMAT "\t%" G_GINT64_FORMAT "\t%d\t%d\t%d\t", p->mtime, p->size, x->width,
                               x->height, x->orientation);
        put_str(s, x->taken);
        put_str(s, x->make);
        put_str(s, x->model);
        put_str(s, x->lens);
        put_str(s, x->exposure);
        put_str(s, x->fnumber);
        put_str(s, x->focal);
        g_string_append_printf(s, "%d\t%d\t%.7f\t%.7f\n", x->iso, x->has_gps, x->lat, x->lon);
    }
    char *dir = g_path_get_dirname(index);
    g_mkdir_with_parents(dir, 0700);
    g_free(dir);
    g_file_set_contents(index, s->str, (gssize)s->len, NULL);  /* a cache: failing is fine */
    g_string_free(s, TRUE);
}

static void get_str(char *dst, size_t cap, const char *field)
{
    char *v = g_strcompress(field);
    g_strlcpy(dst, v, cap);
    g_free(v);
}

/* rel -> Photo (path filled in later) */
static GHashTable *index_load(const char *index)
{
    GHashTable *h = g_hash_table_new_full(g_str_hash, g_str_equal, NULL, (GDestroyNotify)photo_free);
    char *text = NULL;
    gsize len = 0;
    if (!index || !g_file_get_contents(index, &text, &len, NULL)) return h;
    char **lines = g_strsplit(text, "\n", -1);
    g_free(text);
    if (!lines[0] || strcmp(lines[0], INDEX_MAGIC)) {
        g_strfreev(lines);
        return h;
    }
    for (int i = 1; lines[i]; i++) {
        char **f = g_strsplit(lines[i], "\t", -1);
        if (g_strv_length(f) == 17) {
            Photo *p = g_new0(Photo, 1);
            p->rel = g_strcompress(f[0]);
            p->mtime = g_ascii_strtoll(f[1], NULL, 10);
            p->size = g_ascii_strtoll(f[2], NULL, 10);
            PhotoInfo *x = &p->info;
            x->width = (int)g_ascii_strtoll(f[3], NULL, 10);
            x->height = (int)g_ascii_strtoll(f[4], NULL, 10);
            x->orientation = (int)g_ascii_strtoll(f[5], NULL, 10);
            if (x->orientation < 1 || x->orientation > 8) x->orientation = 1;
            get_str(x->taken, sizeof x->taken, f[6]);
            get_str(x->make, sizeof x->make, f[7]);
            get_str(x->model, sizeof x->model, f[8]);
            get_str(x->lens, sizeof x->lens, f[9]);
            get_str(x->exposure, sizeof x->exposure, f[10]);
            get_str(x->fnumber, sizeof x->fnumber, f[11]);
            get_str(x->focal, sizeof x->focal, f[12]);
            x->iso = (int)g_ascii_strtoll(f[13], NULL, 10);
            x->has_gps = f[14][0] == '1';
            x->lat = g_ascii_strtod(f[15], NULL);
            x->lon = g_ascii_strtod(f[16], NULL);
            if (*p->rel && !strstr(p->rel, "..") && p->rel[0] != '/') g_hash_table_replace(h, p->rel, p);
            else photo_free(p);
        }
        g_strfreev(f);
    }
    g_strfreev(lines);
    return h;
}

/* ---- scanning ---- */

typedef struct {
    const char *root;
    GHashTable *known;
    GPtrArray *out;
    volatile gint *cancel;
    gboolean changed;
} Scan;

static void walk(Scan *s, const char *dir, int depth)
{
    if (depth > LIB_MAX_DEPTH || (s->cancel && g_atomic_int_get(s->cancel))) return;
    GDir *d = g_dir_open(dir, 0, NULL);
    if (!d) return;
    const char *name;
    while ((name = g_dir_read_name(d)) && s->out->len < LIB_MAX_PHOTOS) {
        if (name[0] == '.') continue;
        char *path = g_build_filename(dir, name, NULL);
        struct stat st;
        if (lstat(path, &st) == 0) {
            if (S_ISDIR(st.st_mode)) {
                walk(s, path, depth + 1);  /* (a linked folder isn't followed: no loops) */
            } else if ((S_ISREG(st.st_mode) || S_ISLNK(st.st_mode)) && library_wanted_name(name)) {
                char *rel = rel_of(path, s->root);
                Photo *old = rel ? g_hash_table_lookup(s->known, rel) : NULL;
                struct stat fs;
                if (old && stat(path, &fs) == 0 && fs.st_mtime == old->mtime && fs.st_size == old->size) {
                    g_hash_table_steal(s->known, rel);
                    old->path = g_strdup(path);
                    old->name = g_strdup(name);
                    set_taken(old);
                    g_ptr_array_add(s->out, old);
                } else {
                    Photo *p = photo_read(path, s->root);
                    if (p) g_ptr_array_add(s->out, p);
                    s->changed = TRUE;
                }
                g_free(rel);
            }
        }
        g_free(path);
    }
    g_dir_close(d);
}

static gint newest_first(gconstpointer a, gconstpointer b)
{
    const Photo *p = *(Photo *const *)a, *q = *(Photo *const *)b;
    if (p->taken != q->taken) return p->taken > q->taken ? -1 : 1;
    return g_strcmp0(p->path, q->path);
}

GPtrArray *library_scan(const char *root, const char *index, volatile gint *cancel)
{
    Scan s = {root, index_load(index), g_ptr_array_new_with_free_func((GDestroyNotify)photo_free), cancel, FALSE};
    walk(&s, root, 0);
    if (g_hash_table_size(s.known)) s.changed = TRUE;  /* some went away */
    g_hash_table_destroy(s.known);
    g_ptr_array_sort(s.out, newest_first);
    if (index && s.changed && !(cancel && g_atomic_int_get(cancel))) index_save(index, s.out);
    return s.out;
}

/* ---- dates and words ---- */

static const char *MONTHS[12] = {"January", "February", "March",     "April",   "May",      "June",
                                 "July",    "August",   "September", "October", "November", "December"};
static const char *DAYS[7] = {"Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"};

int photo_month(gint64 taken)
{
    time_t t = (time_t)taken;
    struct tm tm;
    if (!gmtime_r(&t, &tm)) return 0;
    return (tm.tm_year + 1900) * 12 + tm.tm_mon;
}

char *month_title(int month)
{
    return g_strdup_printf("%s %d", MONTHS[((month % 12) + 12) % 12], month / 12);
}

char *photo_when(const Photo *p)
{
    time_t t = (time_t)p->taken;
    struct tm tm;
    if (!gmtime_r(&t, &tm)) return g_strdup("");
    return g_strdup_printf("%s %d %s %d, %02d:%02d", DAYS[tm.tm_wday], tm.tm_mday, MONTHS[tm.tm_mon], tm.tm_year + 1900,
                           tm.tm_hour, tm.tm_min);
}

gboolean photo_matches(const Photo *p, const char *query, const char *albums)
{
    if (!query || !*query) return TRUE;
    char *month = month_title(photo_month(p->taken));
    char *hay = g_strjoin(" ", p->name ? p->name : "", p->rel ? p->rel : "", month, p->info.make, p->info.model,
                          albums ? albums : "", NULL);
    char *h = g_utf8_casefold(hay, -1), *q = g_utf8_casefold(query, -1);
    char **words = g_strsplit_set(q, " \t", -1);
    gboolean all = TRUE;
    for (int i = 0; words[i] && all; i++)
        if (*words[i] && !strstr(h, words[i])) all = FALSE;
    g_strfreev(words);
    g_free(q);
    g_free(h);
    g_free(hay);
    g_free(month);
    return all;
}
