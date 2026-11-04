/* nexa-deck: NexaOS's quick settings, a column of cards on the right under
 * the bar (replaces the Python Control Centre). Design:
 * TODOnexa_identity.md, section "Deck".
 *
 *   nexa-deck [now|wifi|bluetooth|sound|brightness|vpn|quiet|playing|signals]
 *
 * Opens Deck (with that card opened), or closes it if it is already open.
 * One full-screen transparent layer-shell surface: a click outside the
 * cards or Escape closes it, and the process ends. Everything it reads is
 * asynchronous and only while it is open.
 *
 * NEXA_DECK_STATE=<file>: write what is shown there as JSON lines (tests).
 * NEXA_BROKER_SOCKET: the broker's socket (default /run/nexa-broker.sock). */
#include <errno.h>
#include <gio/gio.h>
#include <gtk-layer-shell.h>
#include <gtk/gtk.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

#include "model.h"

#define CARD_W 360
#define CHAMFER 16.0
#define STAGGER_MS 40
#define SLIDE_MS 220
#define CLOSE_MS 140
#define MAX_NET 10
#define MAX_BT 10
#define MAX_TUN 8
#define MAX_NOTES 20
#define IWD "net.connman.iwd"
#define BLUEZ "org.bluez"
#define MPRIS "org.mpris.MediaPlayer2."

typedef struct { double r, g, b; } rgb;

enum { C_NOW, C_WIFI, C_BT, C_SOUND, C_BRIGHT, C_VPN, C_QUIET, C_PLAYING, C_SIGNALS, NCARDS };
static const char *const KEYS[NCARDS] = {"now", "wifi", "bluetooth", "sound", "brightness", "vpn", "quiet", "playing", "signals"};

struct card {
    GtkWidget *revealer, *frame, *head, *icon, *title, *sub, *sw, *more, *more_box, *extra;
    int shown, lit, open, expandable;
    gulong sw_handler;
};

struct net { char path[200], name[80], type[16]; int known, connected, percent; };
struct btdev { char path[200], name[80]; int connected; };

static struct {
    GtkApplication *app;
    GtkWidget *win, *column, *scroller;
    struct card c[NCARDS];
    struct { rgb bg, surface, text, muted, accent, warn; double opacity; } col;
    int animations, closing, wanted;
    guint tick;
    GDBusConnection *sys, *ses;
    /* what was last read */
    struct { int have, powered, nnet; char dev[200], station[200], ssid[80], busy[96]; struct net net[MAX_NET]; GtkWidget *rows[MAX_NET]; } wifi;
    char error[200];  /* the last thing that went wrong, for the state file */
    struct { int have, powered, ndev; char adapter[200]; struct btdev dev[MAX_BT]; } bt;
    struct { int have, percent, muted; GtkWidget *scale, *mute; gulong h; guint pending; int want; } sound;
    struct { int have, percent; GtkWidget *scale; gulong h; guint pending; int want; } bright;
    struct { int have, n; struct tunnel t[MAX_TUN]; char error[160]; } vpn;
    struct { int known, on; } quiet;
    struct { int have, playing; char name[160], title[160], artist[160]; GtkWidget *play; } media;
    struct { int n, cleared; struct note notes[MAX_NOTES]; GtkWidget *clear; } sig;
    struct battery bat;
    int have_bat;
    GtkWidget *clock, *date, *battery;
    char theme_path[600];
} D;

static void refresh(void);
static void write_state(void);
static void close_deck(void);

/* ---------- theme ---------- */

static rgb hexcol(const char *s, rgb fb)
{
    unsigned v;
    if (!s || sscanf(s, "#%6x", &v) != 1) return fb;
    return (rgb){((v >> 16) & 255) / 255.0, ((v >> 8) & 255) / 255.0, (v & 255) / 255.0};
}

static void load_theme(void)
{
    D.col.bg = hexcol("#080c1e", D.col.bg);
    D.col.surface = hexcol("#141c3a", D.col.surface);
    D.col.text = hexcol("#e6ecff", D.col.text);
    D.col.muted = hexcol("#8f9bc4", D.col.muted);
    D.col.accent = hexcol("#4da3ff", D.col.accent);
    D.col.warn = hexcol("#ffb547", D.col.warn);
    D.col.opacity = 0.85;
    char *text = NULL;
    if (g_file_get_contents(D.theme_path, &text, NULL, NULL)) {
        char **lines = g_strsplit(text, "\n", -1);
        for (char **l = lines; *l; l++) {
            char *eq = strchr(*l, '=');
            if (!eq || **l == '#') continue;
            *eq = 0;
            const char *k = *l, *v = eq + 1;
            if (!strcmp(k, "bg")) D.col.bg = hexcol(v, D.col.bg);
            else if (!strcmp(k, "surface")) D.col.surface = hexcol(v, D.col.surface);
            else if (!strcmp(k, "text")) D.col.text = hexcol(v, D.col.text);
            else if (!strcmp(k, "muted")) D.col.muted = hexcol(v, D.col.muted);
            else if (!strcmp(k, "accent")) D.col.accent = hexcol(v, D.col.accent);
            else if (!strcmp(k, "warn")) D.col.warn = hexcol(v, D.col.warn);
            else if (!strcmp(k, "opacity")) D.col.opacity = g_ascii_strtod(v, NULL);
            else if (!strcmp(k, "icon_theme")) g_object_set(gtk_settings_get_default(), "gtk-icon-theme-name", v, NULL);
        }
        g_strfreev(lines);
        g_free(text);
    }
}

static char *css_rgb(rgb c, double a)
{
    return g_strdup_printf("rgba(%d,%d,%d,%.2f)", (int)lround(c.r * 255), (int)lround(c.g * 255), (int)lround(c.b * 255), a);
}

static void apply_css(void)
{
    char *text = css_rgb(D.col.text, 1), *muted = css_rgb(D.col.muted, 1), *acc = css_rgb(D.col.accent, 1),
         *acc_soft = css_rgb(D.col.accent, 0.16), *acc_mid = css_rgb(D.col.accent, 0.32), *field = css_rgb(D.col.text, 0.12);
    char *css = g_strdup_printf(
        "window, .deck-root { background: transparent; }\n"
        ".deck-card label { color: %s; font-family: Inter, sans-serif; }\n"
        ".deck-card image { color: %s; }\n"
        ".deck-card label.deck-title { font-weight: 600; font-size: 10.5pt; }\n"
        ".deck-card label.deck-sub, .deck-card label.deck-muted { color: %s; font-size: 9.5pt; }\n"
        ".deck-card label.deck-time { font-size: 22pt; font-weight: 300; }\n"
        ".deck-card label.deck-app { color: %s; font-size: 9pt; font-weight: 600; }\n"
        ".deck-card button { background: transparent; background-image: none; border: none; box-shadow: none; color: %s; "
        "padding: 4px 8px; border-radius: 10px; min-height: 0; }\n"
        ".deck-card button:hover { background: %s; }\n"
        ".deck-card button:focus { box-shadow: inset 0 0 0 2px %s; }\n"
        ".deck-card button.deck-pill { background: %s; padding: 5px 12px; }\n"
        ".deck-card button.deck-pill:hover { background: %s; }\n"
        ".deck-card switch { background: %s; border: none; border-radius: 12px; min-height: 22px; }\n"
        ".deck-card switch:checked { background: %s; }\n"
        ".deck-card switch slider { background: %s; border: none; border-radius: 10px; min-width: 20px; min-height: 20px; box-shadow: none; }\n"
        ".deck-card scale trough { background: %s; border: none; min-height: 6px; border-radius: 3px; }\n"
        ".deck-card scale highlight { background: %s; border-radius: 3px; }\n"
        ".deck-card scale slider { background: %s; border: none; min-width: 16px; min-height: 16px; border-radius: 8px; box-shadow: none; }\n"
        ".deck-card separator { background: %s; }\n",
        text, text, muted, acc, text, acc_soft, acc, field, acc_soft, field, acc, text, field, acc, text, field);
    static GtkCssProvider *p;
    if (!p) {
        p = gtk_css_provider_new();
        gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(p), GTK_STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    }
    gtk_css_provider_load_from_data(p, css, -1, NULL);
    g_free(css);
    g_free(text);
    g_free(muted);
    g_free(acc);
    g_free(acc_soft);
    g_free(acc_mid);
    g_free(field);
}

/* ---------- small helpers ---------- */

static void spawn_argv(const char *const *argv)
{
    g_spawn_async(NULL, (gchar **)argv, NULL, G_SPAWN_SEARCH_PATH | G_SPAWN_STDOUT_TO_DEV_NULL | G_SPAWN_STDERR_TO_DEV_NULL, NULL, NULL,
                  NULL, NULL);
}

static void open_and_close(const char *const *argv)
{
    spawn_argv(argv);
    close_deck();
}

typedef void (*text_cb)(const char *out, gpointer data);
struct cmd_ctx { text_cb cb; gpointer data; };

static void cmd_done(GObject *src, GAsyncResult *res, gpointer user)
{
    struct cmd_ctx *c = user;
    char *out = NULL;
    GError *err = NULL;
    g_subprocess_communicate_utf8_finish(G_SUBPROCESS(src), res, &out, NULL, &err);
    if (err) g_error_free(err);
    if (c->cb) c->cb(out && g_subprocess_get_successful(G_SUBPROCESS(src)) ? out : NULL, c->data);
    g_free(out);
    g_object_unref(src);
    g_free(c);
}

