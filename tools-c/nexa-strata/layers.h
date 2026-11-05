/* Strata: the parts with no screen in them (TODOnexa_identity.md, section
 * "Strata"). Used by the nexa-strata command, the layer stack in Orbit and
 * Living's tint; unit tests in strata-test.c. */
#ifndef NEXA_STRATA_LAYERS_H
#define NEXA_STRATA_LAYERS_H

#define STRATA_LAYERS 5        /* workspaces 1 to 5 in both sessions */
#define STRATA_TINT 0.14       /* default tint strength */
#define STRATA_TINT_MAX 0.3
#define STRATA_HUE_STEP 40.0   /* degrees around the colour wheel per layer */
#define STRATA_GAP_MS 150      /* moves closer together than this are ignored */

typedef struct { double r, g, b; } strata_rgb;

#define STRATA_DIM_MAX 0.6

/* A layer's mood (Layer moods, TODOnexa_identity.md phase 5): set in
 * Settings > Layers, applied when that layer is on screen. */
struct strata_mood {
    char name[32];    /* "Work"; "" = just its number */
    int quiet;        /* notifications: only important ones pop up (Signals) */
    char mute[256];   /* apps whose notifications don't pop up here, comma-separated */
    char style[24];   /* Living's background: "" = as Magic Layer, "none", or a style name */
    double dim;       /* darkens the background, 0..STRATA_DIM_MAX */
    char power[16];   /* "" = unchanged, "powersave" or "performance" (CPU governor) */
};

struct strata_conf {
    int show;      /* the layer stack when the layer changes */
    double tint;   /* 0 = off */
    struct strata_mood mood[STRATA_LAYERS];
};

/* The layer a move of `step` from `cur` lands on, kept within 1..LAYERS.
 * A current layer outside them (a named or special workspace) counts as
 * the surface. */
int strata_target(int cur, int step);

/* "+1", "-1", "1", "-2"... -> step; 0 if not a step. */
int strata_step(const char *s);

/* Layer n's colour: the accent turned (n-1) * HUE_STEP degrees. */
strata_rgb strata_tint(strata_rgb accent, int n);

/* strata.conf: show=yes|no, tint=0..0.3, and per layer N (1..5):
 * layer.N.name, layer.N.signals=all|quiet, layer.N.mute=App,App,
 * layer.N.style, layer.N.dim=0..0.6, layer.N.power=powersave|performance.
 * Unknown lines and bad values are ignored. */
void strata_conf_parse(struct strata_conf *c, const char *text);

/* Does layer mood m keep app's notifications from popping up? */
int strata_mood_mutes(const struct strata_mood *m, const char *app);

/* Where sheet i (0 = layer 1) is drawn when the view is at depth `pos`
 * (0 = layer 1, may be between layers while gliding). */
struct strata_sheet {
    double scale;   /* 1 = the current layer's size */
    double rise;    /* px above the current layer's place (negative: below) */
    double alpha;   /* 0 = not drawn */
};
struct strata_sheet strata_sheet_at(int i, double pos);

/* Ease-out cubic, 0..1 -> 0..1. */
double strata_ease(double t);
#endif
