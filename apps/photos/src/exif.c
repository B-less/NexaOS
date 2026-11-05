#include "exif.h"

#include <math.h>
#include <stdio.h>
#include <string.h>

/* ---- reading numbers, bounds-checked ---- */

typedef struct {
    const unsigned char *b;
    size_t n;
    int le;
} Tiff;

static int get16(const Tiff *t, size_t o, unsigned *v)
{
    if (o > t->n || t->n - o < 2) return 0;
    const unsigned char *p = t->b + o;
    *v = t->le ? (unsigned)(p[0] | p[1] << 8) : (unsigned)(p[0] << 8 | p[1]);
    return 1;
}

static int get32(const Tiff *t, size_t o, unsigned long *v)
{
    if (o > t->n || t->n - o < 4) return 0;
    const unsigned char *p = t->b + o;
    *v = t->le ? ((unsigned long)p[0] | (unsigned long)p[1] << 8 | (unsigned long)p[2] << 16 | (unsigned long)p[3] << 24)
               : ((unsigned long)p[0] << 24 | (unsigned long)p[1] << 16 | (unsigned long)p[2] << 8 | (unsigned long)p[3]);
    return 1;
}

static unsigned be16(const unsigned char *p) { return (unsigned)(p[0] << 8 | p[1]); }
static unsigned le16(const unsigned char *p) { return (unsigned)(p[0] | p[1] << 8); }
static unsigned long be32(const unsigned char *p)
{
    return (unsigned long)p[0] << 24 | (unsigned long)p[1] << 16 | (unsigned long)p[2] << 8 | p[3];
}
static unsigned long le32(const unsigned char *p)
{
    return (unsigned long)p[0] | (unsigned long)p[1] << 8 | (unsigned long)p[2] << 16 | (unsigned long)p[3] << 24;
}

/* One IFD entry: where its value is, if all of it is inside the block. */
typedef struct {
    unsigned tag, kind;
    unsigned long count;
    size_t at;
} Entry;

static int entry(const Tiff *t, size_t e, Entry *out)
{
    unsigned tag, kind;
    unsigned long count, off;
    if (!get16(t, e, &tag) || !get16(t, e + 2, &kind) || !get32(t, e + 4, &count)) return 0;
    unsigned long unit;
    switch (kind) {
    case 1: case 2: case 6: case 7: unit = 1; break;
    case 3: case 8: unit = 2; break;
    case 4: case 9: case 11: unit = 4; break;
    case 5: case 10: case 12: unit = 8; break;
    default: return 0;
    }
    if (count > t->n) return 0;  /* also keeps unit * count from overflowing */
    unsigned long size = unit * count;
    size_t at;
    if (size <= 4) {
        at = e + 8;
    } else {
        if (!get32(t, e + 8, &off)) return 0;
        at = (size_t)off;
    }
    if (at > t->n || t->n - at < size) return 0;
    *out = (Entry){tag, kind, count, at};
    return 1;
}

static int number(const Tiff *t, const Entry *e, unsigned long *v)
{
    unsigned s;
    if (e->kind == 3) {
        if (!get16(t, e->at, &s)) return 0;
        *v = s;
        return 1;
    }
    if (e->kind == 4) return get32(t, e->at, v);
    return 0;
}

static int rational(const Tiff *t, size_t at, double *v)
{
    unsigned long n, d;
    if (!get32(t, at, &n) || !get32(t, at + 4, &d) || d == 0) return 0;
    *v = (double)n / (double)d;
    return 1;
}

/* Text: printable ASCII only, trimmed. */
static void ascii(const Tiff *t, const Entry *e, char *dst, size_t cap)
{
    if (e->kind != 2 || !cap) return;
    size_t k = 0;
    for (unsigned long i = 0; i < e->count && k + 1 < cap; i++) {
        unsigned char c = t->b[e->at + i];
        if (!c) break;
        dst[k++] = (c >= 32 && c < 127) ? (char)c : '?';
    }
    while (k && dst[k - 1] == ' ') k--;
    size_t s = 0;
    while (s < k && dst[s] == ' ') s++;
    memmove(dst, dst + s, k - s);
    dst[k - s] = 0;
}

