#include "hud.h"

#include <gtk-layer-shell.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

#define HUD_W 300
#define HUD_H 240
#define HUD_MARGIN 90      /* above the bottom edge */
#define STACK_X 128.0      /* centre of the sheets */
#define BASE_Y (HUD_H - 54.0)  /* centre of the current sheet */
#define SHEET_BW 210.0     /* near edge */
#define SHEET_TW 170.0     /* far edge: the sheet is tilted away */
#define SHEET_H 54.0
#define GAUGE_X 272.0
#define GAUGE_STEP 33.0
#define FADE_IN 0.12
#define HOLD 0.9
#define FADE_OUT 0.26
#define GLIDE 0.22

static struct {
    GtkWidget *win, *area;
    char conf_path[600];
    struct strata_conf conf;
    struct hud_colours c;
    struct hud_layer layer[STRATA_LAYERS];
    int animations, current, known;
    double from, to, glide_at, shown_at, changed_at;
    guint tick;
} H = {.animations = 1, .c = {{0.03, 0.05, 0.12}, {0.08, 0.11, 0.23}, {0.9, 0.93, 1}, {0.56, 0.61, 0.77}, {0.3, 0.64, 1}, 0.78}};

static double now_s(void) { return g_get_monotonic_time() / 1e6; }

static void set_rgb(cairo_t *cr, strata_rgb c, double a) { cairo_set_source_rgba(cr, c.r, c.g, c.b, a); }

static strata_rgb mix(strata_rgb a, strata_rgb b, double t)
{
    return (strata_rgb){a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t};
}

/* The depth shown now (0 = layer 1), gliding from `from` to `to`. */
static double pos_now(double t)
{
    if (!H.animations) return H.to;
    return H.from + (H.to - H.from) * strata_ease((t - H.glide_at) / GLIDE);
}

static double alpha_now(double t)
{
    double end = H.changed_at + HOLD;
    if (!H.animations) return t < end + FADE_OUT ? 1 : 0;
    double a = fmin(1, (t - H.shown_at) / FADE_IN);
    if (t > end) a = fmin(a, 1 - (t - end) / FADE_OUT);
    return fmax(0, a);
}

/* ---------- test state ---------- */

static void write_state(void)
{
    const char *path = g_getenv("NEXA_STRATA_STATE");
    if (!path) return;
    double t = now_s();
    GString *s = g_string_new(NULL);
    g_string_append_printf(s, "{\"shown\": %d, \"current\": %d, \"pos\": %.2f, \"show\": %d, \"tint\": %.2f, \"animations\": %d",
                           H.win != NULL, H.current, H.win ? pos_now(t) + 1 : (double)H.current, H.conf.show, H.conf.tint,
                           H.animations);
    if (H.win)
        g_string_append_printf(s, ", \"surface\": [%d, %d]", gtk_widget_get_allocated_width(H.area),
                               gtk_widget_get_allocated_height(H.area));
    g_string_append(s, ", \"layers\": [");
    for (int i = 0; i < STRATA_LAYERS; i++) {
        strata_rgb c = strata_tint(H.c.accent, i + 1);
        g_string_append_printf(s, "%s{\"windows\": %d, \"icons\": %d, \"tint\": \"#%02x%02x%02x\"}", i ? ", " : "", H.layer[i].windows,
                               H.layer[i].nicons, (int)lround(c.r * 255), (int)lround(c.g * 255), (int)lround(c.b * 255));
    }
    g_string_append(s, "]}\n");
    char *tmp = g_strconcat(path, ".tmp", NULL);
    if (g_file_set_contents(tmp, s->str, (gssize)s->len, NULL)) rename(tmp, path);
    g_free(tmp);
    g_string_free(s, TRUE);
}

/* ---------- drawing ---------- */

static void hex_path(cairo_t *cr, double cx, double cy, double r)
{
    for (int k = 0; k < 6; k++) {  /* pointy top, like Orbit's frames */
        double a = -M_PI / 2 + k * M_PI / 3, x = cx + r * cos(a), y = cy + r * sin(a);
        if (k) cairo_line_to(cr, x, y);
        else cairo_move_to(cr, x, y);
    }
    cairo_close_path(cr);
}

static void text_at(cairo_t *cr, const char *text, double size, int bold, double cx, double cy, int centred)
{
    PangoLayout *l = pango_cairo_create_layout(cr);
    PangoFontDescription *fd = pango_font_description_from_string(bold ? "Sans Bold" : "Sans");
    pango_font_description_set_absolute_size(fd, size * PANGO_SCALE);
    pango_layout_set_font_description(l, fd);
    pango_layout_set_text(l, text, -1);
    int w, h;
    pango_layout_get_pixel_size(l, &w, &h);
    cairo_move_to(cr, centred ? cx - w / 2.0 : cx, cy - h / 2.0);
    pango_cairo_show_layout(cr, l);
    pango_font_description_free(fd);
    g_object_unref(l);
}

