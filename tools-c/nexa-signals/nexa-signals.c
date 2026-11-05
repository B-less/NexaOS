/* nexa-signals: NexaOS's notifications on screen, as tabs hanging from the
 * bar's bottom edge on the right (replaces mako's pop-ups). Design:
 * TODOnexa_identity.md, section "Signals".
 *
 * What to show comes from nexa-signald (Rust) over
 * $XDG_RUNTIME_DIR/nexa-signals.sock: a {"event": "state"} line on every
 * change. Clicks go back the same way (invoke, dismiss, hold). This program
 * never reads anything an app sent except through nexa-signald, which has
 * already cleaned it.
 *
 * One tab per app (its newest Signal, "+2" when it has more: a click on
 * the count shows them all), at most 4 apps, then a "+N more" tab that
 * opens Deck on Signals.
 *
 * NEXA_SIGNALS_STATE=<file>: write what is shown there as JSON lines (tests). */
#include <gio/gdesktopappinfo.h>
#include <gio/gio.h>
#include <gtk-layer-shell.h>
#include <gtk/gtk.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "json.h"

#define TAB_W 340
#define CHAMFER 14.0
#define MAX_SIG 64
#define MAX_APPS 4
#define SLIDE_MS 200

typedef struct { double r, g, b; } rgb;

struct sig {
    unsigned id;
    char app[80], desktop[140], icon[520], summary[260], body[700];
    char akey[4][72], alabel[4][48];
    int nact, has_default, critical;
    long left_ms, total_ms;  /* -1: no timer */
};

struct tab {
    unsigned id;
    GtkWidget *revealer, *frame, *app, *count, *summary, *body, *close, *actions, *icon;
    struct sig s;
    int count_n, seen, dying, icon_set;
    gint64 got_at;  /* when left_ms was read (us) */
    GtkWidget *act_btn[4];
};

static struct {
    GtkWidget *win, *column, *more, *more_label;
    struct tab tabs[MAX_SIG];
    int ntabs, held, quiet, animations, more_n;
    struct sig sigs[MAX_SIG];
    int nsig;
    char expanded[80];  /* the app whose Signals are all shown */
    struct { rgb surface, text, muted, accent, warn; double opacity; } col;
    GSocketConnection *conn;
    GDataInputStream *in;
    GOutputStream *out;
    guint tick, retry;
    char theme_path[600], prefs_path[600];
    int bottom;  /* Settings > Notifications: at the bottom edge instead of under the bar */
    char position[32];
} S;

static void write_state(void);
static void connect_service(void);

/* ---------- theme ---------- */

static rgb hexcol(const char *s, rgb fb)
{
    unsigned v;
    if (!s || sscanf(s, "#%6x", &v) != 1) return fb;
    return (rgb){((v >> 16) & 255) / 255.0, ((v >> 8) & 255) / 255.0, (v & 255) / 255.0};
}

static char *css_rgb(rgb c, double a)
{
    return g_strdup_printf("rgba(%d,%d,%d,%.2f)", (int)lround(c.r * 255), (int)lround(c.g * 255), (int)lround(c.b * 255), a);
}

static void load_theme(void)
{
    S.col.surface = hexcol("#141c3a", S.col.surface);
    S.col.text = hexcol("#e6ecff", S.col.text);
    S.col.muted = hexcol("#8f9bc4", S.col.muted);
    S.col.accent = hexcol("#4da3ff", S.col.accent);
    S.col.warn = hexcol("#ffb547", S.col.warn);
    S.col.opacity = 0.9;
    char *text = NULL;
    if (g_file_get_contents(S.theme_path, &text, NULL, NULL)) {
        char **lines = g_strsplit(text, "\n", -1);
        for (char **l = lines; *l; l++) {
            char *eq = strchr(*l, '=');
            if (!eq || **l == '#') continue;
            *eq = 0;
            const char *k = *l, *v = eq + 1;
            if (!strcmp(k, "surface")) S.col.surface = hexcol(v, S.col.surface);
            else if (!strcmp(k, "text")) S.col.text = hexcol(v, S.col.text);
            else if (!strcmp(k, "muted")) S.col.muted = hexcol(v, S.col.muted);
            else if (!strcmp(k, "accent")) S.col.accent = hexcol(v, S.col.accent);
            else if (!strcmp(k, "warn")) S.col.warn = hexcol(v, S.col.warn);
            else if (!strcmp(k, "opacity")) S.col.opacity = g_ascii_strtod(v, NULL);
            else if (!strcmp(k, "icon_theme")) g_object_set(gtk_settings_get_default(), "gtk-icon-theme-name", v, NULL);
        }
        g_strfreev(lines);
        g_free(text);
    }
    char *tx = css_rgb(S.col.text, 1), *mu = css_rgb(S.col.muted, 1), *ac = css_rgb(S.col.accent, 1), *soft = css_rgb(S.col.accent, 0.16),
         *field = css_rgb(S.col.text, 0.10);
    char *css = g_strdup_printf(
        "window { background: transparent; }\n"
        ".sig label { color: %s; font-family: Inter, sans-serif; }\n"
        ".sig image { color: %s; }\n"
        ".sig label.sig-app { color: %s; font-size: 8.5pt; font-weight: 600; }\n"
        ".sig label.sig-summary { font-weight: 600; font-size: 10.5pt; }\n"
        ".sig label.sig-body { color: %s; font-size: 9.5pt; }\n"
        ".sig button { background: transparent; background-image: none; border: none; box-shadow: none; color: %s; padding: 2px 8px; "
        "border-radius: 9px; min-height: 0; min-width: 0; }\n"
        ".sig button:hover { background: %s; }\n"
        ".sig button.sig-pill { background: %s; padding: 3px 11px; }\n"
        ".sig button.sig-pill:hover { background: %s; }\n"
        ".sig button.sig-count { color: %s; font-size: 8.5pt; font-weight: 600; padding: 0 6px; }\n",
        tx, tx, ac, mu, tx, soft, field, soft, ac);
    static GtkCssProvider *p;
    if (!p) {
        p = gtk_css_provider_new();
        gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(p), GTK_STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    }
    gtk_css_provider_load_from_data(p, css, -1, NULL);
    g_free(css);
    g_free(tx);
    g_free(mu);
    g_free(ac);
    g_free(soft);
    g_free(field);
    if (S.win) gtk_widget_queue_draw(S.win);
}