/* Runs a program without a shell; its output (NULL if it failed) to cb. */
static void run_cmd(const char *const *argv, text_cb cb, gpointer data)
{
    GError *err = NULL;
    GSubprocess *p = g_subprocess_newv(argv, G_SUBPROCESS_FLAGS_STDOUT_PIPE | G_SUBPROCESS_FLAGS_STDERR_SILENCE, &err);
    if (!p) {
        g_error_free(err);
        if (cb) cb(NULL, data);
        return;
    }
    struct cmd_ctx *c = g_new0(struct cmd_ctx, 1);
    c->cb = cb;
    c->data = data;
    g_subprocess_communicate_utf8_async(p, NULL, NULL, cmd_done, c);
}

/* ---------- the broker (VPN, brightness) ---------- */

static void broker_thread(GTask *task, gpointer src, gpointer data, GCancellable *c)
{
    const char *req = data, *path = g_getenv("NEXA_BROKER_SOCKET");
    if (!path || !*path) path = "/run/nexa-broker.sock";
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    int fd = strlen(path) < sizeof a.sun_path ? socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0) : -1;
    GString *reply = g_string_new(NULL);
    if (fd >= 0) {
        strcpy(a.sun_path, path);
        struct timeval tv = {60, 0};
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
        if (connect(fd, (struct sockaddr *)&a, sizeof a) == 0 && write(fd, req, strlen(req)) == (ssize_t)strlen(req)) {
            char buf[4096];
            ssize_t r;
            while (reply->len < (1u << 20) && (r = read(fd, buf, sizeof buf)) > 0) {
                g_string_append_len(reply, buf, r);
                if (memchr(buf, '\n', (size_t)r)) break;
            }
        }
        close(fd);
    }
    g_task_return_pointer(task, g_string_free(reply, FALSE), g_free);
}

typedef void (*reply_cb)(const char *reply, gpointer data);
struct broker_ctx { reply_cb cb; gpointer data; };

static void broker_done(GObject *src, GAsyncResult *res, gpointer user)
{
    struct broker_ctx *b = user;
    char *reply = g_task_propagate_pointer(G_TASK(res), NULL);
    if (b->cb) b->cb(reply, b->data);
    g_free(reply);
    g_free(b);
}

static void broker(const char *action, const char *name, int percent, reply_cb cb, gpointer data)
{
    char req[256];
    if (broker_request(req, sizeof req, action, name, percent) < 0) {
        if (cb) cb(NULL, data);
        return;
    }
    struct broker_ctx *b = g_new0(struct broker_ctx, 1);
    b->cb = cb;
    b->data = data;
    GTask *t = g_task_new(NULL, NULL, broker_done, b);
    g_task_set_task_data(t, g_strdup(req), g_free);
    g_task_run_in_thread(t, broker_thread);
    g_object_unref(t);
}

/* ---------- card drawing ---------- */

/* Half a hexagon: straight against the screen edge, chamfered toward it. */
static void card_path(cairo_t *cr, double w, double h)
{
    double c = fmin(CHAMFER, h / 2), r = 10;
    cairo_new_path(cr);
    cairo_move_to(cr, c, 0.5);
    cairo_line_to(cr, w - r - 0.5, 0.5);
    cairo_arc(cr, w - r - 0.5, r + 0.5, r, -G_PI / 2, 0);
    cairo_line_to(cr, w - 0.5, h - r - 0.5);
    cairo_arc(cr, w - r - 0.5, h - r - 0.5, r, 0, G_PI / 2);
    cairo_line_to(cr, c, h - 0.5);
    cairo_line_to(cr, 0.5, h - c);
    cairo_line_to(cr, 0.5, c);
    cairo_close_path(cr);
}

static gboolean draw_card(GtkWidget *w, cairo_t *cr, gpointer data)
{
    struct card *c = data;
    double W = gtk_widget_get_allocated_width(w), H = gtk_widget_get_allocated_height(w);
    card_path(cr, W, H);
    cairo_set_source_rgba(cr, D.col.surface.r, D.col.surface.g, D.col.surface.b, fmax(D.col.opacity, 0.88));
    cairo_fill_preserve(cr);
    cairo_set_source_rgba(cr, D.col.accent.r, D.col.accent.g, D.col.accent.b, 0.22);
    cairo_set_line_width(cr, 1);
    cairo_stroke(cr);
    if (c->lit) {  /* the chamfered side lit: this thing is on */
        double ch = fmin(CHAMFER, H / 2);
        cairo_move_to(cr, ch + 10, 1);
        cairo_line_to(cr, ch, 1);
        cairo_line_to(cr, 1, ch);
        cairo_line_to(cr, 1, H - ch);
        cairo_line_to(cr, ch, H - 1);
        cairo_line_to(cr, ch + 10, H - 1);
        cairo_set_source_rgba(cr, D.col.accent.r, D.col.accent.g, D.col.accent.b, 0.95);
        cairo_set_line_width(cr, 2.2);
        cairo_set_line_join(cr, CAIRO_LINE_JOIN_ROUND);
        cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
        cairo_stroke(cr);
    }
    return FALSE;  /* then the card's contents */
}

/* Clicks on a card stay on the card (a click outside closes Deck). */
static gboolean swallow(GtkWidget *w, GdkEventButton *e, gpointer d) { return TRUE; }

/* Text from the outside (network and device names, notifications) shown
 * only as valid UTF-8. */
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

static GtkWidget *label(const char *text, const char *cls)
{
    GtkWidget *l = gtk_label_new(NULL);
    set_text(l, text);
    gtk_label_set_xalign(GTK_LABEL(l), 0);
    gtk_label_set_ellipsize(GTK_LABEL(l), PANGO_ELLIPSIZE_END);
    if (cls) gtk_style_context_add_class(gtk_widget_get_style_context(l), cls);
    return l;
}

static GtkWidget *button(const char *text, const char *cls, GCallback cb, gpointer data)
{
    GtkWidget *b = gtk_button_new_with_label(text);
    if (cls) gtk_style_context_add_class(gtk_widget_get_style_context(b), cls);
    g_signal_connect(b, "clicked", cb, data);
    return b;
}

/* Lists are rebuilt only when what they show changed, so a click in the
 * middle of a 2 s refresh still lands on the row it was meant for. */
static int changed(char **last, GString *now)
{
    int diff = !*last || strcmp(*last, now->str);
    g_free(*last);
    *last = g_string_free(now, FALSE);
    return diff;
}

static void clear_box(GtkWidget *box)
{
    GList *kids = gtk_container_get_children(GTK_CONTAINER(box));
    for (GList *l = kids; l; l = l->next) gtk_widget_destroy(l->data);
    g_list_free(kids);
}

/* An opened card that runs past the bottom of the screen scrolls into view. */
static gboolean scroll_to(gpointer data)
{
    struct card *c = &D.c[GPOINTER_TO_INT(data)];
    int y = 0;
    if (!D.win || !gtk_widget_get_mapped(c->frame) || !gtk_widget_translate_coordinates(c->frame, D.column, 0, 0, NULL, &y))
        return G_SOURCE_REMOVE;
    GtkAdjustment *a = gtk_scrolled_window_get_vadjustment(GTK_SCROLLED_WINDOW(D.scroller));
    double page = gtk_adjustment_get_page_size(a), top = gtk_adjustment_get_value(a), h = gtk_widget_get_allocated_height(c->frame);
    if (y + h > top + page) gtk_adjustment_set_value(a, fmin(y, y + h - page));
    else if (y < top) gtk_adjustment_set_value(a, y);
    return G_SOURCE_REMOVE;
}

static void set_card_open(int i, int open)
{
    struct card *c = &D.c[i];
    if (!c->expandable) return;
    c->open = open;
    gtk_revealer_set_reveal_child(GTK_REVEALER(c->more), open);
    if (open) g_timeout_add(D.animations ? 200 : 30, scroll_to, GINT_TO_POINTER(i));
    write_state();
}

static void on_head(GtkButton *b, gpointer data) { set_card_open(GPOINTER_TO_INT(data), !D.c[GPOINTER_TO_INT(data)].open); }

/* A card: icon, title and state in a button that opens it, an optional
 * switch, an optional always-visible extra row, and the part that opens. */