static void draw_sheet(cairo_t *cr, int i, struct strata_sheet s, double a, double weight)
{
    strata_rgb tint = strata_tint(H.c.accent, i + 1);
    double cx = STACK_X, cy = BASE_Y - s.rise, bw = SHEET_BW * s.scale, tw = SHEET_TW * s.scale, h = SHEET_H * s.scale;
    cairo_new_path(cr);
    cairo_move_to(cr, cx - tw / 2, cy - h / 2);
    cairo_line_to(cr, cx + tw / 2, cy - h / 2);
    cairo_line_to(cr, cx + bw / 2, cy + h / 2);
    cairo_line_to(cr, cx - bw / 2, cy + h / 2);
    cairo_close_path(cr);
    cairo_set_line_join(cr, CAIRO_LINE_JOIN_ROUND);
    if (weight > 0) {  /* the current layer glows */
        set_rgb(cr, tint, 0.2 * weight * a);
        cairo_set_line_width(cr, 7);
        cairo_stroke_preserve(cr);
    }
    set_rgb(cr, mix(H.c.surface, tint, 0.18 + 0.1 * weight), fmin(0.96, H.c.opacity + 0.12) * a);
    cairo_fill_preserve(cr);
    set_rgb(cr, tint, (0.55 + 0.45 * weight) * a);
    cairo_set_line_width(cr, 1.2 + 1.3 * weight);
    cairo_stroke(cr);

    /* its number in a hexagon. A sheet behind shows only its upper part
     * above the one in front, so its labels move up into that strip. */
    double ly = cy - h * 0.25 * (1 - weight);
    double bx = cx - bw / 2 + 24 * s.scale, br = 11 * s.scale;
    hex_path(cr, bx, ly, br);
    set_rgb(cr, H.c.bg, 0.9 * a);
    cairo_fill_preserve(cr);
    set_rgb(cr, tint, a);
    cairo_set_line_width(cr, 1.4);
    cairo_stroke(cr);
    char num[4];
    snprintf(num, sizeof num, "%d", i + 1);
    set_rgb(cr, H.c.text, a);
    text_at(cr, num, 11 * s.scale, 1, bx, ly, 1);

    if (a < 0.3) return;
    /* its windows' apps */
    const struct hud_layer *L = &H.layer[i];
    double ix = bx + 22 * s.scale, isz = (15 + 7 * weight) * s.scale;  /* smaller on the sheets behind */
    for (int k = 0; k < L->nicons; k++) {
        GdkPixbuf *p = L->icons[k];
        int pw = gdk_pixbuf_get_width(p);
        if (pw <= 0) continue;
        cairo_save(cr);
        cairo_translate(cr, ix + k * (isz + 5 * s.scale), ly - isz / 2);
        cairo_scale(cr, isz / pw, isz / pw);
        gdk_cairo_set_source_pixbuf(cr, p, 0, 0);
        cairo_paint_with_alpha(cr, a);
        cairo_restore(cr);
    }
    double after = ix + L->nicons * (isz + 5 * s.scale);
    set_rgb(cr, H.c.muted, a);
    if (L->windows > L->nicons && L->nicons) {
        char more[16];
        snprintf(more, sizeof more, "+%d", L->windows - L->nicons);
        text_at(cr, more, 10 * s.scale, 0, after, ly, 0);
    } else if (!L->windows && weight > 0.5) {
        text_at(cr, "empty", 10 * s.scale, 0, ix, ly, 0);
    }
}

static gboolean draw(GtkWidget *w, cairo_t *cr, gpointer d)
{
    double t = now_s(), a = alpha_now(t), pos = pos_now(t);
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    cairo_set_source_rgba(cr, 0, 0, 0, 0);
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
    if (a <= 0) return TRUE;
    for (int i = STRATA_LAYERS - 1; i >= 0; i--) {  /* the deepest first */
        struct strata_sheet s = strata_sheet_at(i, pos);
        if (s.alpha <= 0.01) continue;
        draw_sheet(cr, i, s, s.alpha * a, fmax(0, 1 - fabs(i - pos)));
    }
    /* where you are: layer 1 at the bottom, 5 at the top */
    for (int i = 0; i < STRATA_LAYERS; i++) {
        strata_rgb tint = strata_tint(H.c.accent, i + 1);
        double y = BASE_Y + 4 - i * GAUGE_STEP;
        hex_path(cr, GAUGE_X, y, 8);
        if (i + 1 == H.current) {
            set_rgb(cr, tint, a);
            cairo_fill(cr);
        } else if (H.layer[i].windows) {
            set_rgb(cr, H.c.bg, 0.7 * a);
            cairo_fill_preserve(cr);
            set_rgb(cr, tint, 0.9 * a);
            cairo_set_line_width(cr, 1.5);
            cairo_stroke(cr);
        } else {
            set_rgb(cr, H.c.muted, 0.4 * a);
            cairo_set_line_width(cr, 1);
            cairo_stroke(cr);
        }
    }
    return TRUE;
}

/* ---------- window ---------- */