/* ---------- talking to nexa-signald ---------- */

static void send_line(const char *line)
{
    if (!S.out) return;
    g_output_stream_write_all(S.out, line, strlen(line), NULL, NULL, NULL);  /* short lines; the socket is local */
}

static void send_cmd(const char *cmd, unsigned id, const char *action)
{
    char line[200];
    if (action) snprintf(line, sizeof line, "{\"cmd\": \"%s\", \"id\": %u, \"action\": \"%s\"}\n", cmd, id, action);
    else if (id) snprintf(line, sizeof line, "{\"cmd\": \"%s\", \"id\": %u}\n", cmd, id);
    else snprintf(line, sizeof line, "{\"cmd\": \"%s\"}\n", cmd);
    send_line(line);
}

static void send_hold(int on)
{
    if (S.held == on) return;
    S.held = on;
    send_line(on ? "{\"cmd\": \"hold\", \"on\": true}\n" : "{\"cmd\": \"hold\", \"on\": false}\n");
}

/* ---------- drawing a tab ---------- */

/* Flat where it meets the bar, chamfered at the bottom (at the bottom of
 * the screen: the other way up). */
static void tab_path(cairo_t *cr, double w, double h)
{
    double c = fmin(CHAMFER, h / 2);
    cairo_save(cr);
    if (S.bottom) {
        cairo_translate(cr, 0, h);
        cairo_scale(cr, 1, -1);
    }
    cairo_new_path(cr);
    cairo_move_to(cr, 0.5, 0);
    cairo_line_to(cr, w - 0.5, 0);
    cairo_line_to(cr, w - 0.5, h - c);
    cairo_line_to(cr, w - c, h - 0.5);
    cairo_line_to(cr, c, h - 0.5);
    cairo_line_to(cr, 0.5, h - c);
    cairo_close_path(cr);
    cairo_restore(cr);
}

static double left_fraction(const struct tab *t)
{
    if (t->s.left_ms < 0 || t->s.total_ms <= 0) return -1;
    double left = t->s.left_ms;
    if (!S.held) left -= (g_get_monotonic_time() - t->got_at) / 1000.0;
    return fmax(0, fmin(1, left / t->s.total_ms));
}

static gboolean draw_tab(GtkWidget *w, cairo_t *cr, gpointer data)
{
    struct tab *t = data;
    double W = gtk_widget_get_allocated_width(w), H = gtk_widget_get_allocated_height(w);
    tab_path(cr, W, H);
    cairo_set_source_rgba(cr, S.col.surface.r, S.col.surface.g, S.col.surface.b, fmax(S.col.opacity, 0.92));
    cairo_fill_preserve(cr);
    rgb edge = t->s.critical ? S.col.warn : S.col.accent;
    cairo_set_source_rgba(cr, edge.r, edge.g, edge.b, t->s.critical ? 0.9 : 0.25);
    cairo_set_line_width(cr, t->s.critical ? 2 : 1);
    cairo_stroke(cr);
    /* the time left: a line along the bottom, shrinking toward the middle */
    double f = left_fraction(t);
    if (f >= 0) {
        double c = fmin(CHAMFER, H / 2), span = (W - 2 * c) * f, y = S.bottom ? 1.5 : H - 1.5;
        cairo_move_to(cr, W / 2 - span / 2, y);
        cairo_line_to(cr, W / 2 + span / 2, y);
        cairo_set_source_rgba(cr, S.col.accent.r, S.col.accent.g, S.col.accent.b, 0.9);
        cairo_set_line_width(cr, 2);
        cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
        cairo_stroke(cr);
    }
    return FALSE;
}

static gboolean tick(gpointer d)
{
    int timers = 0;
    for (int i = 0; i < S.ntabs; i++)
        if (!S.tabs[i].dying && S.tabs[i].s.left_ms >= 0) {
            timers = 1;
            gtk_widget_queue_draw(S.tabs[i].frame);
        }
    if (!timers || S.held) {
        S.tick = 0;
        return G_SOURCE_REMOVE;
    }
    return G_SOURCE_CONTINUE;
}