static void make_card(int i, const char *icon, const char *title, int with_switch, int expandable)
{
    struct card *c = &D.c[i];
    c->expandable = expandable;
    c->frame = gtk_event_box_new();
    gtk_widget_set_app_paintable(c->frame, TRUE);
    gtk_style_context_add_class(gtk_widget_get_style_context(c->frame), "deck-card");
    g_signal_connect(c->frame, "draw", G_CALLBACK(draw_card), c);
    g_signal_connect(c->frame, "button-press-event", G_CALLBACK(swallow), NULL);
    gtk_widget_set_size_request(c->frame, CARD_W, -1);

    GtkWidget *inner = gtk_box_new(GTK_ORIENTATION_VERTICAL, 4);
    g_object_set(inner, "margin-start", 22, "margin-end", 12, "margin-top", 7, "margin-bottom", 7, NULL);
    gtk_container_add(GTK_CONTAINER(c->frame), inner);

    GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 10);
    gtk_box_pack_start(GTK_BOX(inner), row, FALSE, FALSE, 0);
    GtkWidget *headbox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 10);
    c->icon = gtk_image_new_from_icon_name(icon, GTK_ICON_SIZE_LARGE_TOOLBAR);
    gtk_box_pack_start(GTK_BOX(headbox), c->icon, FALSE, FALSE, 0);
    GtkWidget *texts = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    c->title = label(title, "deck-title");
    c->sub = label("", "deck-sub");
    gtk_box_pack_start(GTK_BOX(texts), c->title, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(texts), c->sub, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(headbox), texts, TRUE, TRUE, 0);
    if (expandable) {
        c->head = gtk_button_new();
        gtk_container_add(GTK_CONTAINER(c->head), headbox);
        g_signal_connect(c->head, "clicked", G_CALLBACK(on_head), GINT_TO_POINTER(i));
        gtk_widget_set_tooltip_text(c->head, "Open or close this card");
    } else {
        c->head = headbox;  /* lined up with the button heads of the others */
        g_object_set(headbox, "margin-start", 8, "margin-end", 8, "margin-top", 4, "margin-bottom", 4, NULL);
    }
    gtk_box_pack_start(GTK_BOX(row), c->head, TRUE, TRUE, 0);
    if (with_switch) {
        c->sw = gtk_switch_new();
        gtk_widget_set_valign(c->sw, GTK_ALIGN_CENTER);
        atk_object_set_name(gtk_widget_get_accessible(c->sw), title);
        gtk_box_pack_end(GTK_BOX(row), c->sw, FALSE, FALSE, 0);
    }
    c->extra = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(inner), c->extra, FALSE, FALSE, 0);
    c->more = gtk_revealer_new();
    gtk_revealer_set_transition_type(GTK_REVEALER(c->more), GTK_REVEALER_TRANSITION_TYPE_SLIDE_DOWN);
    gtk_revealer_set_transition_duration(GTK_REVEALER(c->more), D.animations ? 160 : 0);
    c->more_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    gtk_container_add(GTK_CONTAINER(c->more), c->more_box);
    gtk_box_pack_start(GTK_BOX(inner), c->more, FALSE, FALSE, 0);

    c->revealer = gtk_revealer_new();
    gtk_revealer_set_transition_type(GTK_REVEALER(c->revealer), GTK_REVEALER_TRANSITION_TYPE_SLIDE_LEFT);
    gtk_revealer_set_transition_duration(GTK_REVEALER(c->revealer), D.animations ? SLIDE_MS : 0);
    gtk_widget_set_halign(c->revealer, GTK_ALIGN_END);
    gtk_container_add(GTK_CONTAINER(c->revealer), c->frame);
    gtk_box_pack_start(GTK_BOX(D.column), c->revealer, FALSE, FALSE, 0);
    gtk_widget_set_no_show_all(c->revealer, TRUE);
    gtk_widget_show_all(c->frame);
    gtk_widget_hide(c->extra);
}

static void set_sub(int i, const char *text) { set_text(D.c[i].sub, text); }

static void set_switch(int i, int on)
{
    struct card *c = &D.c[i];
    if (!c->sw) return;
    g_signal_handler_block(c->sw, c->sw_handler);
    gtk_switch_set_active(GTK_SWITCH(c->sw), on);
    gtk_switch_set_state(GTK_SWITCH(c->sw), on);
    g_signal_handler_unblock(c->sw, c->sw_handler);
}

static void set_lit(int i, int lit)
{
    if (D.c[i].lit != lit) {
        D.c[i].lit = lit;
        gtk_widget_queue_draw(D.c[i].frame);
    }
}

/* A card appears (slides in) once its thing is known to exist. */
static void show_card(int i, int shown)
{
    struct card *c = &D.c[i];
    if (c->shown == shown) return;
    c->shown = shown;
    if (shown) {
        gtk_widget_show(c->revealer);
        gtk_revealer_set_reveal_child(GTK_REVEALER(c->revealer), !D.closing);
        if (D.wanted == i) set_card_open(i, 1);
    } else {
        gtk_widget_hide(c->revealer);
        gtk_revealer_set_reveal_child(GTK_REVEALER(c->revealer), FALSE);
    }
}

/* ---------- Now ---------- */

static void update_now(void)
{
    GDateTime *t = g_date_time_new_now_local();
    char *hm = g_date_time_format(t, "%H:%M"), *day = g_date_time_format(t, "%A %e %B");
    gtk_label_set_text(GTK_LABEL(D.clock), hm);
    gtk_label_set_text(GTK_LABEL(D.date), g_strstrip(day));
    g_free(hm);
    g_free(day);
    g_date_time_unref(t);
    const char *root = g_getenv("NEXA_DECK_POWER_SUPPLY");
    D.have_bat = battery_read(root ? root : "/sys/class/power_supply", &D.bat) == 0;
    if (D.have_bat) {
        char left[48], text[96];
        battery_text(&D.bat, left, sizeof left);
        snprintf(text, sizeof text, "Battery %d %%%s%s", D.bat.percent, left[0] ? "  \xc2\xb7  " : "", left);
        gtk_label_set_text(GTK_LABEL(D.battery), text);
        gtk_widget_show(D.battery);
    } else {
        gtk_widget_hide(D.battery);
    }
}

static void on_settings(GtkButton *b, gpointer d) { open_and_close((const char *[]){"nexa-settings", NULL}); }
static void on_lock(GtkButton *b, gpointer d) { open_and_close((const char *[]){"nexa-lock", NULL}); }
static void on_power(GtkButton *b, gpointer d) { open_and_close((const char *[]){"nexa-power", NULL}); }

