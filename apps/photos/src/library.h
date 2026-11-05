/* The photo library: every picture under ~/Pictures (subfolders too,
 * hidden ones skipped), newest first. What was read from each file is kept
 * in an index (~/.cache/photos/index), so the next start reads only new or
 * changed files. GLib only: safe to run off the main thread. */
#ifndef PHOTOS_LIBRARY_H
#define PHOTOS_LIBRARY_H

#include <glib.h>

#include "exif.h"

typedef struct {
    char *path;      /* absolute */
    char *rel;       /* relative to the library folder, or NULL if outside it */
    char *name;      /* file name, for people */
    gint64 mtime, size;
    gint64 taken;    /* when taken (wall clock as if UTC): sorts and groups */
    gboolean exif_date;
    PhotoInfo info;
} Photo;

#define LIB_MAX_DEPTH 10
#define LIB_MAX_PHOTOS 50000

/* Photo from a file: NULL if it isn't a picture we know (or can't be read). */
Photo *photo_read(const char *path, const char *root);
void photo_free(Photo *p);

/* Every picture under root, newest first. index: where the index is kept
 * (NULL: none). *cancel set to 1 from another thread stops it early. */
GPtrArray *library_scan(const char *root, const char *index, volatile gint *cancel);

/* Picture names Photos lists (by extension; contents are checked too). */
gboolean library_wanted_name(const char *name);

/* Month of a taken time: year * 12 + month - 1. */
int photo_month(gint64 taken);
/* "November 2026" */
char *month_title(int month);
/* "Sunday 1 November 2026, 18:04" */
char *photo_when(const Photo *p);

/* Does the photo match every word of a search ("2026 hex beach")? Looks at
 * its name, folder, month, camera and the album names given. */
gboolean photo_matches(const Photo *p, const char *query, const char *albums);
#endif