/* "2.80" -> "2.8", "35.0" -> "35". */
static void trim_num(char *dst, size_t cap, double v, int places)
{
    snprintf(dst, cap, "%.*f", places, v);
    if (strchr(dst, '.')) {
        size_t k = strlen(dst);
        while (k && dst[k - 1] == '0') dst[--k] = 0;
        if (k && dst[k - 1] == '.') dst[--k] = 0;
    }
}

static double dms(const Tiff *t, const Entry *e, int *ok)
{
    double d, m, s;
    *ok = e->kind == 5 && e->count >= 3 && rational(t, e->at, &d) && rational(t, e->at + 8, &m) &&
          rational(t, e->at + 16, &s);
    return *ok ? d + m / 60 + s / 3600 : 0;
}

static void read_gps(const Tiff *t, size_t ifd, PhotoInfo *x)
{
    unsigned n;
    if (!get16(t, ifd, &n)) return;
    char lat_ref = 0, lon_ref = 0;
    double lat = 0, lon = 0;
    int have_lat = 0, have_lon = 0;
    for (unsigned i = 0; i < n && i < 512; i++) {
        Entry e;
        if (!entry(t, ifd + 2 + (size_t)i * 12, &e)) continue;
        if (e.tag == 1 && e.kind == 2) lat_ref = (char)t->b[e.at];
        else if (e.tag == 3 && e.kind == 2) lon_ref = (char)t->b[e.at];
        else if (e.tag == 2) lat = dms(t, &e, &have_lat);
        else if (e.tag == 4) lon = dms(t, &e, &have_lon);
    }
    if (!have_lat || !have_lon || lat > 90 || lon > 180) return;
    if (lat_ref == 'S') lat = -lat;
    if (lon_ref == 'W') lon = -lon;
    x->lat = lat;
    x->lon = lon;
    x->has_gps = 1;
}

int exif_parse_tiff(const unsigned char *b, size_t n, PhotoInfo *x)
{
    if (n < 8) return -1;
    Tiff t = {b, n, 0};
    if (!memcmp(b, "II*\0", 4)) t.le = 1;
    else if (memcmp(b, "MM\0*", 4)) return -1;
    unsigned long ifd0, sub = 0, gps = 0;
    unsigned cnt;
    if (!get32(&t, 4, &ifd0) || !get16(&t, ifd0, &cnt)) return -1;
    char date0[20] = "";
    for (unsigned i = 0; i < cnt && i < 512; i++) {
        Entry e;
        unsigned long v;
        if (!entry(&t, ifd0 + 2 + (size_t)i * 12, &e)) continue;
        switch (e.tag) {
        case 0x0100: if (number(&t, &e, &v) && v < 1000000) x->width = (int)v; break;
        case 0x0101: if (number(&t, &e, &v) && v < 1000000) x->height = (int)v; break;
        case 0x010f: ascii(&t, &e, x->make, sizeof x->make); break;
        case 0x0110: ascii(&t, &e, x->model, sizeof x->model); break;
        case 0x0112:
            if (e.kind == 3 && number(&t, &e, &v) && v >= 1 && v <= 8) x->orientation = (int)v;
            break;
        case 0x0132: ascii(&t, &e, date0, sizeof date0); break;
        case 0x8769: if (number(&t, &e, &v)) sub = v; break;
        case 0x8825: if (number(&t, &e, &v)) gps = v; break;
        default: break;
        }
    }
    if (sub && sub != ifd0 && get16(&t, sub, &cnt)) {
        for (unsigned i = 0; i < cnt && i < 512; i++) {
            Entry e;
            unsigned long v;
            double r;
            if (!entry(&t, sub + 2 + (size_t)i * 12, &e)) continue;
            switch (e.tag) {
            case 0x829a:  /* exposure time */
                if (e.kind == 5 && rational(&t, e.at, &r)) {
                    if (r <= 0) snprintf(x->exposure, sizeof x->exposure, "0 s");
                    else if (r < 1) snprintf(x->exposure, sizeof x->exposure, "1/%.0f s", fmin(1e6, round(1 / r)));
                    else {
                        char num[12];
                        trim_num(num, sizeof num, fmin(r, 99999), 1);
                        snprintf(x->exposure, sizeof x->exposure, "%s s", num);
                    }
                }
                break;
            case 0x829d:
                if (e.kind == 5 && rational(&t, e.at, &r) && r > 0 && r < 1000) {
                    char num[8];
                    trim_num(num, sizeof num, r, 1);
                    snprintf(x->fnumber, sizeof x->fnumber, "f/%s", num);
                }
                break;
            case 0x8827: if (number(&t, &e, &v) && v < 10000000) x->iso = (int)v; break;
            case 0x9003: ascii(&t, &e, x->taken, sizeof x->taken); break;
            case 0x920a:
                if (e.kind == 5 && rational(&t, e.at, &r) && r > 0 && r < 100000) {
                    char num[10];
                    trim_num(num, sizeof num, r, 1);
                    snprintf(x->focal, sizeof x->focal, "%s mm", num);
                }
                break;
            case 0xa434: ascii(&t, &e, x->lens, sizeof x->lens); break;
            default: break;
            }
        }
    }
    if (!x->taken[0] && date0[0]) memcpy(x->taken, date0, sizeof date0);
    if (exif_time(x->taken) < 0) x->taken[0] = 0;
    if (gps && gps != ifd0) read_gps(&t, gps, x);
    return 0;
}