static void build_now(void)
{
    struct card *c = &D.c[C_NOW];
    make_card(C_NOW, "preferences-system-time-symbolic", "", 0, 0);
    gtk_widget_hide(c->icon);
    gtk_widget_hide(c->title);
    gtk_widget_hide(c->sub);
    GtkWidget *texts = gtk_widget_get_parent(c->title);
    D.clock = label("", "deck-time");
    D.date = label("", "deck-sub");
    D.battery = label("", "deck-sub");
    gtk_box_pack_start(GTK_BOX(texts), D.clock, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(texts), D.date, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(texts), D.battery, FALSE, FALSE, 0);
    gtk_widget_show(D.clock);
    gtk_widget_show(D.date);
    gtk_box_pack_start(GTK_BOX(c->extra), button("Settings", "deck-pill", G_CALLBACK(on_settings), NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(c->extra), button("Lock", "deck-pill", G_CALLBACK(on_lock), NULL), FALSE, FALSE, 0);
    gtk_box_pack_end(GTK_BOX(c->extra), button("Power", "deck-pill", G_CALLBACK(on_power), NULL), FALSE, FALSE, 0);
    gtk_widget_show_all(c->extra);
    update_now();
    show_card(C_NOW, 1);
}

/* ---------- Wi-Fi (iwd) ---------- */

static void wifi_settings(GtkButton *b, gpointer d) { open_and_close((const char *[]){"nexa-settings", "wi-fi", NULL}); }

static void connect_done(GObject *src, GAsyncResult *res, gpointer d)
{
    GError *err = NULL;
    GVariant *r = g_dbus_connection_call_finish(G_DBUS_CONNECTION(src), res, &err);
    if (r) g_variant_unref(r);
    if (err) {
        snprintf(D.wifi.busy, sizeof D.wifi.busy, "Couldn't connect");
        g_error_free(err);
    } else {
        D.wifi.busy[0] = 0;
    }
    refresh();
}

static void on_network(GtkButton *b, gpointer data)
{
    int k = GPOINTER_TO_INT(data);
    if (k < 0 || k >= D.wifi.nnet) return;
    struct net *n = &D.wifi.net[k];
    if (n->connected) return;
    if (!n->known && strcmp(n->type, "open")) {  /* needs a password: Settings asks for it */
        wifi_settings(NULL, NULL);
        return;
    }
    snprintf(D.wifi.busy, sizeof D.wifi.busy, "Connecting to %s\xe2\x80\xa6", n->name);
    set_sub(C_WIFI, D.wifi.busy);
    g_dbus_connection_call(D.sys, IWD, n->path, IWD ".Network", "Connect", NULL, NULL, G_DBUS_CALL_FLAGS_NONE, 30000, NULL, connect_done,
                           NULL);
}

static void fill_wifi_list(void)
{
    struct card *c = &D.c[C_WIFI];
    static char *last;
    GString *now = g_string_new(NULL);
    g_string_append_printf(now, "%d", D.wifi.powered);
    for (int k = 0; k < D.wifi.nnet; k++)
        g_string_append_printf(now, "|%s%d%d%d", D.wifi.net[k].path, D.wifi.net[k].connected, D.wifi.net[k].known, signal_bars(D.wifi.net[k].percent));
    if (!changed(&last, now)) return;
    memset(D.wifi.rows, 0, sizeof D.wifi.rows);
    clear_box(c->more_box);
    for (int k = 0; k < D.wifi.nnet; k++) {
        struct net *n = &D.wifi.net[k];
        static const char *const bars[] = {"network-wireless-signal-none-symbolic", "network-wireless-signal-weak-symbolic",
                                           "network-wireless-signal-ok-symbolic", "network-wireless-signal-good-symbolic",
                                           "network-wireless-signal-excellent-symbolic"};
        GtkWidget *b = gtk_button_new(), *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        gtk_box_pack_start(GTK_BOX(row), gtk_image_new_from_icon_name(bars[signal_bars(n->percent)], GTK_ICON_SIZE_MENU), FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(row), label(n->name, NULL), TRUE, TRUE, 0);
        const char *what = n->connected ? "Connected" : n->known ? "Saved" : strcmp(n->type, "open") ? "Needs a password" : "Open";
        gtk_box_pack_end(GTK_BOX(row), label(what, "deck-muted"), FALSE, FALSE, 0);
        gtk_container_add(GTK_CONTAINER(b), row);
        g_signal_connect(b, "clicked", G_CALLBACK(on_network), GINT_TO_POINTER(k));
        gtk_box_pack_start(GTK_BOX(c->more_box), b, FALSE, FALSE, 0);
        D.wifi.rows[k] = b;
    }
    if (!D.wifi.nnet) gtk_box_pack_start(GTK_BOX(c->more_box), label(D.wifi.powered ? "No networks found yet" : "Wi-Fi is off", "deck-muted"), FALSE, FALSE, 4);
    GtkWidget *more = button("Wi-Fi settings", NULL, G_CALLBACK(wifi_settings), NULL);
    gtk_widget_set_halign(more, GTK_ALIGN_END);
    gtk_box_pack_start(GTK_BOX(c->more_box), more, FALSE, FALSE, 2);
    gtk_widget_show_all(c->more_box);
}

static GHashTable *wifi_props;  /* network path -> its a{sv} */

static void ordered_done(GObject *src, GAsyncResult *res, gpointer d)
{
    GVariant *r = g_dbus_connection_call_finish(G_DBUS_CONNECTION(src), res, NULL);
    D.wifi.nnet = 0;
    if (r && wifi_props) {
        GVariantIter *it;
        const char *path;
        gint16 sig;
        g_variant_get(r, "(a(on))", &it);
        while (D.wifi.nnet < MAX_NET && g_variant_iter_next(it, "(&on)", &path, &sig)) {
            GVariant *p = g_hash_table_lookup(wifi_props, path);
            const char *name = NULL, *type = NULL;
            if (!p || !g_variant_lookup(p, "Name", "&s", &name)) continue;
            struct net *n = &D.wifi.net[D.wifi.nnet++];
            snprintf(n->path, sizeof n->path, "%s", path);
            snprintf(n->name, sizeof n->name, "%s", name);
            snprintf(n->type, sizeof n->type, "%s", g_variant_lookup(p, "Type", "&s", &type) ? type : "");
            gboolean conn = FALSE;
            g_variant_lookup(p, "Connected", "b", &conn);
            n->connected = conn;
            GVariant *kn = g_variant_lookup_value(p, "KnownNetwork", NULL);
            n->known = kn != NULL;
            if (kn) g_variant_unref(kn);
            n->percent = signal_percent(sig);
        }
        g_variant_iter_free(it);
    }
    if (r) g_variant_unref(r);
    fill_wifi_list();
    write_state();
}

static void wifi_objects_done(GObject *src, GAsyncResult *res, gpointer d)
{
    GVariant *r = g_dbus_connection_call_finish(G_DBUS_CONNECTION(src), res, NULL);
    D.wifi.have = 0;
    D.wifi.ssid[0] = D.wifi.station[0] = 0;
    if (wifi_props) g_hash_table_destroy(wifi_props);
    wifi_props = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, (GDestroyNotify)g_variant_unref);
    if (r) {
        GVariant *objs = g_variant_get_child_value(r, 0);
        GVariantIter it;
        const char *path;
        GVariant *ifaces;
        int best_sim = 2;
        char connected_net[200] = "";
        g_variant_iter_init(&it, objs);
        while (g_variant_iter_next(&it, "{&o@a{sa{sv}}}", &path, &ifaces)) {
            GVariant *dev = g_variant_lookup_value(ifaces, IWD ".Device", G_VARIANT_TYPE_VARDICT);
            GVariant *st = g_variant_lookup_value(ifaces, IWD ".Station", G_VARIANT_TYPE_VARDICT);
            GVariant *net = g_variant_lookup_value(ifaces, IWD ".Network", G_VARIANT_TYPE_VARDICT);
            if (dev && st) {
                /* real adapters before mac80211_hwsim's fake radios */
                const char *addr = "";
                g_variant_lookup(dev, "Address", "&s", &addr);
                int sim = g_str_has_prefix(addr, "02:00:00:00:");
                if (sim < best_sim) {
                    best_sim = sim;
                    D.wifi.have = 1;
                    gboolean pw = FALSE;
                    g_variant_lookup(dev, "Powered", "b", &pw);
                    D.wifi.powered = pw;
                    snprintf(D.wifi.dev, sizeof D.wifi.dev, "%s", path);
                    snprintf(D.wifi.station, sizeof D.wifi.station, "%s", path);
                    const char *cn = NULL;
                    connected_net[0] = 0;
                    if (g_variant_lookup(st, "ConnectedNetwork", "&o", &cn)) snprintf(connected_net, sizeof connected_net, "%s", cn);
                }
            } else if (dev && !D.wifi.have) {  /* a radio switched off has no Station */
                D.wifi.have = 1;
                gboolean pw = FALSE;
                g_variant_lookup(dev, "Powered", "b", &pw);
                D.wifi.powered = pw;
                snprintf(D.wifi.dev, sizeof D.wifi.dev, "%s", path);
            }
            if (net) g_hash_table_insert(wifi_props, g_strdup(path), g_variant_ref(net));
            if (dev) g_variant_unref(dev);
            if (st) g_variant_unref(st);
            if (net) g_variant_unref(net);
            g_variant_unref(ifaces);
        }
        if (connected_net[0]) {
            GVariant *p = g_hash_table_lookup(wifi_props, connected_net);
            const char *name = NULL;
            if (p && g_variant_lookup(p, "Name", "&s", &name)) snprintf(D.wifi.ssid, sizeof D.wifi.ssid, "%s", name);
        }
        g_variant_unref(objs);
        g_variant_unref(r);
    }
    show_card(C_WIFI, D.wifi.have);
    if (!D.wifi.have) return;
    set_switch(C_WIFI, D.wifi.powered);
    set_lit(C_WIFI, D.wifi.powered && D.wifi.ssid[0]);
    set_sub(C_WIFI, D.wifi.busy[0] ? D.wifi.busy : !D.wifi.powered ? "Off" : D.wifi.ssid[0] ? D.wifi.ssid : "Not connected");
    if (D.wifi.station[0] && D.wifi.powered)
        g_dbus_connection_call(D.sys, IWD, D.wifi.station, IWD ".Station", "GetOrderedNetworks", NULL, G_VARIANT_TYPE("(a(on))"),
                               G_DBUS_CALL_FLAGS_NONE, 5000, NULL, ordered_done, NULL);
    else {
        D.wifi.nnet = 0;
        fill_wifi_list();
        write_state();
    }
}

static void prop_set_done(GObject *src, GAsyncResult *res, gpointer d)
{
    GError *err = NULL;
    GVariant *r = g_dbus_connection_call_finish(G_DBUS_CONNECTION(src), res, &err);
    if (r) g_variant_unref(r);
    if (err) {
        snprintf(D.error, sizeof D.error, "%s", err->message);
        g_error_free(err);
    }
    refresh();
}

static void set_bool_prop(const char *bus, const char *path, const char *iface, const char *prop, int on)
{
    g_dbus_connection_call(D.sys, bus, path, "org.freedesktop.DBus.Properties", "Set",
                           g_variant_new("(ssv)", iface, prop, g_variant_new_boolean(on)), NULL, G_DBUS_CALL_FLAGS_NONE, 8000, NULL,
                           prop_set_done, NULL);
}

static void on_wifi_switch(GObject *sw, GParamSpec *p, gpointer d)
{
    int on = gtk_switch_get_active(GTK_SWITCH(sw));
    if (D.wifi.dev[0]) set_bool_prop(IWD, D.wifi.dev, IWD ".Device", "Powered", on);
    set_sub(C_WIFI, on ? "Turning on\xe2\x80\xa6" : "Off");
}

/* ---------- Bluetooth (BlueZ) ---------- */

static void bt_settings(GtkButton *b, gpointer d) { open_and_close((const char *[]){"nexa-settings", "bluetooth", NULL}); }

static void bt_done(GObject *src, GAsyncResult *res, gpointer d)
{
    GVariant *r = g_dbus_connection_call_finish(G_DBUS_CONNECTION(src), res, NULL);
    if (r) g_variant_unref(r);
    refresh();
}

static void on_bt_device(GtkButton *b, gpointer data)
{
    int k = GPOINTER_TO_INT(data);
    if (k < 0 || k >= D.bt.ndev) return;
    struct btdev *dv = &D.bt.dev[k];
    set_sub(C_BT, dv->connected ? "Disconnecting\xe2\x80\xa6" : "Connecting\xe2\x80\xa6");
    g_dbus_connection_call(D.sys, BLUEZ, dv->path, BLUEZ ".Device1", dv->connected ? "Disconnect" : "Connect", NULL, NULL,
                           G_DBUS_CALL_FLAGS_NONE, 30000, NULL, bt_done, NULL);
}

