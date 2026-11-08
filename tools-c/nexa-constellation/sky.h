/* Constellation's sky: every open window as a star, the windows of one app
 * as one figure, dashed lines between windows of one project. Built into
 * nexa-orbit (it shares Orbit's window list, icons and theme); its window
 * exists only while open. Design: TODOnexa_identity.md, section
 * "Constellation".
 *
 * Projects come from nexa-starmap (Rust), run in the background each time
 * the sky opens or its windows change; until it answers (or if it can't),
 * the stars show without project lines.
 *
 * Ctrl+S sends the picked window's file (the one nexa-starmap found) to a
 * paired device nearby with Nexa Link (nexa-linkd send); a result that
 * comes after the sky closed arrives as a Signal. NEXA_LINKD=<path>: the
 * Link program (tests).
 *
 * NEXA_CONSTELLATION_STATE=<file>: what is shown, as JSON, after every
 * change (for tests). NEXA_STARMAP=<path>: the graph program (default
 * nexa-starmap on PATH). */
#ifndef NEXA_CONSTELLATION_SKY_H
#define NEXA_CONSTELLATION_SKY_H

#include <gtk/gtk.h>

#include "layers.h"

struct sky_win {
    unsigned long id;      /* the compositor's id for it */
    int pid;
    char app_id[128];      /* the window's own app id */
    char app[128];         /* its app's name, for people */
    char title[256];
    int group;             /* windows of one app share this (any number >= 0) */
    GdkPixbuf *icon;       /* the app's icon, may be NULL; the sky takes its own reference */
    int ws, focused, urgent, minimized;
};

struct sky_colours {
    strata_rgb bg, surface, text, muted, accent, warn;
    double opacity;
};

struct sky_hooks {
    void (*focus)(unsigned long id);   /* go to this window (its layer too) */
    void (*close)(unsigned long id);   /* ask it to close */
};

void sky_init(const struct sky_hooks *h);
void sky_set_colours(const struct sky_colours *c);
void sky_set_animations(int on);
/* Orbit's windows, after every change (also while closed: the sky keeps
 * the order they were used in). `current`: the layer on screen. */
void sky_set_windows(const struct sky_win *w, int n, int current);
void sky_toggle(void);
int sky_is_open(void);
#endif