static void start_tick(void)
{
    /* 20 frames a second while a line runs; twice a second with reduced motion */
    if (!S.tick && !S.held) S.tick = g_timeout_add(S.animations ? 50 : 500, tick, NULL);
}

/* ---------- tabs ---------- */

static struct tab *find_tab(unsigned id)
{
    for (int i = 0; i < S.ntabs; i++)
        if (S.tabs[i].id == id && !S.tabs[i].dying) return &S.tabs[i];
    return NULL;
}

static void set_text(GtkWidget *l, const char *text)
{
    if (g_utf8_validate(text, -1, NULL)) {
        gtk_label_set_text(GTK_LABEL(l), text);
    } else {
        char *ok = g_utf8_make_valid(text, -1);
        gtk_label_set_text(GTK_LABEL(l), ok);
        g_free(ok);
    }
}

static GtkWidget *label(const char *cls, int lines)
{
    GtkWidget *l = gtk_label_new(NULL);
    gtk_label_set_xalign(GTK_LABEL(l), 0);
    gtk_label_set_ellipsize(GTK_LABEL(l), PANGO_ELLIPSIZE_END);
    if (lines > 1) {
        gtk_label_set_line_wrap(GTK_LABEL(l), TRUE);
        gtk_label_set_line_wrap_mode(GTK_LABEL(l), PANGO_WRAP_WORD_CHAR);
        gtk_label_set_lines(GTK_LABEL(l), lines);
    }
    gtk_label_set_max_width_chars(GTK_LABEL(l), 30);
    gtk_style_context_add_class(gtk_widget_get_style_context(l), cls);
    return l;
}

static void on_close(GtkButton *b, gpointer data) { send_cmd("dismiss", GPOINTER_TO_UINT(data), NULL); }

static void on_action(GtkButton *b, gpointer data)
{
    unsigned id = GPOINTER_TO_UINT(data);
    struct tab *t = find_tab(id);
    const char *key = g_object_get_data(G_OBJECT(b), "key");
    if (t && key) send_cmd("invoke", id, key);
}

static void refresh_view(void);

static void on_count(GtkButton *b, gpointer data)
{
    struct tab *t = find_tab(GPOINTER_TO_UINT(data));
    if (!t) return;
    if (!strcmp(S.expanded, t->s.app)) S.expanded[0] = 0;
    else snprintf(S.expanded, sizeof S.expanded, "%s", t->s.app);
    refresh_view();
}

/* A click on the tab itself: its default action, or just dismiss. */
static gboolean on_tab_click(GtkWidget *w, GdkEventButton *e, gpointer data)
{
    unsigned id = GPOINTER_TO_UINT(data);
    if (e->type != GDK_BUTTON_PRESS) return TRUE;
    if (e->button == 1) send_cmd("invoke", id, "default");
    else send_cmd("dismiss", id, NULL);  /* middle or right: away */
    return TRUE;
}

static GIcon *icon_for(const struct sig *s)
{
    if (s->desktop[0]) {
        char name[160];
        snprintf(name, sizeof name, "%s.desktop", s->desktop);
        GDesktopAppInfo *a = g_desktop_app_info_new(name);
        if (a) {
            GIcon *i = g_app_info_get_icon(G_APP_INFO(a));
            if (i) g_object_ref(i);
            g_object_unref(a);
            if (i) return i;
        }
    }
    if (s->icon[0] == '/' && g_file_test(s->icon, G_FILE_TEST_IS_REGULAR)) {
        GFile *f = g_file_new_for_path(s->icon);
        GIcon *i = g_file_icon_new(f);
        g_object_unref(f);
        return i;
    }
    if (s->icon[0] && !strchr(s->icon, '/')) return g_themed_icon_new_with_default_fallbacks(s->icon);
    return g_themed_icon_new(s->critical ? "dialog-warning-symbolic" : "preferences-system-notifications-symbolic");
}

