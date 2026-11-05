#include "store.h"

#include <glib/gstdio.h>
#include <string.h>

static void album_free(Album *a)
{
    g_free(a->name);
    g_ptr_array_free(a->items, TRUE);
    g_free(a);
}

static void gone_free(Gone *g)
{
    g_free(g->file);
    g_free(g->rel);
    g_free(g);
}

/* A rel path we accept from the file: inside the library, no "..". */
static gboolean rel_ok(const char *rel)
{
    if (!rel || !*rel || rel[0] == '/' || strlen(rel) > 4096) return FALSE;
    char **parts = g_strsplit(rel, "/", -1);
    gboolean ok = TRUE;
    for (int i = 0; parts[i]; i++)
        if (!*parts[i] || !strcmp(parts[i], "..") || !strcmp(parts[i], ".")) ok = FALSE;
    g_strfreev(parts);
    return ok;
}

static gboolean file_name_ok(const char *f)
{
    return f && *f && !strchr(f, '/') && strcmp(f, ".") && strcmp(f, "..");
}

Store *store_load(const char *data_dir)
{
    Store *s = g_new0(Store, 1);
    s->path = g_build_filename(data_dir, "library.ini", NULL);
    s->deleted_dir = g_build_filename(data_dir, "deleted", NULL);
    s->fav = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    s->turns = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    s->albums = g_ptr_array_new_with_free_func((GDestroyNotify)album_free);
    s->gone = g_ptr_array_new_with_free_func((GDestroyNotify)gone_free);
    GKeyFile *k = g_key_file_new();
    if (!g_key_file_load_from_file(k, s->path, G_KEY_FILE_NONE, NULL)) {
        g_key_file_free(k);
        return s;
    }
    char **favs = g_key_file_get_string_list(k, "favourites", "items", NULL, NULL);
    for (int i = 0; favs && favs[i]; i++)
        if (rel_ok(favs[i])) g_hash_table_add(s->fav, g_strdup(favs[i]));
    g_strfreev(favs);
    char **turns = g_key_file_get_string_list(k, "turns", "items", NULL, NULL);
    for (int i = 0; turns && turns[i]; i++) {
        int q = turns[i][0] - '0';
        if (q >= 1 && q <= 3 && turns[i][1] == ':' && rel_ok(turns[i] + 2))
            g_hash_table_replace(s->turns, g_strdup(turns[i] + 2), GINT_TO_POINTER(q));
    }
    g_strfreev(turns);
    gsize ng = 0;
    char **groups = g_key_file_get_groups(k, &ng);
    for (gsize i = 0; i < ng; i++) {
        if (g_str_has_prefix(groups[i], "album ")) {
            char *name = g_key_file_get_string(k, groups[i], "name", NULL);
            if (name && !store_name_problem(s, name, NULL)) {
                Album *a = store_album_new(s, name);
                char **items = g_key_file_get_string_list(k, groups[i], "items", NULL, NULL);
                for (int j = 0; items && items[j]; j++)
                    if (rel_ok(items[j])) album_add(a, items[j]);
                g_strfreev(items);
            }
            g_free(name);
        } else if (g_str_has_prefix(groups[i], "deleted ")) {
            char *file = g_key_file_get_string(k, groups[i], "file", NULL);
            char *rel = g_key_file_get_string(k, groups[i], "rel", NULL);
            gint64 when = g_key_file_get_int64(k, groups[i], "when", NULL);
            if (file_name_ok(file) && rel_ok(rel) && when > 0) {
                Gone *g = g_new0(Gone, 1);
                g->file = file;
                g->rel = rel;
                g->when = when;
                g_ptr_array_add(s->gone, g);
            } else {
                g_free(file);
                g_free(rel);
            }
        }
    }
    g_strfreev(groups);
    g_key_file_free(k);
    return s;
}

static gint by_str(gconstpointer a, gconstpointer b) { return strcmp(*(char *const *)a, *(char *const *)b); }