static void bt_objects_done(GObject *src, GAsyncResult *res, gpointer d)
{
    GVariant *r = g_dbus_connection_call_finish(G_DBUS_CONNECTION(src), res, NULL);
    D.bt.have = 0;
    D.bt.ndev = 0;
    if (r) {
        GVariant *objs = g_variant_get_child_value(r, 0);
        GVariantIter it;
        const char *path;
        GVariant *ifaces;
        g_variant_iter_init(&it, objs);
        while (g_variant_iter_next(&it, "{&o@a{sa{sv}}}", &path, &ifaces)) {
            GVariant *ad = g_variant_lookup_value(ifaces, BLUEZ ".Adapter1", G_VARIANT_TYPE_VARDICT);
            GVariant *dv = g_variant_lookup_value(ifaces, BLUEZ ".Device1", G_VARIANT_TYPE_VARDICT);
            if (ad && !D.bt.have) {
                D.bt.have = 1;
                gboolean pw = FALSE;
                g_variant_lookup(ad, "Powered", "b", &pw);
                D.bt.powered = pw;
                snprintf(D.bt.adapter, sizeof D.bt.adapter, "%s", path);
            }
            gboolean paired = FALSE, conn = FALSE;
            if (dv && g_variant_lookup(dv, "Paired", "b", &paired) && paired && D.bt.ndev < MAX_BT) {
                struct btdev *x = &D.bt.dev[D.bt.ndev++];
                const char *name = NULL;
                if (!g_variant_lookup(dv, "Alias", "&s", &name)) g_variant_lookup(dv, "Name", "&s", &name);
                snprintf(x->path, sizeof x->path, "%s", path);
                snprintf(x->name, sizeof x->name, "%s", name ? name : "Device");
                g_variant_lookup(dv, "Connected", "b", &conn);
                x->connected = conn;
            }
            if (ad) g_variant_unref(ad);
            if (dv) g_variant_unref(dv);
            g_variant_unref(ifaces);
        }
        g_variant_unref(objs);
        g_variant_unref(r);
    }
    show_card(C_BT, D.bt.have);
    if (!D.bt.have) return;
    int nconn = 0;
    const char *first = NULL;
    for (int k = 0; k < D.bt.ndev; k++)
        if (D.bt.dev[k].connected) nconn++, first = first ? first : D.bt.dev[k].name;
    char sub[120];
    if (!D.bt.powered) snprintf(sub, sizeof sub, "Off");
    else if (nconn == 1) snprintf(sub, sizeof sub, "%s connected", first);
    else if (nconn > 1) snprintf(sub, sizeof sub, "%d devices connected", nconn);
    else snprintf(sub, sizeof sub, "On");
    set_sub(C_BT, sub);
    set_switch(C_BT, D.bt.powered);
    set_lit(C_BT, D.bt.powered && nconn > 0);
    struct card *c = &D.c[C_BT];
    static char *last;
    GString *now = g_string_new(NULL);
    g_string_append_printf(now, "%d", D.bt.powered);
    for (int k = 0; k < D.bt.ndev; k++) g_string_append_printf(now, "|%s%d%s", D.bt.dev[k].path, D.bt.dev[k].connected, D.bt.dev[k].name);
    if (!changed(&last, now)) {
        write_state();
        return;
    }
    clear_box(c->more_box);
    for (int k = 0; k < D.bt.ndev && D.bt.powered; k++) {
        GtkWidget *b = gtk_button_new(), *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        gtk_box_pack_start(GTK_BOX(row), label(D.bt.dev[k].name, NULL), TRUE, TRUE, 0);
        gtk_box_pack_end(GTK_BOX(row), label(D.bt.dev[k].connected ? "Connected \xc2\xb7 disconnect" : "Connect", "deck-muted"), FALSE, FALSE, 0);
        gtk_container_add(GTK_CONTAINER(b), row);
        g_signal_connect(b, "clicked", G_CALLBACK(on_bt_device), GINT_TO_POINTER(k));
        gtk_box_pack_start(GTK_BOX(c->more_box), b, FALSE, FALSE, 0);
    }
    if (!D.bt.ndev || !D.bt.powered)
        gtk_box_pack_start(GTK_BOX(c->more_box), label(D.bt.powered ? "No paired devices" : "Bluetooth is off", "deck-muted"), FALSE, FALSE, 4);
    GtkWidget *more = button("Bluetooth settings", NULL, G_CALLBACK(bt_settings), NULL);
    gtk_widget_set_halign(more, GTK_ALIGN_END);
    gtk_box_pack_start(GTK_BOX(c->more_box), more, FALSE, FALSE, 2);
    gtk_widget_show_all(c->more_box);
    write_state();
}

static void on_bt_switch(GObject *sw, GParamSpec *p, gpointer d)
{
    int on = gtk_switch_get_active(GTK_SWITCH(sw));
    if (D.bt.adapter[0]) set_bool_prop(BLUEZ, D.bt.adapter, BLUEZ ".Adapter1", "Powered", on);
}

/* ---------- Sound (wpctl) ---------- */

static void sound_settings(GtkButton *b, gpointer d) { open_and_close((const char *[]){"nexa-settings", "sound", NULL}); }

static void sound_read(const char *out, gpointer d)
{
    int pct, muted;
    D.sound.have = out && parse_volume(out, &pct, &muted) == 0;
    show_card(C_SOUND, D.sound.have);
    if (!D.sound.have) return;
    D.sound.percent = pct;
    D.sound.muted = muted;
    if (!D.sound.pending) {
        g_signal_handler_block(D.sound.scale, D.sound.h);
        gtk_range_set_value(GTK_RANGE(D.sound.scale), pct > 100 ? 100 : pct);
        g_signal_handler_unblock(D.sound.scale, D.sound.h);
    }
    char sub[48];
    snprintf(sub, sizeof sub, muted ? "Muted" : "%d %%", pct);
    set_sub(C_SOUND, sub);
    gtk_button_set_label(GTK_BUTTON(D.sound.mute), muted ? "Unmute" : "Mute");
    set_lit(C_SOUND, !muted && pct > 0);
    write_state();
}

static void after_sound(const char *out, gpointer d) { run_cmd((const char *[]){"wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@", NULL}, sound_read, NULL); }

static gboolean sound_apply(gpointer d)
{
    D.sound.pending = 0;
    char v[16];
    snprintf(v, sizeof v, "%d%%", D.sound.want);
    run_cmd((const char *[]){"wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", v, NULL}, after_sound, NULL);
    return G_SOURCE_REMOVE;
}

/* Sliders send at most one change per 80 ms. */
static void on_volume(GtkRange *r, gpointer d)
{
    D.sound.want = (int)gtk_range_get_value(r);
    if (!D.sound.pending) D.sound.pending = g_timeout_add(80, sound_apply, NULL);
}

static void on_mute(GtkButton *b, gpointer d)
{
    run_cmd((const char *[]){"wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle", NULL}, after_sound, NULL);
}

static void build_sound(void)
{
    make_card(C_SOUND, "audio-volume-high-symbolic", "Sound", 0, 1);
    struct card *c = &D.c[C_SOUND];
    D.sound.scale = gtk_scale_new_with_range(GTK_ORIENTATION_HORIZONTAL, 0, 100, 1);
    gtk_scale_set_draw_value(GTK_SCALE(D.sound.scale), FALSE);
    atk_object_set_name(gtk_widget_get_accessible(D.sound.scale), "Volume");
    D.sound.h = g_signal_connect(D.sound.scale, "value-changed", G_CALLBACK(on_volume), NULL);
    gtk_box_pack_start(GTK_BOX(c->extra), D.sound.scale, TRUE, TRUE, 0);
    D.sound.mute = button("Mute", NULL, G_CALLBACK(on_mute), NULL);
    gtk_box_pack_end(GTK_BOX(c->extra), D.sound.mute, FALSE, FALSE, 0);
    gtk_widget_show_all(c->extra);
    GtkWidget *more = button("Sound settings", NULL, G_CALLBACK(sound_settings), NULL);
    gtk_widget_set_halign(more, GTK_ALIGN_END);
    gtk_box_pack_start(GTK_BOX(c->more_box), more, FALSE, FALSE, 0);
    gtk_widget_show_all(c->more_box);
}

/* ---------- Brightness (sysfs to read, the broker to change) ---------- */

static void bright_done(const char *reply, gpointer d)
{
    char err[160];
    if (!broker_ok(reply, err, sizeof err)) set_sub(C_BRIGHT, err);
}

static gboolean bright_apply(gpointer d)
{
    D.bright.pending = 0;
    broker("set_brightness", NULL, D.bright.want < 1 ? 1 : D.bright.want, bright_done, NULL);
    char sub[16];
    snprintf(sub, sizeof sub, "%d %%", D.bright.want);
    set_sub(C_BRIGHT, sub);
    return G_SOURCE_REMOVE;
}

static void on_bright(GtkRange *r, gpointer d)
{
    D.bright.want = (int)gtk_range_get_value(r);
    if (!D.bright.pending) D.bright.pending = g_timeout_add(120, bright_apply, NULL);
}

static void update_bright(void)
{
    const char *root = g_getenv("NEXA_DECK_BACKLIGHT");
    int pct = backlight_read(root ? root : "/sys/class/backlight");
    D.bright.have = pct >= 0;
    show_card(C_BRIGHT, D.bright.have);
    if (!D.bright.have || D.bright.pending) return;
    D.bright.percent = pct;
    g_signal_handler_block(D.bright.scale, D.bright.h);
    gtk_range_set_value(GTK_RANGE(D.bright.scale), pct);
    g_signal_handler_unblock(D.bright.scale, D.bright.h);
    char sub[16];
    snprintf(sub, sizeof sub, "%d %%", pct);
    set_sub(C_BRIGHT, sub);
}

static void build_bright(void)
{
    make_card(C_BRIGHT, "display-brightness-symbolic", "Brightness", 0, 0);
    struct card *c = &D.c[C_BRIGHT];
    D.bright.scale = gtk_scale_new_with_range(GTK_ORIENTATION_HORIZONTAL, 1, 100, 1);
    gtk_scale_set_draw_value(GTK_SCALE(D.bright.scale), FALSE);
    atk_object_set_name(gtk_widget_get_accessible(D.bright.scale), "Brightness");
    D.bright.h = g_signal_connect(D.bright.scale, "value-changed", G_CALLBACK(on_bright), NULL);
    gtk_box_pack_start(GTK_BOX(c->extra), D.bright.scale, TRUE, TRUE, 0);
    gtk_widget_show_all(c->extra);
}

