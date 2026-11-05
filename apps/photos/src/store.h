/* What Photos keeps about the library: favourites, albums, photos turned
 * in Photos (without changing the file) and Deleted. All in
 * ~/.local/share/photos/library.ini, by each photo's path inside
 * ~/Pictures. GLib/GIO only, unit-tested.
 *
 * Deleted: an installed app may not reach the system Trash (it lives
 * outside ~/Pictures), so deleted photos move to
 * ~/.local/share/photos/deleted and can be put back for 30 days. */
#ifndef PHOTOS_STORE_H
#define PHOTOS_STORE_H

#include <gio/gio.h>

#define KEEP_DELETED_DAYS 30
#define ALBUM_NAME_MAX 60

typedef struct {
    char *name;
    GPtrArray *items;  /* rel paths, in the order added */
} Album;

typedef struct {
    char *file;        /* its name in the deleted folder */
    char *rel;         /* where it was */
    gint64 when;       /* deleted at (unix time) */
} Gone;

typedef struct {
    char *path, *deleted_dir;
    GHashTable *fav;    /* rel set */
    GHashTable *turns;  /* rel -> quarter turns right, 1..3 */
    GPtrArray *albums;  /* Album* */
    GPtrArray *gone;    /* Gone*, oldest first */
} Store;

Store *store_load(const char *data_dir);
gboolean store_save(Store *s, GError **err);
void store_free(Store *s);

gboolean store_is_fav(Store *s, const char *rel);
void store_set_fav(Store *s, const char *rel, gboolean on);
int store_turn(Store *s, const char *rel);
void store_set_turn(Store *s, const char *rel, int quarter_turns);

/* NULL if name can be an album's (renaming `except`), else why not, in
 * plain words. */
const char *store_name_problem(Store *s, const char *name, const Album *except);
Album *store_album_new(Store *s, const char *name);  /* name checked first */
Album *store_album_find(Store *s, const char *name);
void store_album_delete(Store *s, Album *a);        /* the photos stay */
gboolean album_has(const Album *a, const char *rel);
void album_add(Album *a, const char *rel);
void album_remove(Album *a, const char *rel);
/* The names of the albums rel is in, space-separated (for search). */
char *store_albums_of(Store *s, const char *rel);

/* Move root/rel into Deleted. */
Gone *store_delete(Store *s, const char *root, const char *rel, gint64 now, GError **err);
/* Put it back (under another name if one took its place): its new rel. */
char *store_restore(Store *s, Gone *g, const char *root, GError **err);
/* Delete it for good. */
gboolean store_forget(Store *s, Gone *g, GError **err);
/* Forget everything deleted more than KEEP_DELETED_DAYS ago: how many. */
int store_purge(Store *s, gint64 now);
char *store_gone_path(Store *s, const Gone *g);
#endif