/* ---- containers ---- */

int photo_kind(const unsigned char *b, size_t n)
{
    if (n >= 3 && b[0] == 0xff && b[1] == 0xd8 && b[2] == 0xff) return KIND_JPEG;
    if (n >= 8 && !memcmp(b, "\x89PNG\r\n\x1a\n", 8)) return KIND_PNG;
    if (n >= 12 && !memcmp(b, "RIFF", 4) && !memcmp(b + 8, "WEBP", 4)) return KIND_WEBP;
    if (n >= 6 && (!memcmp(b, "GIF87a", 6) || !memcmp(b, "GIF89a", 6))) return KIND_GIF;
    if (n >= 4 && (!memcmp(b, "II*\0", 4) || !memcmp(b, "MM\0*", 4))) return KIND_TIFF;
    if (n >= 26 && b[0] == 'B' && b[1] == 'M') return KIND_BMP;
    return KIND_NONE;
}

static void jpeg_info(const unsigned char *b, size_t n, PhotoInfo *x)
{
    size_t i = 2;
    int seen_exif = 0;
    while (i + 4 <= n) {
        if (b[i] != 0xff) return;  /* lost: stop, keep what we have */
        unsigned m = b[i + 1];
        if (m == 0xff) { i++; continue; }  /* fill byte */
        if (m == 0xd8 || m == 0x01 || (m >= 0xd0 && m <= 0xd7)) { i += 2; continue; }
        if (m == 0xd9 || m == 0xda) return;  /* end, or the picture data starts */
        unsigned len = be16(b + i + 2);
        if (len < 2) return;
        const unsigned char *seg = b + i + 4;
        size_t seglen = len - 2;
        if (i + 4 + seglen > n) seglen = n - (i + 4);  /* cut short: use what is here */
        if (m == 0xe1 && seglen > 6 && !memcmp(seg, "Exif\0\0", 6) && !seen_exif) {
            seen_exif = 1;  /* the first one; later ones are often a thumbnail's */
            exif_parse_tiff(seg + 6, seglen - 6, x);
        }
        if (((m >= 0xc0 && m <= 0xc3) || (m >= 0xc5 && m <= 0xc7) || (m >= 0xc9 && m <= 0xcb) ||
             (m >= 0xcd && m <= 0xcf)) && seglen >= 5) {
            x->height = (int)be16(seg + 1);
            x->width = (int)be16(seg + 3);
        }
        i += 2 + len;
    }
}

static void png_info(const unsigned char *b, size_t n, PhotoInfo *x)
{
    size_t i = 8;
    while (i + 8 <= n) {
        unsigned long len = be32(b + i);
        const unsigned char *type = b + i + 4, *data = b + i + 8;
        if (len > n - i - 8) len = n - i - 8;
        if (!memcmp(type, "IHDR", 4) && len >= 8) {
            unsigned long w = be32(data), h = be32(data + 4);
            if (w < 1000000 && h < 1000000) x->width = (int)w, x->height = (int)h;
        } else if (!memcmp(type, "eXIf", 4)) {
            exif_parse_tiff(data, len, x);
        } else if (!memcmp(type, "IEND", 4)) {
            return;
        }
        if (n - i - 8 - len < 4) return;
        i += 12 + len;
    }
}

