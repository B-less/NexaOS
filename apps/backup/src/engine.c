#define _GNU_SOURCE
#include "engine.h"

#include <errno.h>
#include <fcntl.h>
#include <gio/gio.h>
#include <glib/gstdio.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <time.h>
#include <unistd.h>

#define BUF (256 * 1024)
#define MAX_DEPTH 64

static GQuark quark(void) { return g_quark_from_static_string("backup"); }

/* ---- what is left out ---- */

/* Home-relative paths never backed up (and everything under them). */
static const char *const LEFT_OUT[] = {
    ".cache", ".local/share/Trash", ".ssh", ".gnupg", ".local/share/keyrings", ".password-store", ".pki",
    ".mozilla", ".config/chromium", ".config/google-chrome", ".config/BraveSoftware", ".var/app",
    ".config/nexaos-keys", ".local/share/backup", ".config/backup", ".cache/backup",
    ".thumbnails", ".local/share/recently-used.xbel", ".xsession-errors", NULL,
};

gboolean always_left_out(const char *rel)
{
    if (!rel || !*rel) return FALSE;
    for (int i = 0; LEFT_OUT[i]; i++) {
        size_t n = strlen(LEFT_OUT[i]);
        if (!strncmp(rel, LEFT_OUT[i], n) && (rel[n] == 0 || rel[n] == '/')) return TRUE;
    }
    /* NexaOS's own private data (the rules: never read ~/.local/share/nexa-*) */
    if (g_str_has_prefix(rel, ".local/share/nexa-") || g_str_has_prefix(rel, ".local/state/nexa")) return TRUE;
    const char *base = strrchr(rel, '/');
    base = base ? base + 1 : rel;
    /* secret keys, wherever they are */
    if (g_str_has_suffix(base, ".key") || !strcmp(base, ".netrc") || !strcmp(base, ".git-credentials") ||
        g_str_has_suffix(base, ".kdbx"))
        return TRUE;
    return FALSE;
}

gboolean choice_includes(const Choice *c, const char *rel)
{
    if (!c || !c->off || !rel) return TRUE;
    const char *slash = strchr(rel, '/');
    char *top = slash ? g_strndup(rel, (gsize)(slash - rel)) : g_strdup(rel);
    gboolean on = !g_hash_table_contains(c->off, top);
    g_free(top);
    return on;
}

/* ---- small helpers ---- */

char *escape_field(const char *s)
{
    GString *o = g_string_new(NULL);
    for (const char *c = s ? s : ""; *c; c++) {
        switch (*c) {
        case '\\': g_string_append(o, "\\\\"); break;
        case '\t': g_string_append(o, "\\t"); break;
        case '\n': g_string_append(o, "\\n"); break;
        case '\r': g_string_append(o, "\\r"); break;
        default: g_string_append_c(o, *c);
        }
    }
    return g_string_free(o, FALSE);
}

char *unescape_field(const char *s)
{
    GString *o = g_string_new(NULL);
    for (const char *c = s ? s : ""; *c; c++) {
        if (*c == '\\' && c[1]) {
            c++;
            g_string_append_c(o, *c == 't' ? '\t' : *c == 'n' ? '\n' : *c == 'r' ? '\r' : *c);
        } else {
            g_string_append_c(o, *c);
        }
    }
    return g_string_free(o, FALSE);
}

char *restored_name(const char *path)
{
    const char *slash = strrchr(path, '/'), *dot = strrchr(path, '.');
    const char *base = slash ? slash + 1 : path;
    if (!dot || dot < base || dot == base) return g_strdup_printf("%s (restored)", path);
    return g_strdup_printf("%.*s (restored)%s", (int)(dot - path), path, dot);
}

/* A path from a backup we accept: relative, no "." or ".." parts. */
static gboolean rel_ok(const char *rel)
{
    if (!rel || !*rel || rel[0] == '/' || strlen(rel) > 4096) return FALSE;
    char **parts = g_strsplit(rel, "/", -1);
    gboolean ok = TRUE;
    for (int i = 0; parts[i] && ok; i++) ok = *parts[i] && strcmp(parts[i], ".") && strcmp(parts[i], "..");
    g_strfreev(parts);
    return ok;
}

static gboolean hex64(const char *h)
{
    if (!h || strlen(h) != 64) return FALSE;
    for (int i = 0; i < 64; i++)
        if (!g_ascii_isxdigit(h[i]) || g_ascii_isupper(h[i])) return FALSE;
    return TRUE;
}