static void fill_tab(struct tab *t, const struct sig *s)
{
    int same_actions = t->s.nact == s->nact;
    for (int k = 0; same_actions && k < s->nact; k++) same_actions = !strcmp(t->s.akey[k], s->akey[k]) && !strcmp(t->s.alabel[k], s->alabel[k]);
    int new_icon = !t->icon_set || strcmp(t->s.desktop, s->desktop) || strcmp(t->s.icon, s->icon);
    t->s = *s;
    t->got_at = g_get_monotonic_time();
    set_text(t->app, s->app[0] ? s->app : "Signal");
    set_text(t->summary, s->summary);
    set_text(t->body, s->body);
    gtk_widget_set_visible(t->body, s->body[0] != 0);
    if (new_icon) {
        GIcon *gi = icon_for(s);
        gtk_image_set_from_gicon(GTK_IMAGE(t->icon), gi, GTK_ICON_SIZE_DND);
        gtk_image_set_pixel_size(GTK_IMAGE(t->icon), 28);
        g_object_unref(gi);
        t->icon_set = 1;
    }
    if (!same_actions || !gtk_container_get_children(GTK_CONTAINER(t->actions))) {
        GList *kids = gtk_container_get_children(GTK_CONTAINER(t->actions));
        for (GList *l = kids; l; l = l->next) gtk_widget_destroy(l->data);
        g_list_free(kids);
        memset(t->act_btn, 0, sizeof t->act_btn);
        int shown = 0;
        for (int k = 0; k < s->nact; k++) {
            if (!strcmp(s->akey[k], "default")) continue;  /* that's a click on the tab */
            GtkWidget *b = gtk_button_new_with_label(s->alabel[k]);
            gtk_style_context_add_class(gtk_widget_get_style_context(b), "sig-pill");
            g_object_set_data_full(G_OBJECT(b), "key", g_strdup(s->akey[k]), g_free);
            g_signal_connect(b, "clicked", G_CALLBACK(on_action), GUINT_TO_POINTER(t->id));
            gtk_box_pack_start(GTK_BOX(t->actions), b, FALSE, FALSE, 0);
            t->act_btn[k] = b;
            shown++;
        }
        gtk_widget_show_all(t->actions);
        gtk_widget_set_visible(t->actions, shown > 0);
    }
    gtk_widget_queue_draw(t->frame);
}

static struct tab *new_tab(const struct sig *s)
{
    if (S.ntabs >= MAX_SIG) return NULL;
    struct tab *t = &S.tabs[S.ntabs++];
    memset(t, 0, sizeof *t);
    t->id = s->id;
    t->frame = gtk_event_box_new();
    gtk_widget_set_app_paintable(t->frame, TRUE);
    gtk_style_context_add_class(gtk_widget_get_style_context(t->frame), "sig");
    gtk_widget_set_size_request(t->frame, TAB_W, -1);
    g_signal_connect(t->frame, "draw", G_CALLBACK(draw_tab), t);
    g_signal_connect(t->frame, "button-press-event", G_CALLBACK(on_tab_click), GUINT_TO_POINTER(s->id));

    GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 10);
    g_object_set(row, "margin-start", 14, "margin-end", 8, "margin-top", S.bottom ? 12 : 8, "margin-bottom", S.bottom ? 8 : 12, NULL);
    gtk_container_add(GTK_CONTAINER(t->frame), row);
    t->icon = gtk_image_new();
    gtk_widget_set_valign(t->icon, GTK_ALIGN_START);
    gtk_widget_set_margin_top(t->icon, 4);
    gtk_box_pack_start(GTK_BOX(row), t->icon, FALSE, FALSE, 0);
    GtkWidget *texts = gtk_box_new(GTK_ORIENTATION_VERTICAL, 1);
    gtk_box_pack_start(GTK_BOX(row), texts, TRUE, TRUE, 0);
    GtkWidget *top = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    t->app = label("sig-app", 1);
    gtk_box_pack_start(GTK_BOX(top), t->app, FALSE, FALSE, 0);
    t->count = gtk_button_new_with_label("");
    gtk_style_context_add_class(gtk_widget_get_style_context(t->count), "sig-count");
    gtk_widget_set_tooltip_text(t->count, "Show them all");
    g_signal_connect(t->count, "clicked", G_CALLBACK(on_count), GUINT_TO_POINTER(s->id));
    gtk_box_pack_start(GTK_BOX(top), t->count, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(texts), top, FALSE, FALSE, 0);
    t->summary = label("sig-summary", 2);
    t->body = label("sig-body", 2);
    gtk_box_pack_start(GTK_BOX(texts), t->summary, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(texts), t->body, FALSE, FALSE, 0);
    t->actions = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    gtk_widget_set_margin_top(t->actions, 6);
    gtk_box_pack_start(GTK_BOX(texts), t->actions, FALSE, FALSE, 0);
    t->close = gtk_button_new_with_label("\xc3\x97");  /* × */
    gtk_widget_set_tooltip_text(t->close, "Dismiss");
    atk_object_set_name(gtk_widget_get_accessible(t->close), "Dismiss");
    gtk_widget_set_valign(t->close, GTK_ALIGN_START);
    g_signal_connect(t->close, "clicked", G_CALLBACK(on_close), GUINT_TO_POINTER(s->id));
    gtk_box_pack_end(GTK_BOX(row), t->close, FALSE, FALSE, 0);

    t->revealer = gtk_revealer_new();
    gtk_revealer_set_transition_type(GTK_REVEALER(t->revealer),
                                     S.bottom ? GTK_REVEALER_TRANSITION_TYPE_SLIDE_UP : GTK_REVEALER_TRANSITION_TYPE_SLIDE_DOWN);
    gtk_revealer_set_transition_duration(GTK_REVEALER(t->revealer), S.animations ? SLIDE_MS : 0);
    gtk_container_add(GTK_CONTAINER(t->revealer), t->frame);
    gtk_box_pack_start(GTK_BOX(S.column), t->revealer, FALSE, FALSE, 0);
    gtk_widget_show_all(t->revealer);
    fill_tab(t, s);
    return t;
}

