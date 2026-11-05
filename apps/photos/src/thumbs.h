/* Decoding pictures for Photos, off the main thread (no GTK widgets here).
 * JPEGs are decoded with libjpeg-turbo, shrunk while decoding (1/2, 1/4,
 * 1/8), which is about 25 times faster than going through gdk-pixbuf;
 * everything else goes through gdk-pixbuf. Pictures are turned the right
 * way up (EXIF orientation plus turns made in Photos). Thumbnails are kept
 * as small PNGs in ~/.cache/photos/thumbs. */
#ifndef PHOTOS_THUMBS_H
#define PHOTOS_THUMBS_H

#include <cairo.h>

#include "library.h"

#define THUMB_PX 320          /* cached thumbnails: squares this big */
#define VIEW_MAX_PIXELS 60000000L

/* The picture, shown the right way up: shrunk to cover a box_w x box_h box
 * (cover) or fit in it, or (box 0) as big as VIEW_MAX_PIXELS allows. NULL
 * and *why (plain words) if it can't be shown. */
cairo_surface_t *picture_load(const Photo *p, int orientation, int box_w, int box_h, int cover, char **why);

/* A THUMB_PX square from the middle of the picture, from the cache when it
 * is there. NULL if the picture can't be read. */
cairo_surface_t *thumb_square(const Photo *p, int orientation, const char *cache_dir);

/* The cache file a thumbnail is kept in. */
char *thumb_path(const Photo *p, int orientation, const char *cache_dir);

/* Draw `src` (as stored) turned by EXIF orientation o: a new surface. */
cairo_surface_t *orient_surface(cairo_surface_t *src, int o);
#endif