/* Sorted, so the file is the same for the same contents. */
static char **keys_of(GHashTable *h)
{
    GPtrArray *a = g_ptr_array_new();
    GHashTableIter it;
    gpointer key;
    g_hash_table_iter_init(&it, h);
    while (g_hash_table_iter_next(&it, &key, NULL)) g_ptr_array_add(a, key);
    g_ptr_array_sort(a, by_str);
    g_ptr_array_add(a, NULL);
    return (char **)g_ptr_array_free(a, FALSE);
}

gboolean store_save(Store *s, GError **err)
{
    GKeyFile *k = g_key_file_new();
    char **favs = keys_of(s->fav);
    gsize nf = g_strv_length(favs);
    if (nf) g_key_file_set_string_list(k, "favourites", "items", (const char *const *)favs, nf);
    g_free(favs);
    GPtrArray *t = g_ptr_array_new_with_free_func(g_free);
    GHashTableIter it;
    gpointer key, val;
    g_hash_table_iter_init(&it, s->turns);
    while (g_hash_table_iter_next(&it, &key, &val)) g_ptr_array_add(t, g_strdup_printf("%d:%s", GPOINTER_TO_INT(val), (char *)key));
    g_ptr_array_sort(t, by_str);
    if (t->len) g_key_file_set_string_list(k, "turns", "items", (const char *const *)t->pdata, t->len);
    g_ptr_array_free(t, TRUE);
    for (guint i = 0; i < s->albums->len; i++) {
        Album *a = s->albums->pdata[i];
        char group[32];
        g_snprintf(group, sizeof group, "album %u", i + 1);
        g_key_file_set_string(k, group, "name", a->name);
        g_key_file_set_string_list(k, group, "items", (const char *const *)a->items->pdata, a->items->len);
    }
    for (guint i = 0; i < s->gone->len; i++) {
        Gone *g = s->gone->pdata[i];
        char group[32];
        g_snprintf(group, sizeof group, "deleted %u", i + 1);
        g_key_file_set_string(k, group, "file", g->file);
        g_key_file_set_string(k, group, "rel", g->rel);
        g_key_file_set_int64(k, group, "when", g->when);
    }
    char *dir = g_path_get_dirname(s->path);
    g_mkdir_with_parents(dir, 0700);
    g_free(dir);
    gboolean ok = g_key_file_save_to_file(k, s->path, err);
    g_key_file_free(k);
    return ok;
}

void store_free(Store *s)
{
    if (!s) return;
    g_free(s->path);
    g_free(s->deleted_dir);
    g_hash_table_destroy(s->fav);
    g_hash_table_destroy(s->turns);
    g_ptr_array_free(s->albums, TRUE);
    g_ptr_array_free(s->gone, TRUE);
    g_free(s);
}

gboolean store_is_fav(Store *s, const char *rel) { return rel && g_hash_table_contains(s->fav, rel); }

void store_set_fav(Store *s, const char *rel, gboolean on)
{
    if (!rel_ok(rel)) return;
    if (on) g_hash_table_add(s->fav, g_strdup(rel));
    else g_hash_table_remove(s->fav, rel);
}

int store_turn(Store *s, const char *rel) { return rel ? GPOINTER_TO_INT(g_hash_table_lookup(s->turns, rel)) : 0; }

void store_set_turn(Store *s, const char *rel, int q)
{
    if (!rel_ok(rel)) return;
    q %= 4;
    if (q < 0) q += 4;
    if (q) g_hash_table_replace(s->turns, g_strdup(rel), GINT_TO_POINTER(q));
    else g_hash_table_remove(s->turns, rel);
}

/* ---- albums ---- */

