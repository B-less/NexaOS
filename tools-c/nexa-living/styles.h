/* The ten living-background styles of nexa-living. Each draws one frame of
 * a region (an app window's area, or the desktop) from the time and a few
 * parameters. Geometry is made once per region (struct region) and only
 * moved afterwards: no allocation per frame. */
#ifndef NEXA_LIVING_STYLES_H
#define NEXA_LIVING_STYLES_H
#include <cairo.h>

typedef struct { double r, g, b; } rgb;

enum style_id {
    ST_CIRCUIT, ST_QUANTUM, ST_NEURAL, ST_HOLOGRID, ST_RAIN,
    ST_NEBULA, ST_PULSE, ST_ORBITAL, ST_GEOMETRY, ST_SIGNATURE, ST_COUNT
};

struct params {
    double t;          /* seconds */
    double speed;      /* 0.25 .. 2, 1 normal */
    double bright;     /* 0 .. 1.5, 1 normal */
    double density;    /* 0.25 .. 2, 1 normal (times the quality's own factor) */
    double level;      /* 0 .. 1: real activity (load, playback), 0 if none */
    double focus;      /* 1 focused, ~0.55 not */
    double px, py;     /* the pointer in region coordinates, or < -1e5 */
    double reaction;   /* 0 .. 1: how much the pointer matters */
    int quality;       /* 0 low, 1 balanced, 2 high */
    rgb a1, a2, a3;    /* electric blue, purple, white-ish */
};

#define MAX_PTS 4096
#define MAX_ITEMS 512

struct region {
    int style, w, h;
    double density;          /* what the geometry was made for */
    unsigned seed;
    double last_t;           /* for styles with state (particles) */
    int n, m;                /* item counts */
    float x[MAX_ITEMS], y[MAX_ITEMS], u[MAX_ITEMS], v[MAX_ITEMS], k[MAX_ITEMS];
    int start[MAX_ITEMS + 1];/* polylines: points start[i] .. start[i+1]-1 */
    float px[MAX_PTS], py[MAX_PTS], len[MAX_ITEMS];
    cairo_surface_t *cache;  /* a style's slow layer (Nebula's clouds), made once */
};

extern const char *const style_names[ST_COUNT];   /* "circuit", ... */
extern const char *const style_titles[ST_COUNT];  /* "Cyber Circuit", ... */
int style_by_name(const char *name);              /* -1 if unknown */

/* Makes the geometry if style, size or density changed (frees the old cache). */
void region_prepare(struct region *r, int style, int w, int h, double density, unsigned seed);
/* One frame, into cr (already clipped/translated to the region). */
void style_draw(cairo_t *cr, struct region *r, const struct params *p);
void region_free(struct region *r);

#endif