static gboolean remove_tab_later(gpointer data)
{
    GtkWidget *rev = data;
    for (int i = 0; i < S.ntabs; i++)
        if (S.tabs[i].revealer == rev) {
            gtk_widget_destroy(rev);
            S.tabs[i] = S.tabs[--S.ntabs];
            break;
        }
    g_object_unref(rev);
    write_state();
    return G_SOURCE_REMOVE;
}

/* The tab slides back up into the bar, then goes. */
static void retire(struct tab *t)
{
    t->dying = 1;
    gtk_revealer_set_transition_duration(GTK_REVEALER(t->revealer), S.animations ? 150 : 0);
    gtk_revealer_set_reveal_child(GTK_REVEALER(t->revealer), FALSE);
    g_timeout_add(S.animations ? 170 : 0, remove_tab_later, g_object_ref(t->revealer));
}

static void set_input_region(void)
{
    if (!gtk_widget_get_realized(S.win)) return;
    cairo_region_t *reg = cairo_region_create();
    for (int i = 0; i < S.ntabs; i++) {
        struct tab *t = &S.tabs[i];
        int x, y;
        if (t->dying || !gtk_widget_get_mapped(t->frame) || !gtk_widget_translate_coordinates(t->frame, S.win, 0, 0, &x, &y)) continue;
        cairo_rectangle_int_t r = {x, y, gtk_widget_get_allocated_width(t->frame), gtk_widget_get_allocated_height(t->frame)};
        cairo_region_union_rectangle(reg, &r);
    }
    if (gtk_widget_get_visible(S.more)) {
        int x, y;
        if (gtk_widget_translate_coordinates(S.more, S.win, 0, 0, &x, &y)) {
            cairo_rectangle_int_t r = {x, y, gtk_widget_get_allocated_width(S.more), gtk_widget_get_allocated_height(S.more)};
            cairo_region_union_rectangle(reg, &r);
        }
    }
    gtk_widget_input_shape_combine_region(S.win, reg);
    cairo_region_destroy(reg);
}

static int cmp_sig_newest(const void *a, const void *b)
{
    unsigned x = ((const struct sig *)a)->id, y = ((const struct sig *)b)->id;
    return x < y ? 1 : x > y ? -1 : 0;
}

/* From the Signals to the tabs: one per app (all of one app if expanded),
 * newest first, at most MAX_APPS apps, kept in place where they already are. */
static void refresh_view(void)
{
    static struct sig order[MAX_SIG];
    int counts[MAX_SIG] = {0}, n = 0, apps = 0, hidden = 0;
    memcpy(order, S.sigs, sizeof(struct sig) * (size_t)S.nsig);
    qsort(order, (size_t)S.nsig, sizeof *order, cmp_sig_newest);
    static struct sig want[MAX_SIG];
    for (int i = 0; i < S.nsig; i++) {
        int first = 1;
        for (int j = 0; j < i; j++) first &= strcmp(order[j].app, order[i].app) != 0;
        int expanded = S.expanded[0] && !strcmp(S.expanded, order[i].app);
        if (!first && !expanded) continue;
        if (!first && apps > MAX_APPS) continue;
        int c = 0;
        for (int j = 0; j < S.nsig; j++) c += !strcmp(order[j].app, order[i].app);
        if (first && ++apps > MAX_APPS) {
            hidden += c;  /* every Signal of the apps that don't fit */
            continue;
        }
        counts[n] = first ? c : 0;
        want[n++] = order[i];
    }
    /* gone: slide away */
    for (int i = 0; i < S.ntabs; i++) {
        struct tab *t = &S.tabs[i];
        if (t->dying) continue;
        int keep = 0;
        for (int k = 0; k < n; k++) keep |= want[k].id == t->id;
        if (!keep) retire(t);
    }
    /* new ones slide in; the rest are updated where they are */
    for (int k = 0; k < n; k++) {
        struct tab *t = find_tab(want[k].id);
        if (!t) {
            t = new_tab(&want[k]);
            if (!t) continue;
            gtk_revealer_set_reveal_child(GTK_REVEALER(t->revealer), TRUE);
        } else {
            fill_tab(t, &want[k]);
        }
        t->count_n = counts[k];
        if (counts[k] > 1) {
            char c[24];
            snprintf(c, sizeof c, !strcmp(S.expanded, want[k].app) ? "show less" : "+%d", counts[k] - 1);
            gtk_button_set_label(GTK_BUTTON(t->count), c);
            gtk_widget_show(t->count);
        } else {
            gtk_widget_hide(t->count);
        }
        /* newest nearest the edge they hang from */
        gtk_box_reorder_child(GTK_BOX(S.column), t->revealer, S.bottom ? n - 1 - k : k);
    }
    S.more_n = hidden;
    if (hidden) {
        char text[64];
        snprintf(text, sizeof text, "+%d more in Deck", hidden);
        gtk_button_set_label(GTK_BUTTON(S.more), text);
        gtk_widget_show(S.more);
        gtk_box_reorder_child(GTK_BOX(S.column), S.more, S.bottom ? 0 : -1);
    } else {
        gtk_widget_hide(S.more);
    }
    if (n == 0 && S.held) send_hold(0);  /* nothing left under the pointer */
    start_tick();
    write_state();
}