static void no_input(GtkWidget *w, gpointer d)
{
    cairo_region_t *r = cairo_region_create();  /* empty: every click goes through */
    gdk_window_input_shape_combine_region(gtk_widget_get_window(w), r, 0, 0);
    cairo_region_destroy(r);
}

static void hide(void)
{
    if (H.tick) g_source_remove(H.tick), H.tick = 0;
    if (H.win) gtk_widget_destroy(H.win);
    H.win = H.area = NULL;
    write_state();
}

static gboolean tick(gpointer d)
{
    double t = now_s(), end = H.changed_at + HOLD;
    H.tick = 0;
    if (alpha_now(t) <= 0 && t > end) {
        hide();
        return G_SOURCE_REMOVE;
    }
    gtk_widget_queue_draw(H.area);
    /* Frames only while something moves: the glide, the fades. Holding
     * still, sleep until the fade out starts. */
    double next = t + 0.016;
    int still = !H.animations || (t > H.glide_at + GLIDE && t > H.shown_at + FADE_IN);
    if (still && t < end) next = end + (H.animations ? 0 : FADE_OUT);
    H.tick = g_timeout_add((guint)fmax(1, (next - t) * 1000), tick, NULL);
    return G_SOURCE_REMOVE;
}

static void show(void)
{
    if (!H.win) {
        H.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
        gtk_window_set_title(GTK_WINDOW(H.win), "Strata");
        GdkVisual *v = gdk_screen_get_rgba_visual(gtk_widget_get_screen(H.win));
        if (v) gtk_widget_set_visual(H.win, v);
        gtk_widget_set_app_paintable(H.win, TRUE);
        gtk_layer_init_for_window(GTK_WINDOW(H.win));
        gtk_layer_set_namespace(GTK_WINDOW(H.win), "nexa-strata");
        gtk_layer_set_layer(GTK_WINDOW(H.win), GTK_LAYER_SHELL_LAYER_OVERLAY);
        gtk_layer_set_anchor(GTK_WINDOW(H.win), GTK_LAYER_SHELL_EDGE_BOTTOM, TRUE);
        gtk_layer_set_margin(GTK_WINDOW(H.win), GTK_LAYER_SHELL_EDGE_BOTTOM, HUD_MARGIN);
        gtk_layer_set_keyboard_mode(GTK_WINDOW(H.win), GTK_LAYER_SHELL_KEYBOARD_MODE_NONE);
        H.area = gtk_drawing_area_new();
        gtk_widget_set_size_request(H.area, HUD_W, HUD_H);
        g_signal_connect(H.area, "draw", G_CALLBACK(draw), NULL);
        g_signal_connect(H.win, "realize", G_CALLBACK(no_input), NULL);
        gtk_container_add(GTK_CONTAINER(H.win), H.area);
        gtk_widget_show_all(H.win);
        H.shown_at = now_s();
    }
    if (H.tick) g_source_remove(H.tick);  /* something moves again: frames from now */
    H.tick = g_timeout_add(16, tick, NULL);
    gtk_widget_queue_draw(H.area);
}

/* ---------- from Orbit ---------- */

void hud_reload_conf(void)
{
    char *text = NULL;
    g_file_get_contents(H.conf_path, &text, NULL, NULL);
    strata_conf_parse(&H.conf, text);
    g_free(text);
    if (!H.conf.show && H.win) hide();
    write_state();
}

void hud_init(const char *conf_path)
{
    snprintf(H.conf_path, sizeof H.conf_path, "%s", conf_path);
    hud_reload_conf();
}

void hud_set_colours(const struct hud_colours *c)
{
    H.c = *c;
    if (H.area) gtk_widget_queue_draw(H.area);
    write_state();
}

void hud_set_animations(int on)
{
    H.animations = on;
    write_state();
}

void hud_set_layers(const struct hud_layer layers[STRATA_LAYERS])
{
    for (int i = 0; i < STRATA_LAYERS; i++) {
        for (int k = 0; k < layers[i].nicons; k++) g_object_ref(layers[i].icons[k]);
        for (int k = 0; k < H.layer[i].nicons; k++) g_object_unref(H.layer[i].icons[k]);
        H.layer[i] = layers[i];
    }
    if (H.area) gtk_widget_queue_draw(H.area);
    write_state();
}

void hud_set_current(int ws)
{
    if (ws < 0) return;  /* the compositor didn't answer */
    int was = H.current;
    H.current = ws;
    if (!H.known) {
        H.known = 1;
        H.from = H.to = ws >= 1 && ws <= STRATA_LAYERS ? ws - 1 : 0;
        write_state();
        return;
    }
    if (ws == was) return;
    if (ws < 1 || ws > STRATA_LAYERS || !H.conf.show) {
        if (H.win) hide();
        write_state();
        return;
    }
    double t = now_s();
    H.from = H.win ? pos_now(t) : (was >= 1 && was <= STRATA_LAYERS ? was - 1 : ws - 1);
    H.to = ws - 1;
    H.glide_at = t;
    H.changed_at = t;
    show();
    write_state();
}
