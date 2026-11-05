/* Backup's engine (no GTK; unit-tested in tests/unit.c).
 *
 * On the drive: "NexaOS Backup/<user>/"
 *   objects/ab/abcdef...   each file's contents once, named by its SHA-256
 *   snapshots/<time>.snap  one backup: every file, folder and link, with
 *                          its hash; written last and renamed into place,
 *                          so a backup that was cut short is never listed
 * The next backup copies only what is new or changed (same size and time
 * as last time: taken from the last backup, not read again).
 *
 * Never backed up: caches, the Trash, secret keys and password stores,
 * browser profiles, NexaOS's own private data (docs/WRITING_NEXA_APPS.md),
 * other drives, sockets and devices. */
#ifndef BACKUP_ENGINE_H
#define BACKUP_ENGINE_H

#include <glib.h>

#define SNAP_MAGIC "NXBK1"

typedef struct {
    char kind;        /* 'F' file, 'D' folder, 'L' link */
    guint mode;
    gint64 mtime, size;
    char hash[65];    /* files: SHA-256 in hex */
    char *path;       /* relative to home */
    char *target;     /* links */
} Item;

typedef struct {
    char *name;       /* "2026-11-05_143000" */
    char *file;       /* its .snap path */
    gint64 when, files, bytes, added;
    GPtrArray *items; /* Item*, loaded on demand (snap_load) */
} Snap;

typedef struct {
    volatile gint *cancel;
    void (*progress)(gpointer data, gint64 done_files, gint64 total_files, gint64 done_bytes, gint64 total_bytes,
                     const char *path);
    gpointer data;
} Job;

/* ---- what is backed up ---- */

/* 1 if the home-relative path is always left out. */
gboolean always_left_out(const char *rel);
/* Top-level names the person switched off (NULL: none). */
typedef struct {
    GHashTable *off;
} Choice;
gboolean choice_includes(const Choice *c, const char *rel);

/* ---- the drive ---- */

char *backup_root(const char *drive);  /* <drive>/NexaOS Backup/<user> */
/* Backups on the drive, newest first (Snap*, items not loaded). */
GPtrArray *snaps_list(const char *drive);
gboolean snap_load(Snap *s, GError **err);
void snap_free(Snap *s);
/* Free and total bytes on the drive. */
gboolean drive_space(const char *drive, guint64 *free_b, guint64 *total_b);

/* ---- backing up ---- */

typedef struct {
    gint64 files, bytes;       /* in the backup */
    gint64 copied, copied_bytes;
    gint64 skipped;            /* couldn't be read: listed in `problems` */
    GPtrArray *problems;       /* char*: "path: why" */
    char *snap_name;
} Result;

/* What a backup would copy: bytes not on the drive yet. */
gint64 backup_estimate(const char *home, const char *drive, const Choice *c, gint64 *files, GError **err);
/* Back up home to the drive. FALSE and *err (plain words) if it stopped:
 * the drive is full or gone, or it was cancelled. */
gboolean backup_run(const char *home, const char *drive, const Choice *c, gint64 now, Job *job, Result *out, GError **err);
void result_clear(Result *r);

/* ---- putting back ---- */

/* The items of snapshot s at or under each of `paths` (home-relative; ""
 * = everything). Into `dest` (NULL: where they were). A file that is
 * there and differs is kept, the restored one beside it as
 * "name (restored).ext". Counts what was put back. */
gboolean restore_run(const char *drive, Snap *s, const char *const *paths, const char *home, const char *dest, Job *job,
                     gint64 *restored, GPtrArray *problems, GError **err);

/* ---- keeping the drive tidy ---- */

/* Every object a backup needs re-read and checked: how many are damaged
 * or missing. */
gint64 check_snap(const char *drive, Snap *s, Job *job, GError **err);
/* Which backups to keep: every one of the last 2 days, one a day for 14
 * days, one a week for 8 weeks, one a month for 2 years. The names to
 * delete (newest-first `names`, `now` in unix time). */
GPtrArray *prune_plan(GPtrArray *names_when, gint64 now);
/* Delete one backup and every object no other backup needs. */
gboolean snap_delete(const char *drive, const char *name, GError **err);
/* Objects no backup needs any more: deleted. How many. */
gint64 collect_garbage(const char *drive, GError **err);

/* Helpers, tested */
char *restored_name(const char *path);  /* "a/b.jpg" -> "a/b (restored).jpg" */
char *escape_field(const char *s);
char *unescape_field(const char *s);
#endif