static void item_free(Item *it)
{
    g_free(it->path);
    g_free(it->target);
    g_free(it);
}

char *backup_root(const char *drive)
{
    GString *u = g_string_new(NULL);
    for (const char *c = g_get_user_name(); *c; c++) g_string_append_c(u, g_ascii_isalnum(*c) || *c == '-' || *c == '_' ? *c : '_');
    char *r = g_build_filename(drive, "NexaOS Backup", u->len ? u->str : "user", NULL);
    g_string_free(u, TRUE);
    return r;
}

static char *object_path(const char *root, const char *hash)
{
    char dir[3] = {hash[0], hash[1], 0};
    return g_build_filename(root, "objects", dir, hash, NULL);
}

gboolean drive_space(const char *drive, guint64 *free_b, guint64 *total_b)
{
    struct statvfs v;
    if (statvfs(drive, &v) != 0) return FALSE;
    if (free_b) *free_b = (guint64)v.f_bavail * v.f_frsize;
    if (total_b) *total_b = (guint64)v.f_blocks * v.f_frsize;
    return TRUE;
}

/* ---- snapshots ---- */

void snap_free(Snap *s)
{
    if (!s) return;
    g_free(s->name);
    g_free(s->file);
    if (s->items) g_ptr_array_free(s->items, TRUE);
    g_free(s);
}

static gint newest_first(gconstpointer a, gconstpointer b)
{
    const Snap *x = *(Snap *const *)a, *y = *(Snap *const *)b;
    return x->when == y->when ? -g_strcmp0(x->name, y->name) : (x->when > y->when ? -1 : 1);
}

GPtrArray *snaps_list(const char *drive)
{
    GPtrArray *out = g_ptr_array_new_with_free_func((GDestroyNotify)snap_free);
    char *root = backup_root(drive), *dir = g_build_filename(root, "snapshots", NULL);
    GDir *d = g_dir_open(dir, 0, NULL);
    const char *name;
    while (d && (name = g_dir_read_name(d))) {
        if (!g_str_has_suffix(name, ".snap") || name[0] == '.') continue;
        char *path = g_build_filename(dir, name, NULL);
        FILE *f = fopen(path, "r");
        char line[256] = "";
        if (f && fgets(line, sizeof line, f)) {
            char **h = g_strsplit(g_strchomp(line), "\t", -1);
            if (g_strv_length(h) == 5 && !strcmp(h[0], SNAP_MAGIC)) {
                Snap *s = g_new0(Snap, 1);
                s->name = g_strndup(name, strlen(name) - 5);
                s->file = g_strdup(path);
                s->when = g_ascii_strtoll(h[1], NULL, 10);
                s->files = g_ascii_strtoll(h[2], NULL, 10);
                s->bytes = g_ascii_strtoll(h[3], NULL, 10);
                s->added = g_ascii_strtoll(h[4], NULL, 10);
                g_ptr_array_add(out, s);
            }
            g_strfreev(h);
        }
        if (f) fclose(f);
        g_free(path);
    }
    if (d) g_dir_close(d);
    g_ptr_array_sort(out, newest_first);
    g_free(dir);
    g_free(root);
    return out;
}

gboolean snap_load(Snap *s, GError **err)
{
    if (s->items) return TRUE;
    char *text = NULL;
    gsize len = 0;
    if (!g_file_get_contents(s->file, &text, &len, NULL)) {
        g_set_error(err, quark(), 1, "This backup can't be read from the drive.");
        return FALSE;
    }
    s->items = g_ptr_array_new_with_free_func((GDestroyNotify)item_free);
    char **lines = g_strsplit(text, "\n", -1);
    g_free(text);
    for (int i = 1; lines[i]; i++) {
        char **f = g_strsplit(lines[i], "\t", -1);
        if (g_strv_length(f) == 7 && strlen(f[0]) == 1 && strchr("FDL", f[0][0])) {
            Item *it = g_new0(Item, 1);
            it->kind = f[0][0];
            it->mode = (guint)g_ascii_strtoull(f[1], NULL, 8) & 07777;
            it->mtime = g_ascii_strtoll(f[2], NULL, 10);
            it->size = g_ascii_strtoll(f[3], NULL, 10);
            g_strlcpy(it->hash, f[4], sizeof it->hash);
            it->path = unescape_field(f[5]);
            it->target = it->kind == 'L' ? unescape_field(f[6]) : NULL;
            if (rel_ok(it->path) && (it->kind != 'F' || hex64(it->hash)) && it->size >= 0) g_ptr_array_add(s->items, it);
            else item_free(it);  /* a damaged or hostile line: left out */
        }
        g_strfreev(f);
    }
    g_strfreev(lines);
    return TRUE;
}