const char *store_name_problem(Store *s, const char *name, const Album *except)
{
    if (!name) return "Type a name for the album.";
    char *t = g_strstrip(g_strdup(name));
    const char *why = NULL;
    if (!*t) why = "Type a name for the album.";
    else if (!g_utf8_validate(t, -1, NULL)) why = "That name has characters Photos can't use.";
    else if (g_utf8_strlen(t, -1) > ALBUM_NAME_MAX) why = "That name is too long: 60 characters at most.";
    else {
        for (const char *c = t; *c && !why; c++)
            if ((unsigned char)*c < 32 || *c == 127) why = "That name has characters Photos can't use.";
        if (!why && !g_ascii_strcasecmp(t, "Favourites")) why = "Favourites is already there.";
        Album *same = why ? NULL : store_album_find(s, t);
        if (same && same != except) why = "There's already an album with that name.";
    }
    g_free(t);
    return why;
}

Album *store_album_new(Store *s, const char *name)
{
    Album *a = g_new0(Album, 1);
    a->name = g_strstrip(g_strdup(name));
    a->items = g_ptr_array_new_with_free_func(g_free);
    g_ptr_array_add(s->albums, a);
    return a;
}

Album *store_album_find(Store *s, const char *name)
{
    char *t = g_strstrip(g_strdup(name ? name : ""));
    Album *found = NULL;
    for (guint i = 0; i < s->albums->len && !found; i++) {
        Album *a = s->albums->pdata[i];
        char *x = g_utf8_casefold(a->name, -1), *y = g_utf8_casefold(t, -1);
        if (!strcmp(x, y)) found = a;
        g_free(x);
        g_free(y);
    }
    g_free(t);
    return found;
}

void store_album_delete(Store *s, Album *a) { g_ptr_array_remove(s->albums, a); }

gboolean album_has(const Album *a, const char *rel)
{
    for (guint i = 0; rel && i < a->items->len; i++)
        if (!strcmp(a->items->pdata[i], rel)) return TRUE;
    return FALSE;
}

void album_add(Album *a, const char *rel)
{
    if (rel_ok(rel) && !album_has(a, rel)) g_ptr_array_add(a->items, g_strdup(rel));
}

void album_remove(Album *a, const char *rel)
{
    for (guint i = 0; rel && i < a->items->len; i++)
        if (!strcmp(a->items->pdata[i], rel)) {
            g_ptr_array_remove_index(a->items, i);
            return;
        }
}

char *store_albums_of(Store *s, const char *rel)
{
    GString *out = g_string_new(store_is_fav(s, rel) ? "Favourites" : "");
    for (guint i = 0; i < s->albums->len; i++) {
        Album *a = s->albums->pdata[i];
        if (album_has(a, rel)) {
            if (out->len) g_string_append_c(out, ' ');
            g_string_append(out, a->name);
        }
    }
    return g_string_free(out, FALSE);
}

/* ---- Deleted ---- */

char *store_gone_path(Store *s, const Gone *g) { return g_build_filename(s->deleted_dir, g->file, NULL); }

static gboolean move(const char *from, const char *to, GError **err)
{
    GFile *a = g_file_new_for_path(from), *b = g_file_new_for_path(to);
    gboolean ok = g_file_move(a, b, G_FILE_COPY_NOFOLLOW_SYMLINKS, NULL, NULL, NULL, err);
    g_object_unref(a);
    g_object_unref(b);
    return ok;
}

Gone *store_delete(Store *s, const char *root, const char *rel, gint64 now, GError **err)
{
    if (!rel_ok(rel)) {
        g_set_error_literal(err, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "That photo isn't in your Pictures folder.");
        return NULL;
    }
    g_mkdir_with_parents(s->deleted_dir, 0700);
    char *base = g_path_get_basename(rel), *file = NULL, *dst = NULL;
    for (int n = 0; n < 1000; n++) {
        g_free(file);
        g_free(dst);
        file = g_strdup_printf("%" G_GINT64_FORMAT "-%d-%s", now, n, base);
        dst = g_build_filename(s->deleted_dir, file, NULL);
        if (!g_file_test(dst, G_FILE_TEST_EXISTS)) break;
    }
    char *src = g_build_filename(root, rel, NULL);
    Gone *g = NULL;
    if (move(src, dst, err)) {
        g = g_new0(Gone, 1);
        g->file = file;
        g->rel = g_strdup(rel);
        g->when = now;
        g_ptr_array_add(s->gone, g);
        file = NULL;
    }
    g_free(src);
    g_free(dst);
    g_free(file);
    g_free(base);
    return g;
}

