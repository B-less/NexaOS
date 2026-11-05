/* What Photos reads from a picture file without decoding it: the size,
 * EXIF (when taken, which way up, camera, settings, where) from a JPEG's
 * APP1 segment, a TIFF file, a WebP EXIF chunk or a PNG eXIf chunk. Every
 * read is bounds-checked; a damaged file gives what could be read. No GTK
 * here, so it is unit-tested (tests/unit.c). */
#ifndef PHOTOS_EXIF_H
#define PHOTOS_EXIF_H

#include <stddef.h>

typedef struct {
    int width, height;     /* 0 if unknown */
    int orientation;       /* EXIF 1..8; 1 = as stored */
    char taken[20];        /* "YYYY:MM:DD HH:MM:SS", or "" */
    char make[48], model[64], lens[64];
    char exposure[16], fnumber[12], focal[16];
    int iso;
    int has_gps;           /* coordinates read */
    double lat, lon;
} PhotoInfo;

enum { KIND_NONE, KIND_JPEG, KIND_PNG, KIND_WEBP, KIND_GIF, KIND_TIFF, KIND_BMP };

/* The kind of picture from its first bytes (not its name). */
int photo_kind(const unsigned char *b, size_t n);

/* Everything above from the file's bytes (the start of the file is enough
 * for most: Photos reads at most 1 MB). 0 if it is a picture we know. */
int photo_info_parse(const unsigned char *b, size_t n, PhotoInfo *out);

/* EXIF from TIFF-structured bytes ("II*\0" or "MM\0*"). 0 if it parsed. */
int exif_parse_tiff(const unsigned char *b, size_t n, PhotoInfo *out);

/* "YYYY:MM:DD HH:MM:SS" -> seconds since 1970 (as local time is not known,
 * the time is taken as UTC: only used to sort and group). -1 if bad. */
long long exif_time(const char *s);

/* Orientation after a quarter turn right (+1) or left (-1). */
int orient_turn(int o, int quarter_turns);
/* Width and height as shown, after orientation o. */
void orient_size(int o, int w, int h, int *sw, int *sh);
#endif