/* ---- walking home ---- */

typedef struct {
    char *rel;
    struct stat st;
    char *target;
} Entry;

static void entry_free(Entry *e)
{
    g_free(e->rel);
    g_free(e->target);
    g_free(e);
}

typedef struct {
    const char *home;
    const Choice *c;
    dev_t dev;
    GPtrArray *out;
    volatile gint *cancel;
} Walk;

static gint by_name(gconstpointer a, gconstpointer b) { return strcmp(*(char *const *)a, *(char *const *)b); }

static void walk(Walk *w, const char *rel, int depth)
{
    if (depth > MAX_DEPTH || (w->cancel && g_atomic_int_get(w->cancel))) return;
    char *dir = *rel ? g_build_filename(w->home, rel, NULL) : g_strdup(w->home);
    GDir *d = g_dir_open(dir, 0, NULL);
    const char *name;
    GPtrArray *names = g_ptr_array_new_with_free_func(g_free);
    while (d && (name = g_dir_read_name(d))) g_ptr_array_add(names, g_strdup(name));
    if (d) g_dir_close(d);
    g_ptr_array_sort(names, by_name);  /* the same order every time */
    for (guint i = 0; i < names->len; i++) {
        name = names->pdata[i];
        char *r = *rel ? g_build_filename(rel, name, NULL) : g_strdup(name);
        if (always_left_out(r) || (!*rel && !choice_includes(w->c, r))) {
            g_free(r);
            continue;
        }
        char *path = g_build_filename(w->home, r, NULL);
        struct stat st;
        if (lstat(path, &st) == 0 && st.st_dev == w->dev) {  /* (another drive mounted inside: not this backup's) */
            Entry *e = g_new0(Entry, 1);
            e->rel = r;
            e->st = st;
            r = NULL;
            if (S_ISLNK(st.st_mode)) {
                char buf[4096];
                ssize_t n = readlink(path, buf, sizeof buf - 1);
                if (n >= 0) {
                    buf[n] = 0;
                    e->target = g_strdup(buf);
                    g_ptr_array_add(w->out, e);
                } else {
                    entry_free(e);
                }
            } else if (S_ISREG(st.st_mode)) {
                g_ptr_array_add(w->out, e);
            } else if (S_ISDIR(st.st_mode)) {
                g_ptr_array_add(w->out, e);
                walk(w, e->rel, depth + 1);
            } else {
                entry_free(e);  /* sockets, pipes, devices */
            }
        }
        g_free(r);
        g_free(path);
    }
    g_ptr_array_free(names, TRUE);
    g_free(dir);
}

static GPtrArray *scan(const char *home, const Choice *c, volatile gint *cancel)
{
    struct stat hs;
    GPtrArray *out = g_ptr_array_new_with_free_func((GDestroyNotify)entry_free);
    if (stat(home, &hs) != 0) return out;
    Walk w = {home, c, hs.st_dev, out, cancel};
    walk(&w, "", 0);
    return out;
}

/* The newest backup's files: path -> Item, to skip what hasn't changed. */
static GHashTable *previous(const char *drive, Snap **keep)
{
    GHashTable *h = g_hash_table_new(g_str_hash, g_str_equal);
    GPtrArray *all = snaps_list(drive);
    *keep = NULL;
    if (all->len && snap_load(all->pdata[0], NULL)) {
        *keep = g_ptr_array_steal_index(all, 0);
        for (guint i = 0; i < (*keep)->items->len; i++) {
            Item *it = (*keep)->items->pdata[i];
            if (it->kind == 'F') g_hash_table_replace(h, it->path, it);
        }
    }
    g_ptr_array_free(all, TRUE);
    return h;
}