static void on_more(GtkButton *b, gpointer d)
{
    const char *argv[] = {"nexa-deck", "signals", NULL};
    g_spawn_async(NULL, (gchar **)argv, NULL, G_SPAWN_SEARCH_PATH | G_SPAWN_STDOUT_TO_DEV_NULL | G_SPAWN_STDERR_TO_DEV_NULL, NULL, NULL, NULL,
                  NULL);
}

/* ---------- reading the state ---------- */

static void copy_str(const struct jdoc *d, int o, const char *key, char *out, size_t cap)
{
    snprintf(out, cap, "%s", json_str(d, o, key, ""));
}

static void apply_state(char *line)
{
    struct jdoc d = {0};
    if (json_parse(&d, line) != 0 || strcmp(json_str(&d, 0, "event", ""), "state")) {
        json_free(&d);
        return;  /* a reply to one of our requests */
    }
    S.quiet = json_num(&d, 0, "quiet", 0) != 0;
    S.nsig = 0;
    int list = json_get(&d, 0, "signals");
    for (int e = list >= 0 ? d.n[list].child : -1; e >= 0 && S.nsig < MAX_SIG; e = d.n[e].next) {
        struct sig *s = &S.sigs[S.nsig++];
        memset(s, 0, sizeof *s);
        s->id = (unsigned)json_num(&d, e, "id", 0);
        copy_str(&d, e, "app", s->app, sizeof s->app);
        copy_str(&d, e, "desktop", s->desktop, sizeof s->desktop);
        copy_str(&d, e, "icon", s->icon, sizeof s->icon);
        copy_str(&d, e, "summary", s->summary, sizeof s->summary);
        copy_str(&d, e, "body", s->body, sizeof s->body);
        s->critical = json_num(&d, e, "critical", 0) != 0;
        s->left_ms = (long)json_num(&d, e, "left_ms", -1);
        s->total_ms = (long)json_num(&d, e, "total_ms", -1);
        int acts = json_get(&d, e, "actions");
        for (int a = acts >= 0 ? d.n[acts].child : -1; a >= 0 && s->nact < 4; a = d.n[a].next) {
            int k = d.n[a].child, l = k >= 0 ? d.n[k].next : -1;
            if (k < 0 || l < 0 || d.n[k].type != J_STR || d.n[l].type != J_STR) continue;
            snprintf(s->akey[s->nact], sizeof s->akey[0], "%s", d.n[k].str);
            snprintf(s->alabel[s->nact], sizeof s->alabel[0], "%s", d.n[l].str);
            s->has_default |= !strcmp(d.n[k].str, "default");
            s->nact++;
        }
    }
    json_free(&d);
    if (S.expanded[0]) {  /* that app has nothing left: stop showing it opened */
        int any = 0;
        for (int i = 0; i < S.nsig; i++) any |= !strcmp(S.sigs[i].app, S.expanded);
        if (!any) S.expanded[0] = 0;
    }
    refresh_view();
}

static gboolean retry_cb(gpointer d)
{
    S.retry = 0;
    connect_service();
    return G_SOURCE_REMOVE;
}

static void lost_service(void)
{
    if (S.conn) g_object_unref(S.conn);
    if (S.in) g_object_unref(S.in);
    S.conn = NULL;
    S.in = NULL;
    S.out = NULL;
    S.held = 0;
    S.nsig = 0;
    refresh_view();
    if (!S.retry) S.retry = g_timeout_add_seconds(2, retry_cb, NULL);  /* it may be starting, or restarting */
}

static void read_line_done(GObject *src, GAsyncResult *res, gpointer data)
{
    gsize len = 0;
    char *line = g_data_input_stream_read_line_finish(G_DATA_INPUT_STREAM(src), res, &len, NULL);
    if (!line) {
        lost_service();
        return;
    }
    if (len < (1u << 20)) apply_state(line);
    g_free(line);
    g_data_input_stream_read_line_async(S.in, G_PRIORITY_DEFAULT, NULL, read_line_done, NULL);
}

static void connect_service(void)
{
    const char *p = g_getenv("NEXA_SIGNALS_SOCKET");
    char *path = p && *p ? g_strdup(p) : g_build_filename(g_get_user_runtime_dir(), "nexa-signals.sock", NULL);
    GSocketClient *c = g_socket_client_new();
    GSocketAddress *addr = g_unix_socket_address_new(path);
    S.conn = g_socket_client_connect(c, G_SOCKET_CONNECTABLE(addr), NULL, NULL);
    g_object_unref(addr);
    g_object_unref(c);
    g_free(path);
    if (!S.conn) {
        lost_service();
        return;
    }
    S.out = g_io_stream_get_output_stream(G_IO_STREAM(S.conn));
    S.in = g_data_input_stream_new(g_io_stream_get_input_stream(G_IO_STREAM(S.conn)));
    send_line("{\"cmd\": \"subscribe\"}\n");
    g_data_input_stream_read_line_async(S.in, G_PRIORITY_DEFAULT, NULL, read_line_done, NULL);
}

/* ---------- state for tests ---------- */