static void webp_info(const unsigned char *b, size_t n, PhotoInfo *x)
{
    size_t i = 12;
    while (i + 8 <= n) {
        const unsigned char *id = b + i, *d = b + i + 8;
        unsigned long len = le32(b + i + 4), have = n - i - 8;
        unsigned long l = len < have ? len : have;
        if (!memcmp(id, "VP8X", 4) && l >= 10) {
            x->width = (int)(1 + (d[4] | d[5] << 8 | d[6] << 16));
            x->height = (int)(1 + (d[7] | d[8] << 8 | d[9] << 16));
        } else if (!memcmp(id, "VP8 ", 4) && l >= 10 && !x->width) {
            if (d[3] == 0x9d && d[4] == 0x01 && d[5] == 0x2a) {
                x->width = (int)(le16(d + 6) & 0x3fff);
                x->height = (int)(le16(d + 8) & 0x3fff);
            }
        } else if (!memcmp(id, "VP8L", 4) && l >= 5 && !x->width) {
            if (d[0] == 0x2f) {
                unsigned long v = le32(d + 1);
                x->width = (int)((v & 0x3fff) + 1);
                x->height = (int)(((v >> 14) & 0x3fff) + 1);
            }
        } else if (!memcmp(id, "EXIF", 4)) {
            if (l > 6 && !memcmp(d, "Exif\0\0", 6)) exif_parse_tiff(d + 6, l - 6, x);
            else exif_parse_tiff(d, l, x);
        }
        if (len >= have) return;
        i += 8 + len + (len & 1);
    }
}

int photo_info_parse(const unsigned char *b, size_t n, PhotoInfo *x)
{
    memset(x, 0, sizeof *x);
    x->orientation = 1;
    int kind = photo_kind(b, n);
    switch (kind) {
    case KIND_JPEG: jpeg_info(b, n, x); break;
    case KIND_PNG: png_info(b, n, x); break;
    case KIND_WEBP: webp_info(b, n, x); break;
    case KIND_GIF: x->width = (int)le16(b + 6), x->height = (int)le16(b + 8); break;
    case KIND_TIFF: exif_parse_tiff(b, n, x); break;
    case KIND_BMP: {
        long w = (long)le32(b + 18), h = (long)(int)le32(b + 22);
        if (h < 0) h = -h;
        if (w > 0 && w < 1000000 && h < 1000000) x->width = (int)w, x->height = (int)h;
        break;
    }
    default: return -1;
    }
    if (x->orientation < 1 || x->orientation > 8) x->orientation = 1;
    return 0;
}

/* ---- dates ---- */

static long long days_from_civil(int y, int m, int d)
{
    y -= m <= 2;
    long long era = (y >= 0 ? y : y - 399) / 400;
    long long yoe = y - era * 400;
    long long doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    long long doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}

long long exif_time(const char *s)
{
    int Y, M, D, h, m, sec;
    char tail;
    if (!s || sscanf(s, "%4d:%2d:%2d %2d:%2d:%2d%c", &Y, &M, &D, &h, &m, &sec, &tail) != 6) return -1;
    if (Y < 1900 || Y > 2200 || M < 1 || M > 12 || D < 1 || D > 31 || h > 23 || m > 59 || sec > 60 || h < 0 || m < 0 ||
        sec < 0)
        return -1;
    return days_from_civil(Y, M, D) * 86400 + h * 3600 + m * 60 + sec;
}

/* ---- orientation ---- */

static const int ORIENT[8][4] = {
    {1, 0, 0, 1},   {-1, 0, 0, 1}, {-1, 0, 0, -1}, {1, 0, 0, -1},
    {0, 1, 1, 0},   {0, -1, 1, 0}, {0, -1, -1, 0}, {0, 1, -1, 0},
};

int orient_turn(int o, int q)
{
    if (o < 1 || o > 8) o = 1;
    q %= 4;
    if (q < 0) q += 4;
    for (int k = 0; k < q; k++) {
        const int *a = ORIENT[5], *b = ORIENT[o - 1];
        int m[4] = {a[0] * b[0] + a[1] * b[2], a[0] * b[1] + a[1] * b[3], a[2] * b[0] + a[3] * b[2],
                    a[2] * b[1] + a[3] * b[3]};
        for (int i = 0; i < 8; i++)
            if (!memcmp(ORIENT[i], m, sizeof m)) { o = i + 1; break; }
    }
    return o;
}

void orient_size(int o, int w, int h, int *sw, int *sh)
{
    int swap = o >= 5 && o <= 8;
    *sw = swap ? h : w;
    *sh = swap ? w : h;
}
