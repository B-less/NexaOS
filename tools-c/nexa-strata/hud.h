/* Strata's layer stack: five tilted sheets shown for about a second when
 * the layer on screen changes. Built into nexa-orbit (it shares Orbit's
 * compositor connection, icons and theme); its window exists only while
 * shown. Design: TODOnexa_identity.md, section "Strata".
 *
 * NEXA_STRATA_STATE=<file>: what is shown, as JSON, after every change
 * (for tests). */
#ifndef NEXA_STRATA_HUD_H
#define NEXA_STRATA_HUD_H

#include <gtk/gtk.h>

#include "layers.h"

#define HUD_ICONS 4

struct hud_layer {
    int windows;
    int nicons;
    GdkPixbuf *icons[HUD_ICONS];  /* one per app; the stack keeps its own reference */
};

struct hud_colours {
    strata_rgb bg, surface, text, muted, accent;
    double opacity;
};

void hud_init(const char *conf_path);
void hud_reload_conf(void);
void hud_set_colours(const struct hud_colours *c);
void hud_set_animations(int on);
void hud_set_layers(const struct hud_layer layers[STRATA_LAYERS]);
/* The workspace on screen. The first call only notes it; after that a
 * change to another layer shows the stack (if strata.conf says show=yes). */
void hud_set_current(int ws);
#endif