static void rect_of(GString *s, const char *name, GtkWidget *w)
{
    int x, y;
    if (!w || !gtk_widget_get_mapped(w) || !gtk_widget_translate_coordinates(w, S.win, 0, 0, &x, &y)) return;
    g_string_append_printf(s, ", \"%s\": [%d, %d, %d, %d]", name, x, y, gtk_widget_get_allocated_width(w), gtk_widget_get_allocated_height(w));
}

static void write_state(void)
{
    const char *path = g_getenv("NEXA_SIGNALS_STATE");
    if (!path || !S.win) return;
    GString *s = g_string_new(NULL);
    g_string_append_printf(s, "{\"connected\": %d, \"held\": %d, \"quiet\": %d, \"signals\": %d, \"more\": %d, \"expanded\": \"%s\", "
                           "\"position\": \"%s\", \"window\": [%d, %d]",
                           S.conn != NULL, S.held, S.quiet, S.nsig, S.more_n, S.expanded[0] ? "yes" : "", S.position, gtk_widget_get_allocated_width(S.win),
                           gtk_widget_get_allocated_height(S.win));
    rect_of(s, "more_rect", gtk_widget_get_visible(S.more) ? S.more : NULL);
    g_string_append(s, "}\n");
    for (int i = 0; i < S.ntabs; i++) {
        struct tab *t = &S.tabs[i];
        if (t->dying) continue;
        g_string_append_printf(s, "{\"id\": %u, \"count\": %d, \"critical\": %d, \"actions\": %d, \"icon\": %d", t->id, t->count_n, t->s.critical,
                               t->s.nact, gtk_image_get_storage_type(GTK_IMAGE(t->icon)) != GTK_IMAGE_EMPTY);
        rect_of(s, "rect", t->frame);
        rect_of(s, "close", t->close);
        rect_of(s, "count_rect", gtk_widget_get_visible(t->count) ? t->count : NULL);
        for (int k = 0; k < 4; k++) {
            char nm[16];
            snprintf(nm, sizeof nm, "action%d", k);
            rect_of(s, nm, t->act_btn[k]);
        }
        g_string_append(s, "}\n");
    }
    char *tmp = g_strconcat(path, ".tmp", NULL);
    if (g_file_set_contents(tmp, s->str, (gssize)s->len, NULL)) rename(tmp, path);
    g_free(tmp);
    g_string_free(s, TRUE);
}

/* ---------- start ---------- */

static guint layout_idle;
static gboolean layout_now(gpointer d)
{
    layout_idle = 0;
    set_input_region();
    write_state();
    return G_SOURCE_REMOVE;
}
static void on_layout(GtkWidget *w, GdkRectangle *a, gpointer d)
{
    if (!layout_idle) layout_idle = g_idle_add(layout_now, NULL);
}

/* The pointer on the Signals: their timers wait. */
static gboolean on_enter(GtkWidget *w, GdkEventCrossing *e, gpointer d)
{
    send_hold(1);
    write_state();
    return FALSE;
}
static gboolean on_leave(GtkWidget *w, GdkEventCrossing *e, gpointer d)
{
    if (e->detail == GDK_NOTIFY_INFERIOR) return FALSE;
    send_hold(0);
    start_tick();
    write_state();
    return FALSE;
}

/* Where they appear (Settings > Notifications): top-right (default),
 * top-center, top-left, bottom-right, bottom-center, bottom-left. */
static void load_position(void)
{
    char *text = NULL, pos[32] = "top-right";
    if (g_file_get_contents(S.prefs_path, &text, NULL, NULL)) {
        struct jdoc d = {0};
        if (json_parse(&d, text) == 0) snprintf(pos, sizeof pos, "%s", json_str(&d, 0, "position", "top-right"));
        json_free(&d);
        g_free(text);
    }
    snprintf(S.position, sizeof S.position, "%s", pos);
    int bottom = g_str_has_prefix(pos, "bottom-"), left = g_str_has_suffix(pos, "-left"), centre = g_str_has_suffix(pos, "-center");
    GtkWindow *w = GTK_WINDOW(S.win);
    gtk_layer_set_anchor(w, GTK_LAYER_SHELL_EDGE_TOP, !bottom);
    gtk_layer_set_anchor(w, GTK_LAYER_SHELL_EDGE_BOTTOM, bottom);
    gtk_layer_set_anchor(w, GTK_LAYER_SHELL_EDGE_LEFT, left);
    gtk_layer_set_anchor(w, GTK_LAYER_SHELL_EDGE_RIGHT, !left && !centre);
    gtk_layer_set_margin(w, GTK_LAYER_SHELL_EDGE_LEFT, left ? 16 : 0);
    gtk_layer_set_margin(w, GTK_LAYER_SHELL_EDGE_RIGHT, !left && !centre ? 16 : 0);
    if (bottom != S.bottom) {  /* the other way up: tabs already there are rebuilt */
        S.bottom = bottom;
        for (int i = 0; i < S.ntabs; i++)
            if (!S.tabs[i].dying) retire(&S.tabs[i]);
        refresh_view();
    }
    gtk_box_set_child_packing(GTK_BOX(gtk_widget_get_parent(S.column)), S.column, FALSE, FALSE, 0, bottom ? GTK_PACK_END : GTK_PACK_START);
}