/* ---------- VPN (the broker) ---------- */

static void vpn_settings(GtkButton *b, gpointer d) { open_and_close((const char *[]){"nexa-settings", "vpn", NULL}); }
static void vpn_list(void);

static void vpn_changed(const char *reply, gpointer d)
{
    if (!broker_ok(reply, D.vpn.error, sizeof D.vpn.error)) set_sub(C_VPN, D.vpn.error);
    else D.vpn.error[0] = 0;
    vpn_list();
}

static void vpn_turn(const char *name, int on)
{
    set_sub(C_VPN, on ? "Connecting\xe2\x80\xa6" : "Disconnecting\xe2\x80\xa6");
    broker(on ? "vpn_up" : "vpn_down", name, -1, vpn_changed, NULL);
}

static void on_tunnel_switch(GObject *sw, GParamSpec *p, gpointer data)
{
    int k = GPOINTER_TO_INT(data);
    if (k >= 0 && k < D.vpn.n) vpn_turn(D.vpn.t[k].name, gtk_switch_get_active(GTK_SWITCH(sw)));
}

static void on_vpn_switch(GObject *sw, GParamSpec *p, gpointer d)
{
    int on = gtk_switch_get_active(GTK_SWITCH(sw));
    if (!D.vpn.n) return;
    if (on) {
        vpn_turn(D.vpn.t[0].name, 1);
    } else {
        for (int k = 0; k < D.vpn.n; k++)
            if (D.vpn.t[k].active) vpn_turn(D.vpn.t[k].name, 0);
    }
}

static void vpn_read(const char *reply, gpointer d)
{
    int n = parse_tunnels(reply, D.vpn.t, MAX_TUN);
    D.vpn.have = n > 0;
    D.vpn.n = n > 0 ? n : 0;
    show_card(C_VPN, D.vpn.have);
    if (!D.vpn.have) return;
    const char *active = NULL;
    for (int k = 0; k < D.vpn.n; k++)
        if (D.vpn.t[k].active) active = D.vpn.t[k].name;
    char sub[200];
    if (D.vpn.error[0]) snprintf(sub, sizeof sub, "%s", D.vpn.error);
    else if (active) snprintf(sub, sizeof sub, "Connected: %s", active);
    else snprintf(sub, sizeof sub, "Off \xc2\xb7 %s", D.vpn.t[0].name);
    set_sub(C_VPN, sub);
    set_switch(C_VPN, active != NULL);
    set_lit(C_VPN, active != NULL);
    struct card *c = &D.c[C_VPN];
    static char *last;
    GString *now = g_string_new(NULL);
    for (int k = 0; k < D.vpn.n; k++) g_string_append_printf(now, "|%s%d", D.vpn.t[k].name, D.vpn.t[k].active);
    if (!changed(&last, now)) {
        write_state();
        return;
    }
    clear_box(c->more_box);
    for (int k = 0; k < D.vpn.n; k++) {
        GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8), *sw = gtk_switch_new();
        gtk_box_pack_start(GTK_BOX(row), label(D.vpn.t[k].name, NULL), TRUE, TRUE, 0);
        gtk_switch_set_active(GTK_SWITCH(sw), D.vpn.t[k].active);
        atk_object_set_name(gtk_widget_get_accessible(sw), D.vpn.t[k].name);
        g_signal_connect(sw, "notify::active", G_CALLBACK(on_tunnel_switch), GINT_TO_POINTER(k));
        gtk_box_pack_end(GTK_BOX(row), sw, FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(c->more_box), row, FALSE, FALSE, 2);
    }
    GtkWidget *more = button("VPN settings", NULL, G_CALLBACK(vpn_settings), NULL);
    gtk_widget_set_halign(more, GTK_ALIGN_END);
    gtk_box_pack_start(GTK_BOX(c->more_box), more, FALSE, FALSE, 2);
    gtk_widget_show_all(c->more_box);
    write_state();
}

static void vpn_list(void) { broker("vpn_list", NULL, -1, vpn_read, NULL); }

/* ---------- Quiet (mako's do-not-disturb, until the Quiet modes) ---------- */

static void quiet_read(const char *out, gpointer d)
{
    D.quiet.known = out != NULL;
    D.quiet.on = 0;
    if (out) {
        char **modes = g_strsplit_set(out, " \n", -1);
        for (char **m = modes; *m; m++) D.quiet.on |= !strcmp(*m, "do-not-disturb");
        g_strfreev(modes);
    }
    show_card(C_QUIET, D.quiet.known);
    set_switch(C_QUIET, D.quiet.on);
    set_lit(C_QUIET, D.quiet.on);
    set_sub(C_QUIET, D.quiet.on ? "Notifications wait until you're back" : "Off");
    write_state();
}

static void after_quiet(const char *out, gpointer d) { run_cmd((const char *[]){"makoctl", "mode", NULL}, quiet_read, NULL); }

static void on_quiet_switch(GObject *sw, GParamSpec *p, gpointer d)
{
    int on = gtk_switch_get_active(GTK_SWITCH(sw));
    run_cmd((const char *[]){"makoctl", "mode", on ? "-a" : "-r", "do-not-disturb", NULL}, after_quiet, NULL);
}

/* ---------- Playing (MPRIS) ---------- */

static void media_call(const char *method)
{
    if (D.media.have)
        g_dbus_connection_call(D.ses, D.media.name, "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player", method, NULL, NULL,
                               G_DBUS_CALL_FLAGS_NONE, 3000, NULL, prop_set_done, NULL);
}
static void on_prev(GtkButton *b, gpointer d) { media_call("Previous"); }
static void on_play(GtkButton *b, gpointer d) { media_call("PlayPause"); }
static void on_next(GtkButton *b, gpointer d) { media_call("Next"); }

struct media_probe { char name[160]; };

static void media_props_done(GObject *src, GAsyncResult *res, gpointer data)
{
    struct media_probe *mp = data;
    GVariant *r = g_dbus_connection_call_finish(G_DBUS_CONNECTION(src), res, NULL);
    if (r) {
        GVariant *props = g_variant_get_child_value(r, 0);
        const char *status = "";
        g_variant_lookup(props, "PlaybackStatus", "&s", &status);
        int playing = !strcmp(status, "Playing");
        /* the one playing wins over one that is paused */
        if (!D.media.have || playing || !strcmp(D.media.name, mp->name)) {
            D.media.have = 1;
            D.media.playing = playing;
            snprintf(D.media.name, sizeof D.media.name, "%s", mp->name);
            D.media.title[0] = D.media.artist[0] = 0;
            GVariant *meta = g_variant_lookup_value(props, "Metadata", G_VARIANT_TYPE_VARDICT);
            if (meta) {
                const char *t = NULL;
                if (g_variant_lookup(meta, "xesam:title", "&s", &t)) snprintf(D.media.title, sizeof D.media.title, "%s", t);
                GVariant *artists = g_variant_lookup_value(meta, "xesam:artist", G_VARIANT_TYPE_STRING_ARRAY);
                if (artists) {
                    const gchar **a = g_variant_get_strv(artists, NULL);
                    if (a && a[0]) snprintf(D.media.artist, sizeof D.media.artist, "%s", a[0]);
                    g_free(a);
                    g_variant_unref(artists);
                }
                g_variant_unref(meta);
            }
            set_text(D.c[C_PLAYING].title, D.media.title[0] ? D.media.title : "Playing");
            set_sub(C_PLAYING, D.media.artist[0] ? D.media.artist : (playing ? "Playing" : "Paused"));
            gtk_button_set_label(GTK_BUTTON(D.media.play), playing ? "Pause" : "Play");
            set_lit(C_PLAYING, playing);
            show_card(C_PLAYING, 1);
            write_state();
        }
        g_variant_unref(props);
        g_variant_unref(r);
    }
    g_free(mp);
}

static void media_names_done(GObject *src, GAsyncResult *res, gpointer d)
{
    GVariant *r = g_dbus_connection_call_finish(G_DBUS_CONNECTION(src), res, NULL);
    int any = 0;
    if (r) {
        GVariantIter *it;
        const char *name;
        g_variant_get(r, "(as)", &it);
        while (g_variant_iter_next(it, "&s", &name)) {
            if (!g_str_has_prefix(name, MPRIS)) continue;
            any = 1;
            struct media_probe *mp = g_new0(struct media_probe, 1);
            snprintf(mp->name, sizeof mp->name, "%s", name);
            g_dbus_connection_call(D.ses, name, "/org/mpris/MediaPlayer2", "org.freedesktop.DBus.Properties", "GetAll",
                                   g_variant_new("(s)", "org.mpris.MediaPlayer2.Player"), G_VARIANT_TYPE("(a{sv})"), G_DBUS_CALL_FLAGS_NONE,
                                   2000, NULL, media_props_done, mp);
        }
        g_variant_iter_free(it);
        g_variant_unref(r);
    }
    if (!any) {
        D.media.have = 0;
        show_card(C_PLAYING, 0);
    }
}