/* "a/b.jpg" -> "a/b (2).jpg" */
static char *numbered(const char *rel, int n)
{
    const char *slash = strrchr(rel, '/'), *dot = strrchr(rel, '.');
    if (!dot || (slash && dot < slash) || dot == (slash ? slash + 1 : rel)) return g_strdup_printf("%s (%d)", rel, n);
    return g_strdup_printf("%.*s (%d)%s", (int)(dot - rel), rel, n, dot);
}

static void rename_refs(Store *s, const char *from, const char *to)
{
    if (!strcmp(from, to)) return;
    if (g_hash_table_remove(s->fav, from)) g_hash_table_add(s->fav, g_strdup(to));
    gpointer q;
    if (g_hash_table_lookup_extended(s->turns, from, NULL, &q)) {
        g_hash_table_remove(s->turns, from);
        g_hash_table_replace(s->turns, g_strdup(to), q);
    }
    for (guint i = 0; i < s->albums->len; i++) {
        Album *a = s->albums->pdata[i];
        for (guint j = 0; j < a->items->len; j++)
            if (!strcmp(a->items->pdata[j], from)) {
                g_free(a->items->pdata[j]);
                a->items->pdata[j] = g_strdup(to);
            }
    }
}

static void forget_refs(Store *s, const char *rel)
{
    g_hash_table_remove(s->fav, rel);
    g_hash_table_remove(s->turns, rel);
    for (guint i = 0; i < s->albums->len; i++) album_remove(s->albums->pdata[i], rel);
}

char *store_restore(Store *s, Gone *g, const char *root, GError **err)
{
    char *src = store_gone_path(s, g), *rel = g_strdup(g->rel), *dst = g_build_filename(root, rel, NULL);
    for (int n = 2; g_file_test(dst, G_FILE_TEST_EXISTS) && n < 1000; n++) {
        g_free(rel);
        g_free(dst);
        rel = numbered(g->rel, n);
        dst = g_build_filename(root, rel, NULL);
    }
    char *dir = g_path_get_dirname(dst);
    g_mkdir_with_parents(dir, 0700);
    g_free(dir);
    gboolean ok = move(src, dst, err);
    if (ok) {
        rename_refs(s, g->rel, rel);
        g_ptr_array_remove(s->gone, g);
    } else {
        g_clear_pointer(&rel, g_free);
    }
    g_free(src);
    g_free(dst);
    return rel;
}

gboolean store_forget(Store *s, Gone *g, GError **err)
{
    char *path = store_gone_path(s, g);
    gboolean ok = g_unlink(path) == 0 || !g_file_test(path, G_FILE_TEST_EXISTS);
    if (!ok) g_set_error_literal(err, G_IO_ERROR, G_IO_ERROR_FAILED, "That photo couldn't be deleted.");
    g_free(path);
    if (ok) {
        /* Its favourite and albums go too, unless another photo is there now
         * (or another deleted one came from the same place). */
        gboolean again = FALSE;
        for (guint i = 0; i < s->gone->len; i++) {
            Gone *o = s->gone->pdata[i];
            if (o != g && !strcmp(o->rel, g->rel)) again = TRUE;
        }
        if (!again) forget_refs(s, g->rel);
        g_ptr_array_remove(s->gone, g);
    }
    return ok;
}

int store_purge(Store *s, gint64 now)
{
    int n = 0;
    for (guint i = 0; i < s->gone->len;) {
        Gone *g = s->gone->pdata[i];
        if (now - g->when > (gint64)KEEP_DELETED_DAYS * 86400 && store_forget(s, g, NULL)) n++;
        else i++;
    }
    return n;
}