static void on_prefs_file(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent ev, gpointer d)
{
    if (ev == G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT || ev == G_FILE_MONITOR_EVENT_CREATED || ev == G_FILE_MONITOR_EVENT_RENAMED ||
        ev == G_FILE_MONITOR_EVENT_MOVED_IN || ev == G_FILE_MONITOR_EVENT_DELETED)
        load_position();
}

static void on_theme_file(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent ev, gpointer d)
{
    if (ev == G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT || ev == G_FILE_MONITOR_EVENT_CREATED || ev == G_FILE_MONITOR_EVENT_RENAMED ||
        ev == G_FILE_MONITOR_EVENT_MOVED_IN)
        load_theme();
}

int main(int argc, char **argv)
{
    if (argc > 1) {
        fprintf(stderr, "usage: nexa-signals   (shows what nexa-signald says)\n");
        return 2;
    }
    gtk_init(&argc, &argv);
    if (!gtk_layer_is_supported()) {
        fprintf(stderr, "nexa-signals: this session has no layer-shell (needs Sway or Hyprland)\n");
        return 1;
    }
    const char *cfg = g_getenv("XDG_CONFIG_HOME");
    if (cfg && *cfg) snprintf(S.theme_path, sizeof S.theme_path, "%s/nexa/lock.conf", cfg);
    else snprintf(S.theme_path, sizeof S.theme_path, "%s/.config/nexa/lock.conf", g_get_home_dir());
    if (cfg && *cfg) snprintf(S.prefs_path, sizeof S.prefs_path, "%s/nexa/notifications.json", cfg);
    else snprintf(S.prefs_path, sizeof S.prefs_path, "%s/.config/nexa/notifications.json", g_get_home_dir());
    load_theme();
    gboolean anim = TRUE;
    g_object_get(gtk_settings_get_default(), "gtk-enable-animations", &anim, NULL);
    S.animations = anim;

    S.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(S.win), "Signals");
    GdkVisual *v = gdk_screen_get_rgba_visual(gtk_widget_get_screen(S.win));
    if (v) gtk_widget_set_visual(S.win, v);
    gtk_widget_set_app_paintable(S.win, TRUE);
    gtk_layer_init_for_window(GTK_WINDOW(S.win));
    gtk_layer_set_namespace(GTK_WINDOW(S.win), "nexa-signals");
    gtk_layer_set_layer(GTK_WINDOW(S.win), GTK_LAYER_SHELL_LAYER_TOP);
    gtk_layer_set_keyboard_mode(GTK_WINDOW(S.win), GTK_LAYER_SHELL_KEYBOARD_MODE_NONE);
    gtk_layer_set_exclusive_zone(GTK_WINDOW(S.win), 0);  /* right under the bar */

    gtk_widget_add_events(S.win, GDK_ENTER_NOTIFY_MASK | GDK_LEAVE_NOTIFY_MASK);
    g_signal_connect(S.win, "enter-notify-event", G_CALLBACK(on_enter), NULL);
    g_signal_connect(S.win, "leave-notify-event", G_CALLBACK(on_leave), NULL);
    g_signal_connect(S.win, "destroy", G_CALLBACK(gtk_main_quit), NULL);

    S.column = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    gtk_widget_set_size_request(S.column, TAB_W, 1);
    g_signal_connect(S.column, "size-allocate", G_CALLBACK(on_layout), NULL);
    /* the column sits at the top of its box, or at the bottom when the
     * Signals are at the bottom edge */
    GtkWidget *outer = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_box_pack_start(GTK_BOX(outer), S.column, FALSE, FALSE, 0);
    gtk_container_add(GTK_CONTAINER(S.win), outer);
    S.more = gtk_button_new_with_label("");
    gtk_style_context_add_class(gtk_widget_get_style_context(S.more), "sig-pill");
    GtkWidget *more_wrap = S.more;
    gtk_widget_set_halign(more_wrap, GTK_ALIGN_END);
    g_signal_connect(S.more, "clicked", G_CALLBACK(on_more), NULL);
    gtk_box_pack_start(GTK_BOX(S.column), S.more, FALSE, FALSE, 0);
    gtk_widget_set_no_show_all(S.more, TRUE);
    /* the class for the pill's colours lives on .sig */
    gtk_style_context_add_class(gtk_widget_get_style_context(S.column), "sig");
    load_position();
    gtk_widget_show_all(S.win);
    set_input_region();
    GFile *pf = g_file_new_for_path(S.prefs_path);
    GFileMonitor *pm = g_file_monitor_file(pf, G_FILE_MONITOR_WATCH_MOVES, NULL, NULL);
    if (pm) g_signal_connect(pm, "changed", G_CALLBACK(on_prefs_file), NULL);
    g_object_unref(pf);

    GFile *f = g_file_new_for_path(S.theme_path);
    GFileMonitor *m = g_file_monitor_file(f, G_FILE_MONITOR_WATCH_MOVES, NULL, NULL);
    if (m) g_signal_connect(m, "changed", G_CALLBACK(on_theme_file), NULL);
    g_object_unref(f);
    connect_service();
    gtk_main();
    return 0;
}