static void build_playing(void)
{
    make_card(C_PLAYING, "audio-x-generic-symbolic", "Playing", 0, 0);
    struct card *c = &D.c[C_PLAYING];
    gtk_box_pack_start(GTK_BOX(c->extra), button("Back", "deck-pill", G_CALLBACK(on_prev), NULL), FALSE, FALSE, 0);
    D.media.play = button("Play", "deck-pill", G_CALLBACK(on_play), NULL);
    gtk_box_pack_start(GTK_BOX(c->extra), D.media.play, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(c->extra), button("Next", "deck-pill", G_CALLBACK(on_next), NULL), FALSE, FALSE, 0);
    gtk_widget_show_all(c->extra);
}

/* ---------- Signals (mako's notifications) ---------- */

static char *cleared_path(void)
{
    const char *cache = g_getenv("XDG_CACHE_HOME");
    return cache && *cache ? g_build_filename(cache, "nexa", "notifications-cleared", NULL)
                           : g_build_filename(g_get_home_dir(), ".cache", "nexa", "notifications-cleared", NULL);
}

static int read_cleared(void)
{
    char *p = cleared_path(), *text = NULL;
    int v = 0;
    if (g_file_get_contents(p, &text, NULL, NULL)) v = atoi(text);
    g_free(text);
    g_free(p);
    return v;
}

static void fill_signals(void)
{
    struct card *c = &D.c[C_SIGNALS];
    char sub[300];
    if (!D.sig.n) snprintf(sub, sizeof sub, "You're all caught up");
    else snprintf(sub, sizeof sub, "%d \xc2\xb7 %s%s%s", D.sig.n, D.sig.notes[0].app, D.sig.notes[0].app[0] ? ": " : "", D.sig.notes[0].summary);
    set_sub(C_SIGNALS, sub);
    set_lit(C_SIGNALS, D.sig.n > 0);
    static char *last;
    GString *now = g_string_new(NULL);
    for (int k = 0; k < D.sig.n; k++) g_string_append_printf(now, "|%d", D.sig.notes[k].id);
    if (!changed(&last, now)) {
        write_state();
        return;
    }
    clear_box(c->more_box);
    for (int k = 0; k < D.sig.n; k++) {
        const struct note *n = &D.sig.notes[k];
        GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
        g_object_set(box, "margin-top", 4, "margin-bottom", 4, NULL);
        if (n->app[0]) gtk_box_pack_start(GTK_BOX(box), label(n->app, "deck-app"), FALSE, FALSE, 0);
        GtkWidget *s = label(n->summary, NULL);
        gtk_label_set_line_wrap(GTK_LABEL(s), TRUE);
        gtk_label_set_ellipsize(GTK_LABEL(s), PANGO_ELLIPSIZE_NONE);
        gtk_box_pack_start(GTK_BOX(box), s, FALSE, FALSE, 0);
        if (n->body[0]) {
            GtkWidget *bl = label(n->body, "deck-muted");
            gtk_label_set_line_wrap(GTK_LABEL(bl), TRUE);
            gtk_label_set_ellipsize(GTK_LABEL(bl), PANGO_ELLIPSIZE_NONE);
            gtk_label_set_lines(GTK_LABEL(bl), 3);
            gtk_box_pack_start(GTK_BOX(box), bl, FALSE, FALSE, 0);
        }
        gtk_box_pack_start(GTK_BOX(c->more_box), box, FALSE, FALSE, 0);
    }
    gtk_widget_show_all(c->more_box);
    write_state();
}

static void signals_history(const char *out, gpointer d)
{
    D.sig.n = parse_mako(out, D.sig.cleared, D.sig.notes, D.sig.n, MAX_NOTES);
    fill_signals();
}

static void signals_list(const char *out, gpointer d)
{
    D.sig.cleared = read_cleared();
    D.sig.n = parse_mako(out, D.sig.cleared, D.sig.notes, 0, MAX_NOTES);
    run_cmd((const char *[]){"makoctl", "history", NULL}, signals_history, NULL);
}

static void read_signals(void) { run_cmd((const char *[]){"makoctl", "list", NULL}, signals_list, NULL); }

static void after_clear(const char *out, gpointer d) { read_signals(); }

static void on_clear(GtkButton *b, gpointer d)
{
    int newest = D.sig.cleared;
    for (int k = 0; k < D.sig.n; k++)
        if (D.sig.notes[k].id > newest) newest = D.sig.notes[k].id;
    char *p = cleared_path(), *dir = g_path_get_dirname(p), num[24];
    g_mkdir_with_parents(dir, 0700);
    snprintf(num, sizeof num, "%d", newest);
    g_file_set_contents(p, num, -1, NULL);
    g_free(dir);
    g_free(p);
    run_cmd((const char *[]){"makoctl", "dismiss", "--all", NULL}, after_clear, NULL);
}

static void notify_settings(GtkButton *b, gpointer d) { open_and_close((const char *[]){"nexa-settings", "notifications", NULL}); }

static void build_signals(void)
{
    make_card(C_SIGNALS, "preferences-system-notifications-symbolic", "Signals", 0, 1);
    struct card *c = &D.c[C_SIGNALS];
    D.sig.clear = button("Clear all", "deck-pill", G_CALLBACK(on_clear), NULL);
    gtk_box_pack_start(GTK_BOX(c->extra), D.sig.clear, FALSE, FALSE, 0);
    gtk_box_pack_end(GTK_BOX(c->extra), button("Notification settings", NULL, G_CALLBACK(notify_settings), NULL), FALSE, FALSE, 0);
    gtk_widget_show_all(c->extra);
    show_card(C_SIGNALS, 1);
}

/* ---------- all of it ---------- */

static void refresh(void)
{
    if (D.closing) return;
    update_now();
    if (D.sys) {
        g_dbus_connection_call(D.sys, IWD, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", NULL,
                               G_VARIANT_TYPE("(a{oa{sa{sv}}})"), G_DBUS_CALL_FLAGS_NO_AUTO_START, 4000, NULL, wifi_objects_done, NULL);
        g_dbus_connection_call(D.sys, BLUEZ, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", NULL,
                               G_VARIANT_TYPE("(a{oa{sa{sv}}})"), G_DBUS_CALL_FLAGS_NO_AUTO_START, 4000, NULL, bt_objects_done, NULL);
    }
    if (D.ses)
        g_dbus_connection_call(D.ses, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListNames", NULL,
                               G_VARIANT_TYPE("(as)"), G_DBUS_CALL_FLAGS_NONE, 2000, NULL, media_names_done, NULL);
    run_cmd((const char *[]){"wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@", NULL}, sound_read, NULL);
    update_bright();
    vpn_list();
    run_cmd((const char *[]){"makoctl", "mode", NULL}, quiet_read, NULL);
    read_signals();
    write_state();
}

static gboolean tick(gpointer d)
{
    refresh();
    return G_SOURCE_CONTINUE;
}

/* ---------- state for tests ---------- */

static void rect_of(GString *s, const char *name, GtkWidget *w)
{
    if (!w || !gtk_widget_get_mapped(w)) return;
    int x, y;
    if (!gtk_widget_translate_coordinates(w, D.win, 0, 0, &x, &y)) return;
    g_string_append_printf(s, ", \"%s\": [%d, %d, %d, %d]", name, x, y, gtk_widget_get_allocated_width(w), gtk_widget_get_allocated_height(w));
}

/* JSON string contents: quotes, backslashes and control characters
 * escaped, UTF-8 kept as it is. */
static char *json_escape(const char *t)
{
    GString *s = g_string_new(NULL);
    for (const unsigned char *p = (const unsigned char *)t; *p; p++) {
        if (*p == '"' || *p == '\\') g_string_append_printf(s, "\\%c", *p);
        else if (*p < 0x20) g_string_append_printf(s, "\\u%04x", *p);
        else g_string_append_c(s, (char)*p);
    }
    return g_string_free(s, FALSE);
}

static void write_state(void)
{
    const char *path = g_getenv("NEXA_DECK_STATE");
    if (!path || !D.win) return;
    GString *s = g_string_new(NULL);
    char *err_esc = json_escape(D.error);
    g_string_append_printf(s, "{\"closing\": %d, \"animations\": %d, \"networks\": %d, \"bt_devices\": %d, \"tunnels\": %d, \"notes\": %d, "
                           "\"volume\": %d, \"muted\": %d, \"brightness\": %d, \"quiet\": %d, \"playing\": %d, \"error\": \"%s\"}\n",
                           D.closing, D.animations, D.wifi.nnet, D.bt.ndev, D.vpn.n, D.sig.n, D.sound.percent, D.sound.muted, D.bright.percent,
                           D.quiet.on, D.media.playing, err_esc);
    g_free(err_esc);
    for (int i = 0; i < NCARDS; i++) {
        struct card *c = &D.c[i];
        char *sub = json_escape(gtk_label_get_text(GTK_LABEL(c->sub)));
        char *title = json_escape(gtk_label_get_text(GTK_LABEL(c->title)));
        g_string_append_printf(s, "{\"card\": \"%s\", \"shown\": %d, \"lit\": %d, \"open\": %d, \"title\": \"%s\", \"sub\": \"%s\"",
                               KEYS[i], c->shown, c->lit, c->open, title, sub);
        g_free(sub);
        g_free(title);
        rect_of(s, "rect", c->frame);
        rect_of(s, "head", c->head);
        rect_of(s, "switch", c->sw);
        if (i == C_SOUND) rect_of(s, "slider", D.sound.scale);
        if (i == C_PLAYING) rect_of(s, "play", D.media.play);
        if (i == C_SIGNALS) rect_of(s, "clear", D.sig.clear);
        if (i == C_WIFI) {
            g_string_append(s, ", \"rows\": [");
            for (int k = 0; k < D.wifi.nnet; k++) {
                int x = 0, y = 0;
                GtkWidget *w = D.wifi.rows[k];
                if (w && gtk_widget_get_mapped(w)) gtk_widget_translate_coordinates(w, D.win, 0, 0, &x, &y);
                char *nm = json_escape(D.wifi.net[k].name);
                g_string_append_printf(s, "%s{\"name\": \"%s\", \"rect\": [%d, %d, %d, %d]}", k ? ", " : "", nm, x, y,
                                       w ? gtk_widget_get_allocated_width(w) : 0, w ? gtk_widget_get_allocated_height(w) : 0);
                g_free(nm);
            }
            g_string_append(s, "]");
        }
        if (i == C_BRIGHT) rect_of(s, "slider", D.bright.scale);
        g_string_append(s, "}\n");
    }
    char *tmp = g_strconcat(path, ".tmp", NULL);
    if (g_file_set_contents(tmp, s->str, (gssize)s->len, NULL)) rename(tmp, path);
    g_free(tmp);
    g_string_free(s, TRUE);
}