gint64 backup_estimate(const char *home, const char *drive, const Choice *c, gint64 *files, GError **err)
{
    Snap *prev = NULL;
    GHashTable *old = previous(drive, &prev);
    GPtrArray *e = scan(home, c, NULL);
    char *root = backup_root(drive);
    gint64 need = 0, n = 0;
    for (guint i = 0; i < e->len; i++) {
        Entry *x = e->pdata[i];
        if (!S_ISREG(x->st.st_mode)) continue;
        n++;
        Item *it = g_hash_table_lookup(old, x->rel);
        if (it && it->size == x->st.st_size && it->mtime == x->st.st_mtime) continue;
        need += x->st.st_size;
    }
    if (files) *files = n;
    (void)err;
    g_free(root);
    g_ptr_array_free(e, TRUE);
    g_hash_table_destroy(old);
    snap_free(prev);
    return need;
}

/* Copy one file into the objects, hashing as it goes: its hash. NULL and
 * *err if it can't be read (skipped) or written (the backup stops:
 * *fatal). */
static char *store_file(const char *root, const char *path, Job *job, gint64 *copied, gboolean *fatal, GError **err)
{
    int in = open(path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    if (in < 0) {
        g_set_error(err, quark(), 2, "%s", g_strerror(errno));
        return NULL;
    }
    char *tmpdir = g_build_filename(root, "objects", "tmp", NULL);
    g_mkdir_with_parents(tmpdir, 0700);
    char *tmp = g_build_filename(tmpdir, "partXXXXXX", NULL);
    int out = g_mkstemp_full(tmp, O_WRONLY | O_CLOEXEC, 0600);
    g_free(tmpdir);
    if (out < 0) {
        *fatal = TRUE;
        g_set_error(err, quark(), 3, "Backup can't write to the drive (%s).", g_strerror(errno));
        close(in);
        g_free(tmp);
        return NULL;
    }
    GChecksum *sum = g_checksum_new(G_CHECKSUM_SHA256);
    char *buf = g_malloc(BUF);
    ssize_t n;
    gboolean ok = TRUE;
    gint64 total = 0;
    while ((n = read(in, buf, BUF)) > 0) {
        if (job && job->cancel && g_atomic_int_get(job->cancel)) {
            ok = FALSE;
            *fatal = TRUE;
            g_set_error(err, quark(), 4, "Stopped.");
            break;
        }
        g_checksum_update(sum, (const guchar *)buf, n);
        for (ssize_t off = 0; off < n;) {
            ssize_t w = write(out, buf + off, (size_t)(n - off));
            if (w < 0) {
                if (errno == EINTR) continue;
                ok = FALSE;
                *fatal = TRUE;
                g_set_error(err, quark(), errno == ENOSPC ? 5 : 6,
                            errno == ENOSPC ? "The drive is full." : "Backup can't write to the drive (%s).", g_strerror(errno));
                break;
            }
            off += w;
        }
        if (!ok) break;
        total += n;
    }
    if (ok && n < 0) {
        ok = FALSE;
        g_set_error(err, quark(), 2, "%s", g_strerror(errno));  /* a read error: skipped */
    }
    if (ok && fsync(out) != 0) {
        ok = FALSE;
        *fatal = TRUE;
        g_set_error(err, quark(), errno == ENOSPC ? 5 : 6, errno == ENOSPC ? "The drive is full." : "The drive stopped answering.");
    }
    close(out);
    close(in);
    g_free(buf);
    char *hash = NULL;
    if (ok) {
        hash = g_strdup(g_checksum_get_string(sum));
        char *obj = object_path(root, hash);
        if (g_file_test(obj, G_FILE_TEST_EXISTS)) {
            g_unlink(tmp);  /* the same contents are there already */
        } else {
            char *dir = g_path_get_dirname(obj);
            g_mkdir_with_parents(dir, 0700);
            g_free(dir);
            if (g_rename(tmp, obj) != 0) {
                *fatal = TRUE;
                g_set_error(err, quark(), 6, "Backup can't write to the drive (%s).", g_strerror(errno));
                g_clear_pointer(&hash, g_free);
                g_unlink(tmp);
            } else {
                *copied += total;
            }
        }
        g_free(obj);
    } else {
        g_unlink(tmp);
    }
    g_checksum_free(sum);
    g_free(tmp);
    return hash;
}

void result_clear(Result *r)
{
    if (r->problems) g_ptr_array_free(r->problems, TRUE);
    g_free(r->snap_name);
    memset(r, 0, sizeof *r);
}

static void sync_dir(const char *dir)
{
    int fd = open(dir, O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (fd >= 0) {
        fsync(fd);
        close(fd);
    }
}

gboolean backup_run(const char *home, const char *drive, const Choice *c, gint64 now, Job *job, Result *out, GError **err)
{
    memset(out, 0, sizeof *out);
    out->problems = g_ptr_array_new_with_free_func(g_free);
    char *root = backup_root(drive), *snapdir = g_build_filename(root, "snapshots", NULL);
    if (g_mkdir_with_parents(snapdir, 0700) != 0) {
        g_set_error(err, quark(), 6, "Backup can't write to the drive (%s). Is it read-only?", g_strerror(errno));
        g_free(root);
        g_free(snapdir);
        return FALSE;
    }
    Snap *prev = NULL;
    GHashTable *old = previous(drive, &prev);
    GPtrArray *entries = scan(home, c, job ? job->cancel : NULL);
    gint64 total_files = 0, total_bytes = 0, done_files = 0, done_bytes = 0;
    for (guint i = 0; i < entries->len; i++) {
        Entry *e = entries->pdata[i];
        if (S_ISREG(e->st.st_mode)) {
            total_files++;
            total_bytes += e->st.st_size;
        }
    }
    GString *body = g_string_new(NULL);
    gboolean ok = TRUE;
    for (guint i = 0; i < entries->len && ok; i++) {
        Entry *e = entries->pdata[i];
        char *rel = escape_field(e->rel);
        guint mode = e->st.st_mode & 07777;
        if (S_ISDIR(e->st.st_mode)) {
            g_string_append_printf(body, "D\t%o\t%" G_GINT64_FORMAT "\t0\t\t%s\t\n", mode, (gint64)e->st.st_mtime, rel);
        } else if (S_ISLNK(e->st.st_mode)) {
            char *t = escape_field(e->target);
            g_string_append_printf(body, "L\t%o\t%" G_GINT64_FORMAT "\t0\t\t%s\t%s\n", mode, (gint64)e->st.st_mtime, rel, t);
            g_free(t);
        } else {
            Item *was = g_hash_table_lookup(old, e->rel);
            char *hash = NULL, *obj = NULL;
            if (was && was->size == e->st.st_size && was->mtime == e->st.st_mtime) {
                obj = object_path(root, was->hash);
                if (g_file_test(obj, G_FILE_TEST_IS_REGULAR)) hash = g_strdup(was->hash);  /* unchanged: not read again */
                g_free(obj);
            }
            if (!hash) {
                char *path = g_build_filename(home, e->rel, NULL);
                GError *fe = NULL;
                gboolean fatal = FALSE;
                gint64 before = out->copied_bytes;
                hash = store_file(root, path, job, &out->copied_bytes, &fatal, &fe);
                if (hash && out->copied_bytes > before) out->copied++;
                if (!hash && fatal) {
                    ok = FALSE;
                    g_propagate_error(err, fe);
                    fe = NULL;
                } else if (!hash) {
                    g_ptr_array_add(out->problems, g_strdup_printf("%s: %s", e->rel, fe ? fe->message : "can't be read"));
                    out->skipped++;
                }
                g_clear_error(&fe);
                g_free(path);
            }
            if (hash) {
                g_string_append_printf(body, "F\t%o\t%" G_GINT64_FORMAT "\t%" G_GINT64_FORMAT "\t%s\t%s\t\n", mode,
                                       (gint64)e->st.st_mtime, (gint64)e->st.st_size, hash, rel);
                out->files++;
                out->bytes += e->st.st_size;
            }
            g_free(hash);
            done_files++;
            done_bytes += e->st.st_size;
            if (job && job->progress) job->progress(job->data, done_files, total_files, done_bytes, total_bytes, e->rel);
        }
        g_free(rel);
        if (job && job->cancel && g_atomic_int_get(job->cancel) && ok) {
            ok = FALSE;
            g_set_error(err, quark(), 4, "Stopped. The backups made before are all still there.");
        }
    }
    if (ok) {
        /* The list goes in last, whole: a backup cut short is never listed. */
        time_t t = (time_t)now;
        struct tm tm;
        localtime_r(&t, &tm);
        char name[64];
        strftime(name, sizeof name, "%Y-%m-%d_%H%M%S", &tm);
        char *base = g_strdup(name), *final = NULL;
        for (int k = 2; k < 100; k++) {
            g_free(final);
            char *file = g_strconcat(base, ".snap", NULL);
            final = g_build_filename(snapdir, file, NULL);
            g_free(file);
            if (!g_file_test(final, G_FILE_TEST_EXISTS)) break;
            g_free(base);
            base = g_strdup_printf("%s-%d", name, k);
        }
        char *head = g_strdup_printf(SNAP_MAGIC "\t%" G_GINT64_FORMAT "\t%" G_GINT64_FORMAT "\t%" G_GINT64_FORMAT "\t%" G_GINT64_FORMAT "\n",
                                     now, out->files, out->bytes, out->copied_bytes);
        g_string_prepend(body, head);
        g_free(head);
        char *tmp = g_build_filename(snapdir, ".writing", NULL);
        FILE *f = fopen(tmp, "w");
        gboolean wrote = f && fwrite(body->str, 1, body->len, f) == body->len && fflush(f) == 0 && fsync(fileno(f)) == 0;
        if (f) fclose(f);
        if (wrote && g_rename(tmp, final) == 0) {
            sync_dir(snapdir);
            out->snap_name = g_strdup(base);
        } else {
            g_unlink(tmp);
            ok = FALSE;
            g_set_error(err, quark(), 5, errno == ENOSPC ? "The drive is full." : "Backup couldn't write the list of files to the drive.");
        }
        g_free(tmp);
        g_free(final);
        g_free(base);
    }
    g_string_free(body, TRUE);
    g_ptr_array_free(entries, TRUE);
    g_hash_table_destroy(old);
    snap_free(prev);
    g_free(snapdir);
    g_free(root);
    if (ok) {
        /* Old backups thinned out, and what nothing needs any more goes. */
        GPtrArray *all = snaps_list(drive), *gone = prune_plan(all, now);
        for (guint i = 0; i < gone->len; i++) {
            char *root2 = backup_root(drive), *file = g_strconcat((char *)gone->pdata[i], ".snap", NULL);
            char *path = g_build_filename(root2, "snapshots", file, NULL);
            g_unlink(path);
            g_free(path);
            g_free(file);
            g_free(root2);
        }
        if (gone->len) collect_garbage(drive, NULL);
        g_ptr_array_free(gone, TRUE);
        g_ptr_array_free(all, TRUE);
    }
    return ok;
}

/* ---- putting back ---- */

static gboolean wanted(const char *rel, const char *const *paths)
{
    if (!paths || !paths[0]) return TRUE;
    for (int i = 0; paths[i]; i++) {
        size_t n = strlen(paths[i]);
        if (!n || (!strncmp(rel, paths[i], n) && (rel[n] == 0 || rel[n] == '/'))) return TRUE;
    }
    return FALSE;
}

/* Object -> file, checking the hash on the way. */
static gboolean copy_out(const char *obj, const char *dest, const Item *it, GError **err)
{
    int in = open(obj, O_RDONLY | O_CLOEXEC);
    if (in < 0) {
        g_set_error(err, quark(), 7, "its copy is missing from the drive");
        return FALSE;
    }
    char *dir = g_path_get_dirname(dest), *tmp = g_build_filename(dir, ".restoringXXXXXX", NULL);
    g_mkdir_with_parents(dir, 0700);
    int out = g_mkstemp_full(tmp, O_WRONLY | O_CLOEXEC, 0600);
    gboolean ok = out >= 0;
    if (!ok) g_set_error(err, quark(), 8, "%s", g_strerror(errno));
    GChecksum *sum = g_checksum_new(G_CHECKSUM_SHA256);
    char *buf = g_malloc(BUF);
    ssize_t n;
    while (ok && (n = read(in, buf, BUF)) > 0) {
        g_checksum_update(sum, (const guchar *)buf, n);
        if (write(out, buf, (size_t)n) != n) {
            ok = FALSE;
            g_set_error(err, quark(), 8, errno == ENOSPC ? "the disk is full" : "%s", g_strerror(errno));
        }
    }
    if (ok && strcmp(g_checksum_get_string(sum), it->hash)) {
        ok = FALSE;
        g_set_error(err, quark(), 9, "its copy on the drive is damaged");
    }
    if (out >= 0) {
        fchmod(out, it->mode ? it->mode & 0777 : 0644);
        if (ok && fsync(out) != 0) ok = FALSE;
        close(out);
    }
    close(in);
    if (ok) {
        struct timespec ts[2] = {{.tv_sec = it->mtime}, {.tv_sec = it->mtime}};
        utimensat(AT_FDCWD, tmp, ts, 0);
        ok = g_rename(tmp, dest) == 0;
        if (!ok) g_set_error(err, quark(), 8, "%s", g_strerror(errno));
    }
    if (!ok) g_unlink(tmp);
    g_free(buf);
    g_checksum_free(sum);
    g_free(tmp);
    g_free(dir);
    return ok;
}

gboolean restore_run(const char *drive, Snap *s, const char *const *paths, const char *home, const char *dest, Job *job,
                     gint64 *restored, GPtrArray *problems, GError **err)
{
    if (!snap_load(s, err)) return FALSE;
    char *root = backup_root(drive);
    gint64 total = 0, done = 0, bytes = 0, done_bytes = 0;
    *restored = 0;
    for (guint i = 0; i < s->items->len; i++) {
        Item *it = s->items->pdata[i];
        if (it->kind == 'F' && wanted(it->path, paths)) total++, bytes += it->size;
    }
    const char *base = dest ? dest : home;
    for (guint i = 0; i < s->items->len; i++) {
        Item *it = s->items->pdata[i];
        if (!wanted(it->path, paths)) continue;
        if (job && job->cancel && g_atomic_int_get(job->cancel)) {
            g_set_error(err, quark(), 4, "Stopped.");
            g_free(root);
            return FALSE;
        }
        char *target = g_build_filename(base, it->path, NULL);
        if (it->kind == 'D') {
            g_mkdir_with_parents(target, it->mode ? (int)(it->mode & 0777) : 0755);
        } else if (it->kind == 'L') {
            if (!g_file_test(target, G_FILE_TEST_EXISTS) && !g_file_test(target, G_FILE_TEST_IS_SYMLINK)) {
                char *dir = g_path_get_dirname(target);
                g_mkdir_with_parents(dir, 0755);
                g_free(dir);
                if (symlink(it->target, target) == 0) (*restored)++;
            }
        } else {
            struct stat st;
            char *to = g_strdup(target);
            gboolean there = lstat(target, &st) == 0;
            if (there && S_ISREG(st.st_mode) && st.st_size == it->size && st.st_mtime == it->mtime) {
                g_free(to);
                to = NULL;  /* already there, as it was */
                (*restored)++;
            } else if (there) {
                g_free(to);
                to = restored_name(target);  /* the one there now is kept */
            }
            if (to) {
                char *obj = object_path(root, it->hash);
                GError *e = NULL;
                if (copy_out(obj, to, it, &e)) (*restored)++;
                else if (problems) g_ptr_array_add(problems, g_strdup_printf("%s: %s", it->path, e ? e->message : "failed"));
                g_clear_error(&e);
                g_free(obj);
                g_free(to);
            }
            done++;
            done_bytes += it->size;
            if (job && job->progress) job->progress(job->data, done, total, done_bytes, bytes, it->path);
        }
        g_free(target);
    }
    g_free(root);
    return TRUE;
}

/* ---- checking and tidying ---- */

gint64 check_snap(const char *drive, Snap *s, Job *job, GError **err)
{
    if (!snap_load(s, err)) return -1;
    char *root = backup_root(drive);
    GHashTable *seen = g_hash_table_new(g_str_hash, g_str_equal);
    gint64 bad = 0, done = 0, total = 0;
    for (guint i = 0; i < s->items->len; i++)
        if (((Item *)s->items->pdata[i])->kind == 'F') total++;
    char *buf = g_malloc(BUF);
    for (guint i = 0; i < s->items->len; i++) {
        Item *it = s->items->pdata[i];
        if (it->kind != 'F') continue;
        done++;
        if (!g_hash_table_contains(seen, it->hash)) {
            g_hash_table_add(seen, it->hash);
            char *obj = object_path(root, it->hash);
            int fd = open(obj, O_RDONLY | O_CLOEXEC);
            GChecksum *sum = g_checksum_new(G_CHECKSUM_SHA256);
            ssize_t n = 0;
            if (fd >= 0) {
                while ((n = read(fd, buf, BUF)) > 0) g_checksum_update(sum, (const guchar *)buf, n);
                close(fd);
            }
            if (fd < 0 || n < 0 || strcmp(g_checksum_get_string(sum), it->hash)) bad++;
            g_checksum_free(sum);
            g_free(obj);
        }
        if (job && job->progress) job->progress(job->data, done, total, 0, 0, it->path);
        if (job && job->cancel && g_atomic_int_get(job->cancel)) break;
    }
    g_free(buf);
    g_hash_table_destroy(seen);
    g_free(root);
    return bad;
}

GPtrArray *prune_plan(GPtrArray *snaps, gint64 now)
{
    GPtrArray *gone = g_ptr_array_new_with_free_func(g_free);
    GHashTable *kept = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    for (guint i = 0; i < snaps->len; i++) {
        Snap *s = snaps->pdata[i];
        gint64 age = now - s->when;
        char *key;
        if (i == 0 || age < 2 * 86400) continue;  /* the newest, and the last two days: all of them */
        time_t t = (time_t)s->when;
        struct tm tm;
        localtime_r(&t, &tm);
        if (age < 14 * 86400) key = g_strdup_printf("d%d-%d", tm.tm_year, tm.tm_yday);
        else if (age < 56 * 86400) key = g_strdup_printf("w%" G_GINT64_FORMAT, s->when / (7 * 86400));
        else if (age < 730 * 86400) key = g_strdup_printf("m%d-%d", tm.tm_year, tm.tm_mon);
        else key = NULL;
        if (!key || g_hash_table_contains(kept, key)) g_ptr_array_add(gone, g_strdup(s->name));
        else g_hash_table_add(kept, key), key = NULL;
        g_free(key);
    }
    g_hash_table_destroy(kept);
    return gone;
}

gint64 collect_garbage(const char *drive, GError **err)
{
    GPtrArray *all = snaps_list(drive);
    GHashTable *need = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    for (guint i = 0; i < all->len; i++) {
        Snap *s = all->pdata[i];
        if (!snap_load(s, NULL)) {
            /* a list that can't be read: delete nothing, to be safe */
            g_set_error(err, quark(), 10, "A backup on the drive can't be read, so nothing was tidied.");
            g_hash_table_destroy(need);
            g_ptr_array_free(all, TRUE);
            return 0;
        }
        for (guint j = 0; j < s->items->len; j++) {
            Item *it = s->items->pdata[j];
            if (it->kind == 'F') g_hash_table_add(need, g_strdup(it->hash));
        }
    }
    char *root = backup_root(drive), *objects = g_build_filename(root, "objects", NULL);
    gint64 n = 0;
    GDir *d = g_dir_open(objects, 0, NULL);
    const char *sub;
    while (d && (sub = g_dir_read_name(d))) {
        gboolean tmp = !strcmp(sub, "tmp");
        if (!tmp && !(strlen(sub) == 2 && g_ascii_isxdigit(sub[0]) && g_ascii_isxdigit(sub[1]))) continue;  /* not ours */
        char *dir = g_build_filename(objects, sub, NULL);
        GDir *e = g_dir_open(dir, 0, NULL);
        const char *name;
        while (e && (name = g_dir_read_name(e))) {
            if (tmp || !g_hash_table_contains(need, name)) {
                char *p = g_build_filename(dir, name, NULL);
                if (g_unlink(p) == 0) n++;
                g_free(p);
            }
        }
        if (e) g_dir_close(e);
        g_rmdir(dir);  /* only if empty */
        g_free(dir);
    }
    if (d) g_dir_close(d);
    g_free(objects);
    g_free(root);
    g_hash_table_destroy(need);
    g_ptr_array_free(all, TRUE);
    return n;
}

gboolean snap_delete(const char *drive, const char *name, GError **err)
{
    if (!name || strchr(name, '/') || name[0] == '.') {
        g_set_error(err, quark(), 11, "That isn't a backup.");
        return FALSE;
    }
    char *root = backup_root(drive), *file = g_strconcat(name, ".snap", NULL);
    char *path = g_build_filename(root, "snapshots", file, NULL);
    gboolean ok = g_unlink(path) == 0;
    if (!ok) g_set_error(err, quark(), 11, "That backup couldn't be deleted (%s).", g_strerror(errno));
    g_free(path);
    g_free(file);
    g_free(root);
    if (ok) collect_garbage(drive, NULL);
    return ok;
}