/* Once per change of layout (the slide-in, a card opening), not per widget. */
static guint state_idle;
static gboolean state_now(gpointer d)
{
    state_idle = 0;
    write_state();
    return G_SOURCE_REMOVE;
}
static void on_layout(GtkWidget *w, GdkRectangle *a, gpointer d)
{
    if (!state_idle && g_getenv("NEXA_DECK_STATE")) state_idle = g_idle_add(state_now, NULL);
}

/* ---------- open and close ---------- */

static gboolean destroy_now(gpointer d)
{
    if (D.win) gtk_widget_destroy(D.win);
    D.win = NULL;
    return G_SOURCE_REMOVE;
}

static void close_deck(void)
{
    if (!D.win || D.closing) return;
    D.closing = 1;
    if (D.tick) g_source_remove(D.tick), D.tick = 0;
    write_state();
    for (int i = 0; i < NCARDS; i++) {
        gtk_revealer_set_transition_duration(GTK_REVEALER(D.c[i].revealer), D.animations ? CLOSE_MS : 0);
        gtk_revealer_set_reveal_child(GTK_REVEALER(D.c[i].revealer), FALSE);
    }
    g_timeout_add(D.animations ? CLOSE_MS + 20 : 0, destroy_now, NULL);
}

static gboolean on_key(GtkWidget *w, GdkEventKey *e, gpointer d)
{
    if (e->keyval == GDK_KEY_Escape) {
        close_deck();
        return TRUE;
    }
    return FALSE;
}

/* A click that no card took: outside the cards. */
static gboolean on_outside(GtkWidget *w, GdkEventButton *e, gpointer d)
{
    close_deck();
    return TRUE;
}

static gboolean reveal_one(gpointer data)
{
    int i = GPOINTER_TO_INT(data);
    if (D.win && !D.closing && D.c[i].shown) gtk_revealer_set_reveal_child(GTK_REVEALER(D.c[i].revealer), TRUE);
    return G_SOURCE_REMOVE;
}

/* Cards slide in from the edge one after another. */
static void stagger_in(void)
{
    for (int i = 0, k = 0; i < NCARDS; i++) {
        gtk_revealer_set_reveal_child(GTK_REVEALER(D.c[i].revealer), FALSE);
        g_timeout_add(D.animations ? (guint)(k++ * STAGGER_MS) : 0, reveal_one, GINT_TO_POINTER(i));
    }
}

static void build_window(void)
{
    D.win = gtk_application_window_new(D.app);
    gtk_window_set_title(GTK_WINDOW(D.win), "Deck");
    GdkVisual *v = gdk_screen_get_rgba_visual(gtk_widget_get_screen(D.win));
    if (v) gtk_widget_set_visual(D.win, v);
    gtk_widget_set_app_paintable(D.win, TRUE);
    gtk_layer_init_for_window(GTK_WINDOW(D.win));
    gtk_layer_set_namespace(GTK_WINDOW(D.win), "nexa-deck");
    gtk_layer_set_layer(GTK_WINDOW(D.win), GTK_LAYER_SHELL_LAYER_OVERLAY);
    gtk_layer_set_keyboard_mode(GTK_WINDOW(D.win), GTK_LAYER_SHELL_KEYBOARD_MODE_EXCLUSIVE);
    gtk_layer_set_exclusive_zone(GTK_WINDOW(D.win), 0);  /* below the bar, over everything else */
    for (int e = 0; e < GTK_LAYER_SHELL_EDGE_ENTRY_NUMBER; e++) gtk_layer_set_anchor(GTK_WINDOW(D.win), e, TRUE);
    g_signal_connect(D.win, "key-press-event", G_CALLBACK(on_key), NULL);

    GtkWidget *outside = gtk_event_box_new();
    gtk_style_context_add_class(gtk_widget_get_style_context(outside), "deck-root");
    g_signal_connect(outside, "button-press-event", G_CALLBACK(on_outside), NULL);
    gtk_container_add(GTK_CONTAINER(D.win), outside);
    D.scroller = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(D.scroller), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    gtk_scrolled_window_set_propagate_natural_height(GTK_SCROLLED_WINDOW(D.scroller), TRUE);
    gtk_scrolled_window_set_overlay_scrolling(GTK_SCROLLED_WINDOW(D.scroller), TRUE);
    gtk_widget_set_halign(D.scroller, GTK_ALIGN_END);
    gtk_widget_set_valign(D.scroller, GTK_ALIGN_START);
    g_object_set(D.scroller, "margin-top", 8, "margin-end", 8, "margin-bottom", 8, NULL);
    gtk_container_add(GTK_CONTAINER(outside), D.scroller);
    D.column = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    gtk_container_add(GTK_CONTAINER(D.scroller), D.column);
    g_signal_connect(D.scroller, "size-allocate", G_CALLBACK(on_layout), NULL);

    memset(D.c, 0, sizeof D.c);
    build_now();
    make_card(C_WIFI, "network-wireless-symbolic", "Wi-Fi", 1, 1);
    D.c[C_WIFI].sw_handler = g_signal_connect(D.c[C_WIFI].sw, "notify::active", G_CALLBACK(on_wifi_switch), NULL);
    make_card(C_BT, "bluetooth-active-symbolic", "Bluetooth", 1, 1);
    D.c[C_BT].sw_handler = g_signal_connect(D.c[C_BT].sw, "notify::active", G_CALLBACK(on_bt_switch), NULL);
    build_sound();
    build_bright();
    make_card(C_VPN, "network-vpn-symbolic", "VPN", 1, 1);
    D.c[C_VPN].sw_handler = g_signal_connect(D.c[C_VPN].sw, "notify::active", G_CALLBACK(on_vpn_switch), NULL);
    make_card(C_QUIET, "notifications-disabled-symbolic", "Quiet", 1, 0);
    D.c[C_QUIET].sw_handler = g_signal_connect(D.c[C_QUIET].sw, "notify::active", G_CALLBACK(on_quiet_switch), NULL);
    build_playing();
    build_signals();
    gtk_widget_show_all(D.win);
    stagger_in();
    refresh();
    D.tick = g_timeout_add_seconds(2, tick, NULL);
    if (D.wanted >= 0) set_card_open(D.wanted, 1);
}

static int key_index(const char *k)
{
    for (int i = 0; k && i < NCARDS; i++)
        if (!strcmp(k, KEYS[i])) return i;
    return -1;
}

static int on_command_line(GApplication *app, GApplicationCommandLine *cl, gpointer d)
{
    int argc;
    gchar **argv = g_application_command_line_get_arguments(cl, &argc);
    int want = argc > 1 ? key_index(argv[1]) : -1;
    if (argc > 1 && want < 0) g_application_command_line_printerr(cl, "nexa-deck: no card called %s\n", argv[1]);
    g_strfreev(argv);
    if (D.win && !D.closing) {
        /* open again: close, unless another card was asked for */
        if (want < 0 || D.c[want].open) close_deck();
        else set_card_open(want, 1);
    } else if (!D.win) {
        D.wanted = want;
        D.closing = 0;
        build_window();
    }
    return 0;
}

int main(int argc, char **argv)
{
    const char *cfg = g_getenv("XDG_CONFIG_HOME");
    if (cfg && *cfg) snprintf(D.theme_path, sizeof D.theme_path, "%s/nexa/lock.conf", cfg);
    else snprintf(D.theme_path, sizeof D.theme_path, "%s/.config/nexa/lock.conf", g_get_home_dir());
    D.app = gtk_application_new("os.nexa.Deck", G_APPLICATION_HANDLES_COMMAND_LINE);
    g_signal_connect(D.app, "command-line", G_CALLBACK(on_command_line), NULL);
    g_signal_connect_swapped(D.app, "startup", G_CALLBACK(load_theme), NULL);
    g_signal_connect_swapped(D.app, "startup", G_CALLBACK(apply_css), NULL);
    D.sys = g_bus_get_sync(G_BUS_TYPE_SYSTEM, NULL, NULL);
    D.ses = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, NULL);
    gtk_init(&argc, &argv);
    if (!gtk_layer_is_supported()) {
        fprintf(stderr, "nexa-deck: this session has no layer-shell (needs Sway or Hyprland)\n");
        return 1;
    }
    gboolean anim = TRUE;
    g_object_get(gtk_settings_get_default(), "gtk-enable-animations", &anim, NULL);
    D.animations = anim;
    D.wanted = -1;
    int r = g_application_run(G_APPLICATION(D.app), argc, argv);
    g_object_unref(D.app);
    return r;
}
