/* Photos: a NexaOS app in C (GTK 3), for the pictures in ~/Pictures.
 *
 *   photos [FILE]     the library, or FILE in the viewer
 *
 * The library is every picture under ~/Pictures, newest first, by month;
 * Photos finds new ones by itself. Favourites and albums are Photos' own
 * (the files stay where they are), and so is turning a photo: the file is
 * never rewritten. Deleted photos wait 30 days in Deleted (an installed app
 * can't reach the system Trash). Installed, it runs confined to
 * files.pictures by nexa-run.
 *
 * Library: click opens, Ctrl+click / Shift+click select, Ctrl+A all,
 * Delete, F favourite, Ctrl+F search, F5 slideshow.
 * Viewer: Left/Right, Home/End, +/-/0/1 zoom, drag to move, R / Shift+R
 * turn, F favourite, I details, Delete, F5 slideshow (Space pauses),
 * F11 fullscreen, Esc back.
 *
 * PHOTOS_TEST=<dir>: for tests only, writes <dir>/state.json after changes
 * and takes commands from the pipe <dir>/cmd (see tests/headless.py). */
#include <errno.h>
#include <fcntl.h>
#include <glib/gstdio.h>
#include <gtk/gtk.h>
#include <math.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "exif.h"
#include "library.h"
#include "store.h"
#include "thumbs.h"

#define APP_ID "photos"
#define TILE 160
#define THUMB_MEMORY 700        /* thumbnails kept in memory */
#define SLIDE_MS 4000
#define FADE_MS 320
#define TOAST_MS 7000
#define MAX_MONITORS 256

enum { COLL_LIBRARY, COLL_FAVOURITES, COLL_ALBUM };

typedef struct {
    cairo_surface_t *s;
    guint64 used;
} Thumb;

typedef struct {
    GPtrArray *all;            /* Photo*, newest first (owned) */
    GHashTable *by_path;       /* path -> Photo* */
    Store *store;
    char *root, *cache, *data;
    gboolean scanned, scanning, rescan_again;
    gint cancel;
    GPtrArray *monitors;
    guint rescan_id;

    GtkWidget *win, *stack, *nav[3], *search;
    gboolean fullscreen, animations;
    int restore_w, restore_h;

    /* the grid page: library, favourites or an album */
    int coll;
    char *album;               /* its name, for COLL_ALBUM */
    GtkWidget *grid_scroll, *grid_box, *grid_head, *grid_sub, *grid_empty, *grid_back, *album_tools, *slide_btn;
    GPtrArray *shown;          /* Photo* listed (not owned) */
    GHashTable *tile_of;       /* path -> tile drawing area */
    GHashTable *selected;      /* paths (owned) */
    int anchor;                /* last clicked, for Shift+click */
    GtkWidget *selbar, *sel_label, *sel_remove, *sel_fav;

    /* albums */
    GtkWidget *albums_flow;
    /* Deleted */
    GtkWidget *del_list, *del_empty, *del_sub, *del_tools;

    /* viewer */
    GtkWidget *view_area, *view_top, *view_name, *view_when, *fav_btn, *info_rev, *info_grid, *view_msg, *view_msg_box, *view_count;
    GPtrArray *view_list;      /* Photo* stepped through (not owned) */
    Photo *extra;              /* a photo opened from outside the library (owned) */
    int extra_turn;
    int cur;
    char *cur_path;
    cairo_surface_t *pic, *old_pic;
    char *pic_key, *why;
    guint load_gen;
    gboolean fit, dragging;
    double zoom, pan_x, pan_y, drag_x, drag_y, fade_from;
    gint64 fade_at;
    guint fade_id, slide_id;
    gboolean slideshow, paused;

    /* thumbnails */
    GThreadPool *pool;
    GHashTable *thumbs;        /* key -> Thumb* */
    GHashTable *wanted;        /* keys being made */
    guint64 use_clock;
    int tile_px;

    /* the toast */
    GtkWidget *toast_rev, *toast_label, *toast_undo;
    guint toast_id;
    GPtrArray *undo;           /* Gone* that Undo puts back (not owned) */

    char *test_dir;
    guint state_id;
} App;

static App app;

static void show_grid(void);
static void fill_grid(void);
static void fill_albums(void);
static void fill_deleted(void);
static void open_viewer(GPtrArray *list, int index);
static void show_photo(void);
static void state_changed(void);
static void toast(const char *text, gboolean undo);
static void leave_viewer(void);
static void rescan(void);

/* ---- small helpers ---- */

static void destroy_child(GtkWidget *w, gpointer d)
{
    (void)d;
    gtk_widget_destroy(w);
}

static void add_class(GtkWidget *w, const char *c) { gtk_style_context_add_class(gtk_widget_get_style_context(w), c); }

static GtkWidget *label(const char *text, const char *cls, float xalign)
{
    GtkWidget *l = gtk_label_new(text);
    if (cls) add_class(l, cls);
    gtk_label_set_xalign(GTK_LABEL(l), xalign);
    gtk_label_set_line_wrap(GTK_LABEL(l), TRUE);
    gtk_label_set_line_wrap_mode(GTK_LABEL(l), PANGO_WRAP_WORD_CHAR);
    return l;
}

/* Built, then hidden until needed (its insides shown, so showing it later
 * shows them too). */
static void keep_hidden(GtkWidget *w)
{
    gtk_widget_show_all(w);
    gtk_widget_hide(w);
    gtk_widget_set_no_show_all(w, TRUE);
}

static GtkWidget *button(const char *text, const char *tip, GCallback cb, gpointer data, const char *cls)
{
    GtkWidget *b = gtk_button_new_with_label(text);
    if (tip) gtk_widget_set_tooltip_text(b, tip);
    if (cls) add_class(b, cls);
    if (cb) g_signal_connect(b, "clicked", cb, data);
    return b;
}

static void hex_path(cairo_t *cr, double cx, double cy, double r)
{
    for (int i = 0; i < 6; i++) {
        double a = (-90 + 60 * i) * G_PI / 180.0;
        if (i == 0) cairo_move_to(cr, cx + r * cos(a), cy + r * sin(a));
        else cairo_line_to(cr, cx + r * cos(a), cy + r * sin(a));
    }
    cairo_close_path(cr);
}

static void rounded(cairo_t *cr, double x, double y, double w, double h, double r)
{
    cairo_new_sub_path(cr);
    cairo_arc(cr, x + w - r, y + r, r, -G_PI / 2, 0);
    cairo_arc(cr, x + w - r, y + h - r, r, 0, G_PI / 2);
    cairo_arc(cr, x + r, y + h - r, r, G_PI / 2, G_PI);
    cairo_arc(cr, x + r, y + r, r, G_PI, 3 * G_PI / 2);
    cairo_close_path(cr);
}

/* A theme colour (@nexa_*) as the style context sees it. */
static GdkRGBA colour(GtkWidget *w, const char *name, const char *fallback)
{
    GdkRGBA c;
    if (!gtk_style_context_lookup_color(gtk_widget_get_style_context(w), name, &c)) gdk_rgba_parse(&c, fallback);
    return c;
}

static int orientation_of(const Photo *p)
{
    int turn = p == app.extra ? app.extra_turn : store_turn(app.store, p->rel);
    return orient_turn(p->info.orientation, turn);
}

static char *thumb_key(const Photo *p, int o) { return g_strdup_printf("%d\n%s", o, p->path); }

static Photo *photo_copy(const Photo *p)
{
    Photo *c = g_new0(Photo, 1);
    *c = *p;
    c->path = g_strdup(p->path);
    c->rel = g_strdup(p->rel);
    c->name = g_strdup(p->name);
    return c;
}

static void save_store(void)
{
    GError *err = NULL;
    if (!store_save(app.store, &err)) {
        toast("Photos couldn't save your albums. Is the disk full?", FALSE);
        g_clear_error(&err);
    }
}

static gboolean reduced_motion(void)
{
    gboolean on = TRUE;
    g_object_get(gtk_settings_get_default(), "gtk-enable-animations", &on, NULL);
    return !on;
}

/* ---- window buttons (as in Video Player: ask the compositor) ---- */

static gboolean wm_run(const char *const *argv)
{
    char *out = NULL;
    int status = 0;
    gboolean ok = g_spawn_sync(NULL, (char **)argv, NULL, G_SPAWN_SEARCH_PATH | G_SPAWN_STDERR_TO_DEV_NULL, NULL, NULL, &out,
                               NULL, &status, NULL) &&
                  g_spawn_check_wait_status(status, NULL) && out && (strstr(out, "ok") || strstr(out, "\"success\": true"));
    g_free(out);
    return ok;
}

static void on_minimize(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    char sel[64];
    gboolean done = FALSE;
    if (g_getenv("HYPRLAND_INSTANCE_SIGNATURE")) {
        g_snprintf(sel, sizeof sel, "special:minimized,pid:%d", (int)getpid());
        const char *argv[] = {"hyprctl", "dispatch", "movetoworkspacesilent", sel, NULL};
        done = wm_run(argv);
    } else if (g_getenv("SWAYSOCK")) {
        g_snprintf(sel, sizeof sel, "[pid=%d] move scratchpad", (int)getpid());
        const char *argv[] = {"swaymsg", sel, NULL};
        done = wm_run(argv);
    }
    if (!done) gtk_window_iconify(GTK_WINDOW(app.win));
}

static void on_maximize(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    char sel[160];
    gboolean done = FALSE;
    if (g_getenv("HYPRLAND_INSTANCE_SIGNATURE")) {
        g_snprintf(sel, sizeof sel, "pid:%d", (int)getpid());
        const char *focus[] = {"hyprctl", "dispatch", "focuswindow", sel, NULL};
        const char *max[] = {"hyprctl", "dispatch", "fullscreen", "1", NULL};
        done = wm_run(focus) && wm_run(max);
    } else if (g_getenv("SWAYSOCK")) {
        if (app.restore_w > 0) {
            g_snprintf(sel, sizeof sel, "[pid=%d] resize set width %d px height %d px, move position center", (int)getpid(),
                       app.restore_w, app.restore_h);
        } else {
            int w, h;
            gtk_window_get_size(GTK_WINDOW(app.win), &w, &h);
            GtkWidget *bar = gtk_window_get_titlebar(GTK_WINDOW(app.win));
            app.restore_w = w;
            app.restore_h = h + (bar ? gtk_widget_get_allocated_height(bar) : 0);
            g_snprintf(sel, sizeof sel, "[pid=%d] resize set width 100 ppt height 100 ppt, move position 0 0", (int)getpid());
        }
        const char *argv[] = {"swaymsg", sel, NULL};
        done = wm_run(argv);
        if (strstr(sel, "center") || !done) app.restore_w = app.restore_h = 0;
    }
    if (!done) {
        if (gtk_window_is_maximized(GTK_WINDOW(app.win))) gtk_window_unmaximize(GTK_WINDOW(app.win));
        else gtk_window_maximize(GTK_WINDOW(app.win));
    }
}

static void on_close(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    gtk_window_close(GTK_WINDOW(app.win));
}

static GtkWidget *title_button(const char *icon, const char *tip, const char *role, GCallback cb)
{
    GtkWidget *btn = gtk_button_new_from_icon_name(icon, GTK_ICON_SIZE_MENU);
    gtk_widget_set_tooltip_text(btn, tip);
    gtk_widget_set_can_focus(btn, FALSE);
    gtk_widget_set_valign(btn, GTK_ALIGN_CENTER);
    add_class(btn, "titlebutton");
    add_class(btn, role);
    g_signal_connect(btn, "clicked", cb, NULL);
    return btn;
}

static void fit_to_screen(GtkWidget *win, gpointer data)
{
    (void)data;
    GdkWindow *gw = gtk_widget_get_window(win);
    GdkDisplay *d = gdk_display_get_default();
    if (!gw || !d) return;
    GdkMonitor *m = gdk_display_get_monitor_at_window(d, gw);
    if (!m) m = gdk_display_get_monitor(d, 0);
    if (!m) return;
    GdkRectangle area;
    gdk_monitor_get_workarea(m, &area);
    int w, h;
    gtk_window_get_default_size(GTK_WINDOW(win), &w, &h);
    double grow = CLAMP(MIN(area.width / 1366.0, area.height / 740.0), 1.0, 1.5);
    int fw = MIN((int)(w * grow), (int)(area.width * 0.92)), fh = MIN((int)(h * grow), (int)(area.height * 0.92));
    if (fw != w || fh != h) {
        gtk_window_set_default_size(GTK_WINDOW(win), fw, fh);
        gtk_window_resize(GTK_WINDOW(win), fw, fh);
    }
}

/* ---- thumbnails, made off the main thread ---- */

typedef struct {
    Photo *p;                  /* a copy */
    int o, px;
    char *key;
    cairo_surface_t *s;
} ThumbJob;

static gboolean thumb_done(gpointer data)
{
    ThumbJob *j = data;
    g_hash_table_remove(app.wanted, j->key);
    {  /* (one that can't be made is noted too, so it isn't asked for again) */
        Thumb *t = g_new0(Thumb, 1);
        t->s = j->s;
        t->used = ++app.use_clock;
        g_hash_table_replace(app.thumbs, g_strdup(j->key), t);
        GtkWidget *tile = app.tile_of ? g_hash_table_lookup(app.tile_of, j->p->path) : NULL;
        if (tile) gtk_widget_queue_draw(tile);
        if (app.view_area && !app.pic && app.cur_path && !strcmp(app.cur_path, j->p->path)) gtk_widget_queue_draw(app.view_area);
        state_changed();
    }
    photo_free(j->p);
    g_free(j->key);
    g_free(j);
    return G_SOURCE_REMOVE;
}

static void thumb_work(gpointer data, gpointer user)
{
    (void)user;
    ThumbJob *j = data;
    cairo_surface_t *sq = thumb_square(j->p, j->o, app.cache);
    if (sq && j->px != THUMB_PX) {  /* kept in memory at the size it is drawn */
        cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_RGB24, j->px, j->px);
        cairo_t *cr = cairo_create(s);
        cairo_scale(cr, (double)j->px / THUMB_PX, (double)j->px / THUMB_PX);
        cairo_set_source_surface(cr, sq, 0, 0);
        cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_GOOD);
        cairo_paint(cr);
        cairo_destroy(cr);
        cairo_surface_destroy(sq);
        sq = s;
    }
    j->s = sq;
    g_idle_add(thumb_done, j);
}

/* Drop the thumbnails not drawn for longest when too many are kept. */
static gint by_age(gconstpointer a, gconstpointer b)
{
    guint64 x = *(const guint64 *)a, y = *(const guint64 *)b;
    return x < y ? -1 : x > y;
}

static void thumb_free(gpointer d)
{
    Thumb *t = d;
    if (t->s) cairo_surface_destroy(t->s);
    g_free(t);
}

static void thumbs_trim(void)
{
    guint n = g_hash_table_size(app.thumbs);
    if (n <= THUMB_MEMORY) return;
    GArray *ages = g_array_sized_new(FALSE, FALSE, sizeof(guint64), n);
    GHashTableIter it;
    gpointer k, v;
    g_hash_table_iter_init(&it, app.thumbs);
    while (g_hash_table_iter_next(&it, &k, &v)) g_array_append_val(ages, ((Thumb *)v)->used);
    g_array_sort(ages, by_age);
    guint64 cut = g_array_index(ages, guint64, n / 5);  /* the oldest fifth goes */
    g_array_free(ages, TRUE);
    g_hash_table_iter_init(&it, app.thumbs);
    while (g_hash_table_iter_next(&it, &k, &v))
        if (((Thumb *)v)->used <= cut) g_hash_table_iter_remove(&it);
}

/* The thumbnail if it is ready; otherwise it is asked for. */
static cairo_surface_t *thumb_want(const Photo *p)
{
    int o = orientation_of(p);
    char *key = thumb_key(p, o);
    Thumb *t = g_hash_table_lookup(app.thumbs, key);
    if (t) {
        t->used = ++app.use_clock;
        g_free(key);
        return t->s;
    }
    if (!g_hash_table_contains(app.wanted, key)) {
        ThumbJob *j = g_new0(ThumbJob, 1);
        j->p = photo_copy(p);
        j->o = o;
        j->px = app.tile_px;
        j->key = g_strdup(key);
        g_hash_table_add(app.wanted, g_strdup(key));
        g_thread_pool_push(app.pool, j, NULL);
        thumbs_trim();
    }
    g_free(key);
    return NULL;
}

/* ---- the grid ---- */

static Photo *tile_photo(GtkWidget *tile) { return g_object_get_data(G_OBJECT(tile), "photo"); }

static gboolean draw_tile(GtkWidget *w, cairo_t *cr, gpointer d)
{
    (void)d;
    Photo *p = tile_photo(w);
    if (!p) return TRUE;
    int W = gtk_widget_get_allocated_width(w), H = gtk_widget_get_allocated_height(w);
    gboolean sel = g_hash_table_contains(app.selected, p->path);
    double inset = sel ? 8 : 0;
    rounded(cr, inset, inset, W - 2 * inset, H - 2 * inset, 10);
    cairo_save(cr);
    cairo_clip(cr);
    cairo_surface_t *s = thumb_want(p);
    if (s) {
        double k = (double)(W - 2 * inset) / cairo_image_surface_get_width(s);
        cairo_translate(cr, inset, inset);
        cairo_scale(cr, k, k);
        cairo_set_source_surface(cr, s, 0, 0);
        cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_GOOD);
        cairo_paint(cr);
    } else {
        GdkRGBA c = colour(w, "nexa_surface_hi", "#222a46");
        gdk_cairo_set_source_rgba(cr, &c);
        cairo_paint(cr);
    }
    cairo_restore(cr);
    GtkStateFlags st = gtk_widget_get_state_flags(gtk_widget_get_parent(w));
    if (st & (GTK_STATE_FLAG_PRELIGHT | GTK_STATE_FLAG_FOCUSED)) {
        GdkRGBA c = colour(w, "nexa_accent", "#4da3ff");
        rounded(cr, 1.5, 1.5, W - 3, H - 3, 10);
        cairo_set_source_rgba(cr, c.red, c.green, c.blue, st & GTK_STATE_FLAG_FOCUSED ? 0.9 : 0.45);
        cairo_set_line_width(cr, 2);
        cairo_stroke(cr);
    }
    if (sel) {
        GdkRGBA c = colour(w, "nexa_accent", "#4da3ff"), on = colour(w, "nexa_on_accent", "#080c1e");
        rounded(cr, 2, 2, W - 4, H - 4, 12);
        gdk_cairo_set_source_rgba(cr, &c);
        cairo_set_line_width(cr, 3);
        cairo_stroke(cr);
        hex_path(cr, 20, 20, 11);
        gdk_cairo_set_source_rgba(cr, &c);
        cairo_fill(cr);
        cairo_move_to(cr, 15, 20);
        cairo_line_to(cr, 19, 24);
        cairo_line_to(cr, 25.5, 16.5);
        gdk_cairo_set_source_rgba(cr, &on);
        cairo_set_line_width(cr, 2.2);
        cairo_stroke(cr);
    }
    if (store_is_fav(app.store, p->rel)) {  /* a favourite: a small filled hexagon */
        GdkRGBA c = colour(w, "nexa_accent", "#4da3ff");
        hex_path(cr, W - 18 - inset, H - 18 - inset, 8);
        cairo_set_source_rgba(cr, 0, 0, 0, 0.35);
        cairo_set_line_width(cr, 4);
        cairo_stroke_preserve(cr);
        gdk_cairo_set_source_rgba(cr, &c);
        cairo_fill(cr);
    }
    return TRUE;
}

static int shown_index(const Photo *p)
{
    for (guint i = 0; app.shown && i < app.shown->len; i++)
        if (app.shown->pdata[i] == p) return (int)i;
    return -1;
}

static void redraw_tiles(void)
{
    GHashTableIter it;
    gpointer v;
    g_hash_table_iter_init(&it, app.tile_of);
    while (g_hash_table_iter_next(&it, NULL, &v)) gtk_widget_queue_draw(v);
}

static void update_selbar(void)
{
    guint n = g_hash_table_size(app.selected);
    gtk_revealer_set_reveal_child(GTK_REVEALER(app.selbar), n > 0);
    char *t = g_strdup_printf(n == 1 ? "%u photo selected" : "%u photos selected", n);
    gtk_label_set_text(GTK_LABEL(app.sel_label), t);
    g_free(t);
    gtk_widget_set_visible(app.sel_remove, app.coll == COLL_ALBUM || app.coll == COLL_FAVOURITES);
    gtk_button_set_label(GTK_BUTTON(app.sel_remove), app.coll == COLL_FAVOURITES ? "Not a Favourite" : "Remove from Album");
    gtk_widget_set_visible(app.sel_fav, app.coll != COLL_FAVOURITES);
    redraw_tiles();
    state_changed();
}

static void select_photo(const Photo *p, gboolean on)
{
    if (on) g_hash_table_add(app.selected, g_strdup(p->path));
    else g_hash_table_remove(app.selected, p->path);
}

static void clear_selection(void)
{
    g_hash_table_remove_all(app.selected);
    update_selbar();
}

static void tile_clicked(Photo *p, gboolean ctrl, gboolean shift)
{
    int i = shown_index(p);
    if (i < 0) return;
    if (shift && app.anchor >= 0) {
        int a = MIN(app.anchor, i), b = MAX(app.anchor, i);
        for (int k = a; k <= b && k < (int)app.shown->len; k++) select_photo(app.shown->pdata[k], TRUE);
        update_selbar();
    } else if (ctrl || g_hash_table_size(app.selected)) {
        select_photo(p, !g_hash_table_contains(app.selected, p->path));
        app.anchor = i;
        update_selbar();
    } else {
        app.anchor = i;
        open_viewer(app.shown, i);
    }
}

static gboolean on_tile_press(GtkWidget *w, GdkEventButton *e, gpointer d)
{
    (void)d;
    if (e->type != GDK_BUTTON_PRESS || e->button != 1) return FALSE;
    gtk_widget_grab_focus(gtk_widget_get_parent(w));
    tile_clicked(tile_photo(w), (e->state & GDK_CONTROL_MASK) != 0, (e->state & GDK_SHIFT_MASK) != 0);
    return TRUE;
}

static void on_tile_activated(GtkFlowBox *box, GtkFlowBoxChild *child, gpointer d)
{
    (void)box; (void)d;
    GtkWidget *tile = gtk_bin_get_child(GTK_BIN(child));
    Photo *p = tile_photo(tile);
    if (p) tile_clicked(p, FALSE, FALSE);
}

static GtkWidget *new_tile(Photo *p)
{
    GtkWidget *a = gtk_drawing_area_new();
    gtk_widget_set_size_request(a, TILE, TILE);
    gtk_widget_add_events(a, GDK_BUTTON_PRESS_MASK);
    g_object_set_data(G_OBJECT(a), "photo", p);
    g_signal_connect(a, "draw", G_CALLBACK(draw_tile), NULL);
    g_signal_connect(a, "button-press-event", G_CALLBACK(on_tile_press), NULL);
    char *tip = photo_when(p);
    char *t = g_strdup_printf("%s\n%s", p->name, tip);
    gtk_widget_set_tooltip_text(a, t);
    g_free(t);
    g_free(tip);
    g_hash_table_replace(app.tile_of, g_strdup(p->path), a);
    return a;
}

static const char *query(void) { return gtk_entry_get_text(GTK_ENTRY(app.search)); }

/* The photos the grid lists now. */
static void collect(void)
{
    if (app.shown) g_ptr_array_free(app.shown, TRUE);
    app.shown = g_ptr_array_new();
    Album *alb = app.coll == COLL_ALBUM ? store_album_find(app.store, app.album) : NULL;
    if (app.coll == COLL_ALBUM && alb) {
        for (guint i = 0; i < alb->items->len; i++) {
            char *path = g_build_filename(app.root, (char *)alb->items->pdata[i], NULL);
            Photo *p = g_hash_table_lookup(app.by_path, path);
            if (p) g_ptr_array_add(app.shown, p);
            g_free(path);
        }
        return;
    }
    const char *q = app.coll == COLL_LIBRARY ? query() : "";
    for (guint i = 0; i < app.all->len; i++) {
        Photo *p = app.all->pdata[i];
        if (app.coll == COLL_FAVOURITES && !store_is_fav(app.store, p->rel)) continue;
        if (*q) {
            char *albums = store_albums_of(app.store, p->rel);
            gboolean ok = photo_matches(p, q, albums);
            g_free(albums);
            if (!ok) continue;
        }
        g_ptr_array_add(app.shown, p);
    }
}

static GtkWidget *new_flow(void)
{
    GtkWidget *f = gtk_flow_box_new();
    gtk_flow_box_set_homogeneous(GTK_FLOW_BOX(f), TRUE);
    gtk_flow_box_set_selection_mode(GTK_FLOW_BOX(f), GTK_SELECTION_NONE);
    gtk_flow_box_set_min_children_per_line(GTK_FLOW_BOX(f), 2);
    gtk_flow_box_set_max_children_per_line(GTK_FLOW_BOX(f), 40);
    gtk_flow_box_set_column_spacing(GTK_FLOW_BOX(f), 8);
    gtk_flow_box_set_row_spacing(GTK_FLOW_BOX(f), 8);
    gtk_flow_box_set_activate_on_single_click(GTK_FLOW_BOX(f), FALSE);
    g_signal_connect(f, "child-activated", G_CALLBACK(on_tile_activated), NULL);
    return f;
}

static void set_heading(const char *head, const char *sub)
{
    gtk_label_set_text(GTK_LABEL(app.grid_head), head);
    gtk_label_set_text(GTK_LABEL(app.grid_sub), sub);
}

static char *count_text(guint n, const char *one, const char *many)
{
    char *num = g_strdup_printf("%u", n);
    if (n >= 1000) {  /* 1,234 */
        GString *s = g_string_new(NULL);
        int len = (int)strlen(num);
        for (int i = 0; i < len; i++) {
            if (i && (len - i) % 3 == 0) g_string_append_c(s, ',');
            g_string_append_c(s, num[i]);
        }
        g_free(num);
        num = g_string_free(s, FALSE);
    }
    char *t = g_strdup_printf("%s %s", num, n == 1 ? one : many);
    g_free(num);
    return t;
}

static void fill_grid(void)
{
    GtkAdjustment *adj = gtk_scrolled_window_get_vadjustment(GTK_SCROLLED_WINDOW(app.grid_scroll));
    double keep = gtk_adjustment_get_value(adj);
    gtk_container_foreach(GTK_CONTAINER(app.grid_box), destroy_child, NULL);
    g_hash_table_remove_all(app.tile_of);
    collect();
    /* Selected photos that went away are no longer selected. */
    GHashTable *still = g_hash_table_new(g_str_hash, g_str_equal);
    for (guint i = 0; i < app.shown->len; i++) g_hash_table_add(still, ((Photo *)app.shown->pdata[i])->path);
    GHashTableIter it;
    gpointer k;
    g_hash_table_iter_init(&it, app.selected);
    while (g_hash_table_iter_next(&it, &k, NULL))
        if (!g_hash_table_contains(still, k)) g_hash_table_iter_remove(&it);
    g_hash_table_destroy(still);

    const char *q = query();
    char *sub = count_text(app.shown->len, "photo", "photos");
    gtk_widget_set_visible(app.grid_back, app.coll != COLL_LIBRARY);
    gtk_widget_set_visible(app.album_tools, app.coll == COLL_ALBUM);
    gtk_widget_set_sensitive(app.slide_btn, app.shown->len > 0);
    const char *empty = NULL;
    char *emptybuf = NULL;
    if (app.coll == COLL_ALBUM) {
        set_heading(app.album, sub);
        empty = "This album is empty. In the library, select photos (Ctrl+click), then Add to Album.";
    } else if (app.coll == COLL_FAVOURITES) {
        set_heading("Favourites", sub);
        empty = "No favourites yet. Press F, or the hexagon, on a photo you love.";
    } else if (*q) {
        char *h = g_strdup_printf("Results for “%s”", q);
        set_heading(h, sub);
        g_free(h);
        emptybuf = g_strdup_printf("No photos match “%s”. Try fewer words, a month (“June 2025”) or a folder name.", q);
        empty = emptybuf;
    } else {
        char *s2 = app.scanning && !app.scanned ? g_strdup("Looking for photos in ~/Pictures…")
                                                : g_strdup_printf("%s in ~/Pictures", sub);
        set_heading("Library", s2);
        g_free(s2);
        empty = app.scanned ? "No photos yet. Put some in ~/Pictures: Photos finds them by itself, in every folder inside it."
                            : "";
    }
    g_free(sub);
    gtk_label_set_text(GTK_LABEL(app.grid_empty), empty);
    gtk_widget_set_visible(app.grid_empty, app.shown->len == 0 && *empty);
    g_free(emptybuf);

    /* Months (an album keeps its own order, so it isn't split by month). */
    GtkWidget *flow = NULL;
    int month = -1, in_month = 0;
    GtkWidget *count_label = NULL;
    for (guint i = 0; i < app.shown->len; i++) {
        Photo *p = app.shown->pdata[i];
        int m = photo_month(p->taken);
        if (!flow || (app.coll != COLL_ALBUM && m != month)) {
            if (count_label) {
                char *c = count_text((guint)in_month, "photo", "photos");
                gtk_label_set_text(GTK_LABEL(count_label), c);
                g_free(c);
            }
            if (app.coll != COLL_ALBUM) {
                GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
                char *t = month_title(m);
                GtkWidget *ml = label(t, "month", 0);
                g_free(t);
                count_label = label("", "sub", 0);
                gtk_box_pack_start(GTK_BOX(row), ml, FALSE, FALSE, 0);
                gtk_box_pack_start(GTK_BOX(row), count_label, FALSE, FALSE, 0);
                gtk_widget_set_margin_top(row, flow ? 16 : 0);
                gtk_box_pack_start(GTK_BOX(app.grid_box), row, FALSE, FALSE, 0);
            }
            flow = new_flow();
            gtk_box_pack_start(GTK_BOX(app.grid_box), flow, FALSE, FALSE, 0);
            month = m;
            in_month = 0;
        }
        gtk_container_add(GTK_CONTAINER(flow), new_tile(p));
        in_month++;
    }
    if (count_label) {
        char *c = count_text((guint)in_month, "photo", "photos");
        gtk_label_set_text(GTK_LABEL(count_label), c);
        g_free(c);
    }
    gtk_widget_show_all(app.grid_box);
    gtk_adjustment_set_value(adj, keep);
    update_selbar();
    state_changed();
}

static void set_nav(int which)
{
    for (int i = 0; i < 3; i++) {
        GtkStyleContext *sc = gtk_widget_get_style_context(app.nav[i]);
        if (i == which) gtk_style_context_add_class(sc, "on");
        else gtk_style_context_remove_class(sc, "on");
    }
}

static void show_grid(void)
{
    set_nav(0);
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "grid");
    fill_grid();
}

static void show_collection(int coll, const char *album)
{
    app.coll = coll;
    g_free(app.album);
    app.album = g_strdup(album);
    g_hash_table_remove_all(app.selected);
    app.anchor = -1;
    if (coll != COLL_LIBRARY) {
        set_nav(1);
        gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "grid");
        fill_grid();
    } else {
        show_grid();
    }
}

static void on_nav_library(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    show_collection(COLL_LIBRARY, NULL);
}

static void show_albums(void)
{
    set_nav(1);
    fill_albums();
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "albums");
    state_changed();
}

static void on_nav_albums(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    show_albums();
}

static void show_deleted(void)
{
    set_nav(2);
    fill_deleted();
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "deleted");
    state_changed();
}

static void on_nav_deleted(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    show_deleted();
}

static void on_back(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    if (app.coll == COLL_LIBRARY) return;
    app.coll = COLL_LIBRARY;
    show_albums();
}

static void on_search(GtkSearchEntry *e, gpointer d)
{
    (void)e; (void)d;
    if (app.coll != COLL_LIBRARY) {
        app.coll = COLL_LIBRARY;
        set_nav(0);
    }
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "grid");
    fill_grid();
}

/* ---- favourites, albums, deleting (on a set of photos) ---- */

static GPtrArray *targets(void)
{
    GPtrArray *a = g_ptr_array_new();
    const char *view = gtk_stack_get_visible_child_name(GTK_STACK(app.stack));
    if (!g_strcmp0(view, "viewer")) {
        if (app.view_list && app.cur >= 0 && app.cur < (int)app.view_list->len) g_ptr_array_add(a, app.view_list->pdata[app.cur]);
        return a;
    }
    for (guint i = 0; app.shown && i < app.shown->len; i++) {
        Photo *p = app.shown->pdata[i];
        if (g_hash_table_contains(app.selected, p->path)) g_ptr_array_add(a, p);
    }
    if (!a->len) {  /* nothing selected: the photo with the keyboard focus */
        GtkWidget *f = gtk_window_get_focus(GTK_WINDOW(app.win));
        if (f && GTK_IS_FLOW_BOX_CHILD(f)) {
            Photo *p = tile_photo(gtk_bin_get_child(GTK_BIN(f)));
            if (p) g_ptr_array_add(a, p);
        }
    }
    return a;
}

static void toggle_fav(void)
{
    GPtrArray *t = targets();
    gboolean all = t->len > 0;
    for (guint i = 0; i < t->len; i++) all &= store_is_fav(app.store, ((Photo *)t->pdata[i])->rel);
    int outside = 0;
    for (guint i = 0; i < t->len; i++) {
        Photo *p = t->pdata[i];
        if (!p->rel) outside++;
        else store_set_fav(app.store, p->rel, !all);
    }
    if (t->len && outside < (int)t->len) save_store();
    if (outside) toast("Only photos in your Pictures folder can be favourites.", FALSE);
    g_ptr_array_free(t, TRUE);
    if (!g_strcmp0(gtk_stack_get_visible_child_name(GTK_STACK(app.stack)), "viewer")) show_photo();
    else if (app.coll == COLL_FAVOURITES) fill_grid();
    else redraw_tiles();
    state_changed();
}

static void on_sel_fav(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    toggle_fav();
}

static void on_sel_remove(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    GPtrArray *t = targets();
    Album *a = app.coll == COLL_ALBUM ? store_album_find(app.store, app.album) : NULL;
    for (guint i = 0; i < t->len; i++) {
        Photo *p = t->pdata[i];
        if (a) album_remove(a, p->rel);
        else if (app.coll == COLL_FAVOURITES) store_set_fav(app.store, p->rel, FALSE);
    }
    g_ptr_array_free(t, TRUE);
    save_store();
    g_hash_table_remove_all(app.selected);
    fill_grid();
}

static gboolean add_to_album(const char *name, gboolean create, char **why)
{
    Album *a = store_album_find(app.store, name);
    if (!a && create) {
        const char *problem = store_name_problem(app.store, name, NULL);
        if (problem) {
            if (why) *why = g_strdup(problem);
            return FALSE;
        }
        a = store_album_new(app.store, name);
    }
    if (!a) {
        if (why) *why = g_strdup("That album isn't there any more.");
        return FALSE;
    }
    GPtrArray *t = targets();
    int added = 0;
    for (guint i = 0; i < t->len; i++) {
        Photo *p = t->pdata[i];
        if (p->rel && !album_has(a, p->rel)) {
            album_add(a, p->rel);
            added++;
        }
    }
    g_ptr_array_free(t, TRUE);
    save_store();
    char *msg;
    if (added == 1) msg = g_strdup_printf("Added 1 photo to %s.", a->name);
    else if (added) msg = g_strdup_printf("Added %d photos to %s.", added, a->name);
    else msg = g_strdup_printf("Those photos are already in %s.", a->name);
    toast(msg, FALSE);
    g_free(msg);
    if (!g_strcmp0(gtk_stack_get_visible_child_name(GTK_STACK(app.stack)), "grid")) {
        g_hash_table_remove_all(app.selected);
        update_selbar();
    }
    return TRUE;
}

static void on_add_album_item(GtkButton *b, gpointer d)
{
    (void)d;
    gtk_popover_popdown(GTK_POPOVER(gtk_widget_get_ancestor(GTK_WIDGET(b), GTK_TYPE_POPOVER)));
    add_to_album(gtk_button_get_label(b), FALSE, NULL);
}

static void on_new_album_entry(GtkEntry *e, gpointer err_label)
{
    char *why = NULL;
    if (add_to_album(gtk_entry_get_text(e), TRUE, &why)) {
        gtk_popover_popdown(GTK_POPOVER(gtk_widget_get_ancestor(GTK_WIDGET(e), GTK_TYPE_POPOVER)));
    } else {
        gtk_label_set_text(GTK_LABEL(err_label), why);
        gtk_widget_show(err_label);
    }
    g_free(why);
}

static void on_new_album_go(GtkButton *b, gpointer entry)
{
    (void)b;
    g_signal_emit_by_name(entry, "activate");
}

/* Add to Album: the albums, then a field for a new one. */
static void on_add_album(GtkButton *b, gpointer d)
{
    (void)d;
    GtkWidget *pop = gtk_popover_new(GTK_WIDGET(b));
    add_class(pop, "menu-pop");
    GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 4);
    g_object_set(box, "margin", 12, NULL);
    for (guint i = 0; i < app.store->albums->len; i++) {
        Album *a = app.store->albums->pdata[i];
        GtkWidget *item = button(a->name, NULL, G_CALLBACK(on_add_album_item), NULL, "menu-item");
        gtk_widget_set_halign(gtk_bin_get_child(GTK_BIN(item)), GTK_ALIGN_START);
        gtk_box_pack_start(GTK_BOX(box), item, FALSE, FALSE, 0);
    }
    GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    GtkWidget *entry = gtk_entry_new();
    gtk_entry_set_placeholder_text(GTK_ENTRY(entry), "New album name");
    gtk_entry_set_max_length(GTK_ENTRY(entry), ALBUM_NAME_MAX);
    GtkWidget *err = label("", "error", 0);
    gtk_widget_set_no_show_all(err, TRUE);
    g_signal_connect(entry, "activate", G_CALLBACK(on_new_album_entry), err);
    gtk_box_pack_start(GTK_BOX(row), entry, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(row), button("Create", "Make the album and add the photos", G_CALLBACK(on_new_album_go), entry, "primary"),
                       FALSE, FALSE, 0);
    if (app.store->albums->len) gtk_widget_set_margin_top(row, 8);
    gtk_box_pack_start(GTK_BOX(box), row, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(box), err, FALSE, FALSE, 0);
    gtk_container_add(GTK_CONTAINER(pop), box);
    gtk_widget_show_all(box);
    gtk_popover_popup(GTK_POPOVER(pop));
    gtk_widget_grab_focus(entry);
}

static void remove_from_library(const Photo *p)
{
    g_hash_table_remove(app.by_path, p->path);
    g_hash_table_remove(app.selected, p->path);
    g_ptr_array_remove(app.all, (gpointer)p);  /* frees it */
}

static void delete_photos(void)
{
    GPtrArray *t = targets();
    if (!t->len) {
        g_ptr_array_free(t, TRUE);
        return;
    }
    gboolean in_viewer = !g_strcmp0(gtk_stack_get_visible_child_name(GTK_STACK(app.stack)), "viewer");
    int done = 0, failed = 0, outside = 0;
    if (app.undo) g_ptr_array_set_size(app.undo, 0);
    else app.undo = g_ptr_array_new();
    char *first_error = NULL;
    for (guint i = 0; i < t->len; i++) {
        Photo *p = t->pdata[i];
        if (!p->rel) {
            outside++;
            continue;
        }
        GError *err = NULL;
        Gone *g = store_delete(app.store, app.root, p->rel, g_get_real_time() / G_USEC_PER_SEC, &err);
        if (g) {
            g_ptr_array_add(app.undo, g);
            if (app.view_list && in_viewer) g_ptr_array_remove(app.view_list, p);
            if (app.shown && app.view_list != app.shown) g_ptr_array_remove(app.shown, p);
            remove_from_library(p);
            done++;
        } else {
            failed++;
            if (!first_error && err) first_error = g_strdup(err->message);
            g_clear_error(&err);
        }
    }
    g_ptr_array_free(t, TRUE);
    save_store();
    char *msg;
    if (failed)
        msg = g_strdup_printf("%d couldn't be moved to Deleted (%s). Check that the files can be changed, and try again.", failed,
                              first_error ? first_error : "unknown");
    else if (outside)
        msg = g_strdup("Photos only deletes photos in your Pictures folder. Delete this one in Files.");
    else
        msg = g_strdup_printf(done == 1 ? "Moved 1 photo to Deleted." : "Moved %d photos to Deleted.", done);
    toast(msg, done > 0 && !failed);
    g_free(msg);
    g_free(first_error);
    if (in_viewer) {
        if (!app.view_list || !app.view_list->len) {
            leave_viewer();
        } else {
            if (app.cur >= (int)app.view_list->len) app.cur = (int)app.view_list->len - 1;
            show_photo();
        }
    } else {
        fill_grid();
    }
}

static void on_sel_delete(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    delete_photos();
}

static void on_sel_cancel(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    clear_selection();
}

/* ---- the albums page ---- */

static gboolean draw_cover(GtkWidget *w, cairo_t *cr, gpointer d)
{
    (void)d;
    Photo *p = g_object_get_data(G_OBJECT(w), "photo");
    int W = gtk_widget_get_allocated_width(w), H = gtk_widget_get_allocated_height(w);
    rounded(cr, 0, 0, W, H, 16);
    cairo_clip(cr);
    cairo_surface_t *s = p ? thumb_want(p) : NULL;
    if (s) {
        double k = fmax((double)W / cairo_image_surface_get_width(s), (double)H / cairo_image_surface_get_height(s));
        cairo_translate(cr, (W - cairo_image_surface_get_width(s) * k) / 2, (H - cairo_image_surface_get_height(s) * k) / 2);
        cairo_scale(cr, k, k);
        cairo_set_source_surface(cr, s, 0, 0);
        cairo_paint(cr);
    } else {
        GdkRGBA c = colour(w, "nexa_surface_hi", "#222a46"), a = colour(w, "nexa_accent", "#4da3ff");
        gdk_cairo_set_source_rgba(cr, &c);
        cairo_paint(cr);
        hex_path(cr, W / 2.0, H / 2.0, 22);
        cairo_set_source_rgba(cr, a.red, a.green, a.blue, 0.8);
        cairo_set_line_width(cr, 3);
        if (g_object_get_data(G_OBJECT(w), "fav")) cairo_fill(cr);
        else cairo_stroke(cr);
    }
    return TRUE;
}

static void on_album_card(GtkButton *b, gpointer d)
{
    (void)d;
    const char *name = g_object_get_data(G_OBJECT(b), "album");
    if (name) show_collection(COLL_ALBUM, name);
    else show_collection(COLL_FAVOURITES, NULL);
}

static GtkWidget *album_card(const char *name, guint count, Photo *cover, gboolean fav)
{
    GtkWidget *b = gtk_button_new();
    add_class(b, "card");
    GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
    GtkWidget *pic = gtk_drawing_area_new();
    gtk_widget_set_size_request(pic, 200, 150);
    g_object_set_data(G_OBJECT(pic), "photo", cover);
    if (fav) g_object_set_data(G_OBJECT(pic), "fav", GINT_TO_POINTER(1));
    g_signal_connect(pic, "draw", G_CALLBACK(draw_cover), NULL);
    gtk_box_pack_start(GTK_BOX(box), pic, FALSE, FALSE, 0);
    GtkWidget *n = label(name, "card-name", 0);
    gtk_label_set_line_wrap(GTK_LABEL(n), FALSE);
    gtk_label_set_ellipsize(GTK_LABEL(n), PANGO_ELLIPSIZE_END);
    gtk_label_set_max_width_chars(GTK_LABEL(n), 22);
    gtk_box_pack_start(GTK_BOX(box), n, FALSE, FALSE, 0);
    char *c = count_text(count, "photo", "photos");
    gtk_box_pack_start(GTK_BOX(box), label(c, "sub", 0), FALSE, FALSE, 0);
    g_free(c);
    gtk_container_add(GTK_CONTAINER(b), box);
    if (!fav) g_object_set_data_full(G_OBJECT(b), "album", g_strdup(name), g_free);
    g_signal_connect(b, "clicked", G_CALLBACK(on_album_card), NULL);
    return b;
}

static Photo *photo_at(const char *rel)
{
    char *path = g_build_filename(app.root, rel, NULL);
    Photo *p = g_hash_table_lookup(app.by_path, path);
    g_free(path);
    return p;
}

static void fill_albums(void)
{
    gtk_container_foreach(GTK_CONTAINER(app.albums_flow), destroy_child, NULL);
    guint nf = 0;
    Photo *fcover = NULL;
    for (guint i = 0; i < app.all->len; i++) {
        Photo *p = app.all->pdata[i];
        if (store_is_fav(app.store, p->rel)) {
            if (!fcover) fcover = p;
            nf++;
        }
    }
    gtk_container_add(GTK_CONTAINER(app.albums_flow), album_card("Favourites", nf, fcover, TRUE));
    for (guint i = 0; i < app.store->albums->len; i++) {
        Album *a = app.store->albums->pdata[i];
        guint n = 0;
        Photo *cover = NULL;
        for (guint j = 0; j < a->items->len; j++) {
            Photo *p = photo_at(a->items->pdata[j]);
            if (p) {
                if (!cover) cover = p;
                n++;
            }
        }
        gtk_container_add(GTK_CONTAINER(app.albums_flow), album_card(a->name, n, cover, FALSE));
    }
    gtk_widget_show_all(app.albums_flow);
}

/* A small dialog with one field: New Album and Rename. */
static char *ask_name(const char *title, const char *action, const char *initial, const Album *renaming)
{
    GtkWidget *d = gtk_dialog_new_with_buttons(title, GTK_WINDOW(app.win), GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT,
                                               "Cancel", GTK_RESPONSE_CANCEL, action, GTK_RESPONSE_OK, NULL);
    add_class(d, "photos-dialog");
    GtkWidget *ok = gtk_dialog_get_widget_for_response(GTK_DIALOG(d), GTK_RESPONSE_OK);
    add_class(ok, "primary");
    gtk_dialog_set_default_response(GTK_DIALOG(d), GTK_RESPONSE_OK);
    GtkWidget *area = gtk_dialog_get_content_area(GTK_DIALOG(d));
    g_object_set(area, "margin", 16, "spacing", 8, NULL);
    GtkWidget *entry = gtk_entry_new();
    gtk_entry_set_max_length(GTK_ENTRY(entry), ALBUM_NAME_MAX);
    gtk_entry_set_text(GTK_ENTRY(entry), initial ? initial : "");
    gtk_entry_set_activates_default(GTK_ENTRY(entry), TRUE);
    gtk_entry_set_width_chars(GTK_ENTRY(entry), 30);
    GtkWidget *err = label("", "error", 0);
    gtk_box_pack_start(GTK_BOX(area), label("Album name", "sub", 0), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(area), entry, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(area), err, FALSE, FALSE, 0);
    gtk_widget_show_all(d);
    gtk_widget_hide(err);
    char *result = NULL;
    while (!result && gtk_dialog_run(GTK_DIALOG(d)) == GTK_RESPONSE_OK) {
        const char *why = store_name_problem(app.store, gtk_entry_get_text(GTK_ENTRY(entry)), renaming);
        if (why) {
            gtk_label_set_text(GTK_LABEL(err), why);
            gtk_widget_show(err);
        } else {
            result = g_strstrip(g_strdup(gtk_entry_get_text(GTK_ENTRY(entry))));
        }
    }
    gtk_widget_destroy(d);
    return result;
}

static gboolean confirm(const char *title, const char *text, const char *action)
{
    GtkWidget *d = gtk_message_dialog_new(GTK_WINDOW(app.win), GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT,
                                          GTK_MESSAGE_OTHER, GTK_BUTTONS_NONE, "%s", title);
    add_class(d, "photos-dialog");
    gtk_message_dialog_format_secondary_text(GTK_MESSAGE_DIALOG(d), "%s", text);
    gtk_dialog_add_button(GTK_DIALOG(d), "Cancel", GTK_RESPONSE_CANCEL);
    GtkWidget *go = gtk_dialog_add_button(GTK_DIALOG(d), action, GTK_RESPONSE_OK);
    add_class(go, "danger");
    gtk_dialog_set_default_response(GTK_DIALOG(d), GTK_RESPONSE_CANCEL);
    gboolean yes = gtk_dialog_run(GTK_DIALOG(d)) == GTK_RESPONSE_OK;
    gtk_widget_destroy(d);
    return yes;
}

static void new_album_named(const char *name)
{
    if (store_name_problem(app.store, name, NULL)) return;
    store_album_new(app.store, name);
    save_store();
    fill_albums();
    state_changed();
}

static void on_new_album(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    char *name = ask_name("New Album", "Create", "", NULL);
    if (name) new_album_named(name);
    g_free(name);
}

static void on_rename_album(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    Album *a = store_album_find(app.store, app.album);
    if (!a) return;
    char *name = ask_name("Rename Album", "Rename", a->name, a);
    if (name) {
        g_free(a->name);
        a->name = name;
        g_free(app.album);
        app.album = g_strdup(name);
        save_store();
        fill_grid();
    }
}

static void on_delete_album(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    Album *a = store_album_find(app.store, app.album);
    if (!a) return;
    char *title = g_strdup_printf("Delete the album “%s”?", a->name);
    if (confirm(title, "The photos in it stay in your library. Only the album goes.", "Delete Album")) {
        store_album_delete(app.store, a);
        save_store();
        app.coll = COLL_LIBRARY;
        show_albums();
    }
    g_free(title);
}

/* ---- Deleted ---- */

static void after_restore(void);

static void on_restore(GtkButton *b, gpointer d)
{
    (void)b;
    Gone *g = d;
    GError *err = NULL;
    char *rel = store_restore(app.store, g, app.root, &err);
    if (!rel) {
        char *m = g_strdup_printf("Photos couldn't put it back: %s", err ? err->message : "unknown");
        toast(m, FALSE);
        g_free(m);
    }
    g_clear_error(&err);
    g_free(rel);
    save_store();
    after_restore();
}

static void on_forget(GtkButton *b, gpointer d)
{
    (void)b;
    Gone *g = d;
    if (!confirm("Delete this photo for good?", "It can't be put back after this.", "Delete for Good")) return;
    store_forget(app.store, g, NULL);
    save_store();
    fill_deleted();
    state_changed();
}

static void on_restore_all(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    while (app.store->gone->len) {
        char *rel = store_restore(app.store, app.store->gone->pdata[0], app.root, NULL);
        if (!rel) break;
        g_free(rel);
    }
    save_store();
    after_restore();
}

static void empty_deleted(void)
{
    while (app.store->gone->len)
        if (!store_forget(app.store, app.store->gone->pdata[0], NULL)) break;
    save_store();
    fill_deleted();
    state_changed();
}

static void on_empty(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    char *t = g_strdup_printf("Delete %u photos for good?", app.store->gone->len);
    if (confirm(t, "They can't be put back after this.", "Delete for Good")) empty_deleted();
    g_free(t);
}

static char *ago(gint64 when)
{
    gint64 days = (g_get_real_time() / G_USEC_PER_SEC - when) / 86400;
    if (days < 1) return g_strdup("today");
    if (days == 1) return g_strdup("yesterday");
    return g_strdup_printf("%" G_GINT64_FORMAT " days ago", days);
}

static gboolean draw_gone(GtkWidget *w, cairo_t *cr, gpointer d)
{
    (void)d;
    cairo_surface_t *s = g_object_get_data(G_OBJECT(w), "pic");
    int W = gtk_widget_get_allocated_width(w), H = gtk_widget_get_allocated_height(w);
    rounded(cr, 0, 0, W, H, 6);
    cairo_clip(cr);
    if (s) {
        double k = fmax((double)W / cairo_image_surface_get_width(s), (double)H / cairo_image_surface_get_height(s));
        cairo_scale(cr, k, k);
        cairo_set_source_surface(cr, s, 0, 0);
        cairo_paint(cr);
    } else {
        GdkRGBA c = colour(w, "nexa_surface_hi", "#222a46");
        gdk_cairo_set_source_rgba(cr, &c);
        cairo_paint(cr);
    }
    return TRUE;
}

static void fill_deleted(void)
{
    gtk_container_foreach(GTK_CONTAINER(app.del_list), destroy_child, NULL);
    guint n = app.store->gone->len;
    char *c = count_text(n, "photo", "photos");
    char *sub = g_strdup_printf("%s. Deleted photos stay here for %d days, then they're gone for good.", c, KEEP_DELETED_DAYS);
    gtk_label_set_text(GTK_LABEL(app.del_sub), sub);
    g_free(sub);
    g_free(c);
    gtk_widget_set_visible(app.del_empty, n == 0);
    gtk_widget_set_visible(app.del_tools, n > 0);
    for (guint i = n; i-- > 0;) {  /* newest first */
        Gone *g = app.store->gone->pdata[i];
        GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
        add_class(row, "gone");
        GtkWidget *pic = gtk_drawing_area_new();
        gtk_widget_set_size_request(pic, 64, 64);
        Photo *p = NULL;
        char *path = store_gone_path(app.store, g);
        p = photo_read(path, NULL);
        g_free(path);
        if (p) {  /* small and rare: made right here, not cached */
            cairo_surface_t *s = picture_load(p, p->info.orientation, 64, 64, 1, NULL);
            if (s) g_object_set_data_full(G_OBJECT(pic), "pic", s, (GDestroyNotify)cairo_surface_destroy);
            photo_free(p);
        }
        g_signal_connect(pic, "draw", G_CALLBACK(draw_gone), NULL);
        gtk_box_pack_start(GTK_BOX(row), pic, FALSE, FALSE, 0);
        GtkWidget *texts = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
        char *base = g_path_get_basename(g->rel), *dir = g_path_get_dirname(g->rel), *when = ago(g->when);
        char *from = g_strdup_printf("Deleted %s · from %s", when, !strcmp(dir, ".") ? "Pictures" : dir);
        GtkWidget *nl = label(base, "name", 0);
        gtk_label_set_line_wrap(GTK_LABEL(nl), FALSE);
        gtk_label_set_ellipsize(GTK_LABEL(nl), PANGO_ELLIPSIZE_MIDDLE);
        gtk_box_pack_start(GTK_BOX(texts), nl, FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(texts), label(from, "sub", 0), FALSE, FALSE, 0);
        gtk_widget_set_valign(texts, GTK_ALIGN_CENTER);
        g_free(base); g_free(dir); g_free(when); g_free(from);
        gtk_box_pack_start(GTK_BOX(row), texts, TRUE, TRUE, 0);
        GtkWidget *put = button("Put Back", "Back to where it was in Pictures", G_CALLBACK(on_restore), g, NULL);
        GtkWidget *bye = button("Delete for Good", NULL, G_CALLBACK(on_forget), g, "danger-text");
        gtk_widget_set_valign(put, GTK_ALIGN_CENTER);
        gtk_widget_set_valign(bye, GTK_ALIGN_CENTER);
        gtk_box_pack_end(GTK_BOX(row), bye, FALSE, FALSE, 0);
        gtk_box_pack_end(GTK_BOX(row), put, FALSE, FALSE, 0);
        gtk_container_add(GTK_CONTAINER(app.del_list), row);
    }
    gtk_widget_show_all(app.del_list);
}

/* ---- the toast ---- */

static gboolean toast_hide(gpointer d)
{
    (void)d;
    app.toast_id = 0;
    gtk_revealer_set_reveal_child(GTK_REVEALER(app.toast_rev), FALSE);
    if (app.undo) g_ptr_array_set_size(app.undo, 0);
    state_changed();
    return G_SOURCE_REMOVE;
}

static void toast(const char *text, gboolean undo)
{
    gtk_label_set_text(GTK_LABEL(app.toast_label), text);
    gtk_widget_set_visible(app.toast_undo, undo);
    gtk_revealer_set_reveal_child(GTK_REVEALER(app.toast_rev), TRUE);
    if (app.toast_id) g_source_remove(app.toast_id);
    app.toast_id = g_timeout_add(TOAST_MS, toast_hide, NULL);
    state_changed();
}

static void do_undo(void)
{
    if (!app.undo || !app.undo->len) return;
    int back = 0;
    for (guint i = 0; i < app.undo->len; i++) {
        Gone *g = app.undo->pdata[i];
        if (g_ptr_array_find(app.store->gone, g, NULL)) {
            char *rel = store_restore(app.store, g, app.root, NULL);
            if (rel) back++;
            g_free(rel);
        }
    }
    g_ptr_array_set_size(app.undo, 0);
    save_store();
    char *m = g_strdup_printf(back == 1 ? "Put 1 photo back." : "Put %d photos back.", back);
    toast(m, FALSE);
    g_free(m);
    after_restore();
}

static void on_undo(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    do_undo();
}

/* ---- the viewer ---- */

typedef struct {
    Photo *p;
    int o;
    guint gen;
    cairo_surface_t *s;
    char *why;
} LoadJob;

static gboolean load_done(gpointer data)
{
    LoadJob *j = data;
    if (j->gen == app.load_gen) {
        if (app.pic) {
            if (app.old_pic) cairo_surface_destroy(app.old_pic);
            app.old_pic = NULL;
            cairo_surface_destroy(app.pic);
        }
        app.pic = j->s;
        g_free(app.why);
        app.why = j->why;
        gtk_label_set_text(GTK_LABEL(app.view_msg), app.why ? app.why : "");
        gtk_widget_set_visible(app.view_msg_box, app.why != NULL);
        gtk_widget_queue_draw(app.view_area);
        state_changed();
    } else {
        if (j->s) cairo_surface_destroy(j->s);
        g_free(j->why);
    }
    photo_free(j->p);
    g_free(j);
    return G_SOURCE_REMOVE;
}

static gpointer load_work(gpointer data)
{
    LoadJob *j = data;
    j->s = picture_load(j->p, j->o, 0, 0, 0, &j->why);
    g_idle_add(load_done, j);
    return NULL;
}

static Photo *current(void)
{
    if (!app.view_list || app.cur < 0 || app.cur >= (int)app.view_list->len) return NULL;
    return app.view_list->pdata[app.cur];
}

static gboolean fade_tick(GtkWidget *w, GdkFrameClock *clock, gpointer d)
{
    (void)clock; (void)d;
    gtk_widget_queue_draw(w);
    if (g_get_monotonic_time() - app.fade_at > FADE_MS * 1000) {
        app.fade_id = 0;
        if (app.old_pic) cairo_surface_destroy(app.old_pic);
        app.old_pic = NULL;
        return G_SOURCE_REMOVE;
    }
    return G_SOURCE_CONTINUE;
}

static void set_info_row(int row, const char *name, const char *value)
{
    GtkWidget *k = gtk_grid_get_child_at(GTK_GRID(app.info_grid), 0, row);
    GtkWidget *v = gtk_grid_get_child_at(GTK_GRID(app.info_grid), 1, row);
    if (!k) {
        k = label(name, "info-key", 0);
        v = label("", "info-value", 0);
        gtk_label_set_max_width_chars(GTK_LABEL(v), 24);
        gtk_widget_set_size_request(v, 200, -1);
        gtk_label_set_selectable(GTK_LABEL(v), TRUE);
        gtk_widget_set_hexpand(v, TRUE);
        gtk_widget_set_valign(k, GTK_ALIGN_START);
        gtk_grid_attach(GTK_GRID(app.info_grid), k, 0, row, 1, 1);
        gtk_grid_attach(GTK_GRID(app.info_grid), v, 1, row, 1, 1);
    }
    gtk_label_set_text(GTK_LABEL(v), value && *value ? value : "—");
    gtk_widget_show(k);
    gtk_widget_show(v);
}

static void fill_info(const Photo *p)
{
    char *when = photo_when(p);
    char *taken = p->exif_date ? when : g_strdup_printf("%s (the file's date: no date inside it)", when);
    set_info_row(0, "Taken", taken);
    if (taken != when) g_free(taken);
    g_free(when);
    set_info_row(1, "File", p->name);
    char *dir = p->rel ? g_path_get_dirname(p->rel) : g_path_get_dirname(p->path);
    char *folder = p->rel ? (!strcmp(dir, ".") ? g_strdup("Pictures") : g_strdup_printf("Pictures/%s", dir)) : g_strdup(dir);
    set_info_row(2, "Folder", folder);
    g_free(dir);
    g_free(folder);
    int sw, sh;
    orient_size(p->info.orientation, p->info.width, p->info.height, &sw, &sh);
    char *sz = g_format_size((guint64)p->size);
    char *size = sw ? g_strdup_printf("%d × %d (%.1f megapixels) · %s", sw, sh, sw * (double)sh / 1e6, sz) : g_strdup(sz);
    set_info_row(3, "Size", size);
    g_free(sz);
    g_free(size);
    char *cam = g_strstrip(g_strdup_printf("%s %s", p->info.make, p->info.model));
    set_info_row(4, "Camera", cam);
    g_free(cam);
    GString *set = g_string_new(NULL);
    const char *parts[] = {p->info.fnumber, p->info.exposure, NULL, p->info.focal};
    char iso[24] = "";
    if (p->info.iso) g_snprintf(iso, sizeof iso, "ISO %d", p->info.iso);
    parts[2] = iso;
    for (int i = 0; i < 4; i++)
        if (parts[i] && *parts[i]) g_string_append_printf(set, "%s%s", set->len ? " · " : "", parts[i]);
    set_info_row(5, "Settings", set->str);
    g_string_free(set, TRUE);
    set_info_row(6, "Lens", p->info.lens);
    char *where = p->info.has_gps ? g_strdup_printf("%.4f° %s, %.4f° %s", fabs(p->info.lat), p->info.lat >= 0 ? "N" : "S",
                                                     fabs(p->info.lon), p->info.lon >= 0 ? "E" : "W")
                                  : NULL;
    set_info_row(7, "Place", where);
    g_free(where);
    char *albums = p->rel ? store_albums_of(app.store, p->rel) : NULL;
    set_info_row(8, "Albums", albums);
    g_free(albums);
}

static void update_fav_button(const Photo *p)
{
    gboolean fav = p && store_is_fav(app.store, p->rel);
    gtk_button_set_label(GTK_BUTTON(app.fav_btn), fav ? "⬢ Favourite" : "⬡ Favourite");
    GtkStyleContext *sc = gtk_widget_get_style_context(app.fav_btn);
    if (fav) gtk_style_context_add_class(sc, "on");
    else gtk_style_context_remove_class(sc, "on");
    gtk_widget_set_sensitive(app.fav_btn, p && p->rel);
}

static void show_photo(void)
{
    Photo *p = current();
    if (!p) return;
    g_free(app.cur_path);
    app.cur_path = g_strdup(p->path);
    int o = orientation_of(p);
    char *key = thumb_key(p, o);
    gboolean same = app.pic_key && !strcmp(app.pic_key, key);
    if (!same) {
        /* The old picture fades out under the new one. */
        if (app.old_pic) cairo_surface_destroy(app.old_pic);
        app.old_pic = app.pic && app.animations && !reduced_motion() ? app.pic : NULL;
        if (app.pic && !app.old_pic) cairo_surface_destroy(app.pic);
        app.pic = NULL;
        g_free(app.pic_key);
        app.pic_key = key;
        key = NULL;
        app.fit = TRUE;
        app.pan_x = app.pan_y = 0;
        if (app.old_pic) {
            app.fade_at = g_get_monotonic_time();
            if (!app.fade_id) app.fade_id = gtk_widget_add_tick_callback(app.view_area, fade_tick, NULL, NULL);
        }
        g_free(app.why);
        app.why = NULL;
        gtk_widget_hide(app.view_msg_box);
        LoadJob *j = g_new0(LoadJob, 1);
        j->p = photo_copy(p);
        j->o = o;
        j->gen = ++app.load_gen;
        g_thread_unref(g_thread_new("photos-load", load_work, j));
    }
    g_free(key);
    gtk_label_set_text(GTK_LABEL(app.view_name), p->name);
    char *when = photo_when(p);
    gtk_label_set_text(GTK_LABEL(app.view_when), when);
    g_free(when);
    char *count = g_strdup_printf("%d of %u", app.cur + 1, app.view_list->len);
    gtk_label_set_text(GTK_LABEL(app.view_count), count);
    g_free(count);
    gtk_window_set_title(GTK_WINDOW(app.win), p->name);
    update_fav_button(p);
    fill_info(p);
    gtk_widget_queue_draw(app.view_area);
    state_changed();
}

static void open_viewer(GPtrArray *list, int index)
{
    if (!list || index < 0 || index >= (int)list->len) return;
    if (app.view_list && app.view_list != app.shown) g_ptr_array_free(app.view_list, TRUE);
    app.view_list = g_ptr_array_copy(list, NULL, NULL);
    app.cur = index;
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "viewer");
    show_photo();
    gtk_widget_grab_focus(app.view_area);
}

static void stop_slideshow(void)
{
    if (app.slide_id) g_source_remove(app.slide_id);
    app.slide_id = 0;
    if (app.slideshow && app.fullscreen) gtk_window_unfullscreen(GTK_WINDOW(app.win));
    app.slideshow = app.paused = FALSE;
    gtk_widget_set_visible(app.view_top, TRUE);
}

static void leave_viewer(void)
{
    stop_slideshow();
    app.load_gen++;
    if (app.pic) cairo_surface_destroy(app.pic);
    if (app.old_pic) cairo_surface_destroy(app.old_pic);
    app.pic = app.old_pic = NULL;
    g_clear_pointer(&app.pic_key, g_free);
    gtk_window_set_title(GTK_WINDOW(app.win), "Photos");
    if (app.extra) {  /* opened from outside: nothing to go back to but the library */
        app.coll = COLL_LIBRARY;
    }
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "grid");
    fill_grid();
    /* Back where you were: the photo you looked at last gets the focus. */
    GtkWidget *tile = app.cur_path ? g_hash_table_lookup(app.tile_of, app.cur_path) : NULL;
    if (tile) gtk_widget_grab_focus(gtk_widget_get_parent(tile));
}

static void step(int by)
{
    if (!app.view_list || !app.view_list->len) return;
    int n = (int)app.view_list->len, to = app.cur + by;
    if (app.slideshow) to = ((to % n) + n) % n;  /* a slideshow goes round */
    if (to < 0 || to >= n || to == app.cur) return;
    app.cur = to;
    show_photo();
}

static gboolean slide_next(gpointer d)
{
    (void)d;
    if (!app.paused) step(1);
    return G_SOURCE_CONTINUE;
}

static void start_slideshow(void)
{
    if (!app.view_list || !app.view_list->len) return;
    app.slideshow = TRUE;
    app.paused = FALSE;
    gtk_widget_set_visible(app.view_top, FALSE);
    gtk_widget_hide(app.info_rev);
    if (!app.fullscreen) gtk_window_fullscreen(GTK_WINDOW(app.win));
    if (app.slide_id) g_source_remove(app.slide_id);
    app.slide_id = g_timeout_add(SLIDE_MS, slide_next, NULL);
    state_changed();
}

static void on_slideshow(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    if (!g_strcmp0(gtk_stack_get_visible_child_name(GTK_STACK(app.stack)), "viewer")) {
        start_slideshow();
    } else if (app.shown && app.shown->len) {
        open_viewer(app.shown, 0);
        start_slideshow();
    }
}

static void turn(int q)
{
    Photo *p = current();
    if (!p) return;
    if (p == app.extra) app.extra_turn = (app.extra_turn + q + 4) % 4;
    else {
        store_set_turn(app.store, p->rel, store_turn(app.store, p->rel) + q);
        save_store();
    }
    show_photo();
}

static void on_turn_left(GtkButton *b, gpointer d) { (void)b; (void)d; turn(-1); }
static void on_turn_right(GtkButton *b, gpointer d) { (void)b; (void)d; turn(1); }
static void on_fav_btn(GtkButton *b, gpointer d) { (void)b; (void)d; toggle_fav(); }
static void on_view_delete(GtkButton *b, gpointer d) { (void)b; (void)d; delete_photos(); }
static void on_view_back(GtkButton *b, gpointer d) { (void)b; (void)d; leave_viewer(); }

static void toggle_info(void)
{
    gtk_widget_set_visible(app.info_rev, !gtk_widget_get_visible(app.info_rev));
    state_changed();
}

static void on_info_btn(GtkButton *b, gpointer d) { (void)b; (void)d; toggle_info(); }

/* Scale the picture is drawn at, and where. */
static double fit_scale(int aw, int ah, int w, int h) { return fmin(1, fmin((double)aw / w, (double)ah / h)); }

static double view_scale(void)
{
    if (!app.pic) return 1;
    int aw = gtk_widget_get_allocated_width(app.view_area), ah = gtk_widget_get_allocated_height(app.view_area);
    int w = cairo_image_surface_get_width(app.pic), h = cairo_image_surface_get_height(app.pic);
    return app.fit ? fit_scale(aw, ah, w, h) : app.zoom;
}

/* The picture's full size against the one loaded (big ones are loaded smaller). */
static double pic_ratio(void)
{
    Photo *p = current();
    if (!p || !app.pic) return 1;
    int sw, sh;
    orient_size(orientation_of(p), p->info.width, p->info.height, &sw, &sh);
    int w = cairo_image_surface_get_width(app.pic);
    return sw > 0 && w > 0 ? (double)sw / w : 1;
}

static void clamp_pan(void)
{
    if (!app.pic) return;
    int aw = gtk_widget_get_allocated_width(app.view_area), ah = gtk_widget_get_allocated_height(app.view_area);
    double s = view_scale(), w = cairo_image_surface_get_width(app.pic) * s, h = cairo_image_surface_get_height(app.pic) * s;
    double mx = fmax(0, (w - aw) / 2), my = fmax(0, (h - ah) / 2);
    app.pan_x = CLAMP(app.pan_x, -mx, mx);
    app.pan_y = CLAMP(app.pan_y, -my, my);
}

/* Zoom to scale s, keeping the point (x, y) of the view still. */
static void zoom_to(double s, double x, double y)
{
    if (!app.pic) return;
    int aw = gtk_widget_get_allocated_width(app.view_area), ah = gtk_widget_get_allocated_height(app.view_area);
    double old = view_scale();
    double lo = fmin(fit_scale(aw, ah, cairo_image_surface_get_width(app.pic), cairo_image_surface_get_height(app.pic)), 1);
    double hi = 8 / pic_ratio();
    s = CLAMP(s, lo, fmax(lo, hi));
    double cx = aw / 2.0 + app.pan_x, cy = ah / 2.0 + app.pan_y;  /* the picture's centre on screen */
    app.pan_x = (cx - x) * s / old + x - aw / 2.0;
    app.pan_y = (cy - y) * s / old + y - ah / 2.0;
    app.zoom = s;
    app.fit = fabs(s - lo) < 1e-6;
    if (app.fit) app.pan_x = app.pan_y = 0;
    clamp_pan();
    gtk_widget_queue_draw(app.view_area);
    state_changed();
}

static void zoom_by(double k)
{
    int aw = gtk_widget_get_allocated_width(app.view_area), ah = gtk_widget_get_allocated_height(app.view_area);
    zoom_to(view_scale() * k, aw / 2.0, ah / 2.0);
}

static void zoom_actual(void)
{
    int aw = gtk_widget_get_allocated_width(app.view_area), ah = gtk_widget_get_allocated_height(app.view_area);
    zoom_to(1 / pic_ratio(), aw / 2.0, ah / 2.0);
}

static void paint_pic(cairo_t *cr, cairo_surface_t *s, int aw, int ah, double scale, double px, double py, double alpha)
{
    int w = cairo_image_surface_get_width(s), h = cairo_image_surface_get_height(s);
    cairo_save(cr);
    cairo_translate(cr, round(aw / 2.0 + px - w * scale / 2), round(ah / 2.0 + py - h * scale / 2));
    cairo_scale(cr, scale, scale);
    cairo_set_source_surface(cr, s, 0, 0);
    cairo_pattern_set_filter(cairo_get_source(cr), scale < 1 ? CAIRO_FILTER_GOOD : scale > 2 ? CAIRO_FILTER_NEAREST : CAIRO_FILTER_BILINEAR);
    cairo_paint_with_alpha(cr, alpha);
    cairo_restore(cr);
}

static gboolean draw_view(GtkWidget *w, cairo_t *cr, gpointer d)
{
    (void)d;
    int aw = gtk_widget_get_allocated_width(w), ah = gtk_widget_get_allocated_height(w);
    GdkRGBA bg = colour(w, "nexa_bg_deep", "#080c1e");
    gdk_cairo_set_source_rgba(cr, &bg);
    cairo_paint(cr);
    double k = app.old_pic ? fmin(1, (g_get_monotonic_time() - app.fade_at) / (FADE_MS * 1000.0)) : 1;
    k = 1 - pow(1 - k, 3);  /* ease out */
    if (app.old_pic && k < 1) {
        int ow = cairo_image_surface_get_width(app.old_pic), oh = cairo_image_surface_get_height(app.old_pic);
        paint_pic(cr, app.old_pic, aw, ah, fit_scale(aw, ah, ow, oh), 0, 0, 1 - k);
    }
    cairo_surface_t *s = app.pic;
    double scale, px = 0, py = 0;
    if (s) {
        scale = view_scale();
        px = app.pan_x;
        py = app.pan_y;
    } else {  /* still loading: the thumbnail, big, until the picture is here */
        Photo *p = current();
        s = p && !app.why ? thumb_want(p) : NULL;
        if (!s) return TRUE;
        int sw, sh;
        orient_size(orientation_of(p), p->info.width, p->info.height, &sw, &sh);
        int tw = cairo_image_surface_get_width(s);
        if (sw <= 0 || sh <= 0) return TRUE;
        double fs = fit_scale(aw, ah, sw, sh);
        /* The square thumbnail is the middle of the picture: draw it where that is. */
        double side = fmin(sw, sh) * fs;
        cairo_save(cr);
        cairo_rectangle(cr, (aw - side) / 2, (ah - side) / 2, side, side);
        cairo_clip(cr);
        paint_pic(cr, s, aw, ah, side / tw, 0, 0, app.old_pic ? k : 1);
        cairo_restore(cr);
        return TRUE;
    }
    paint_pic(cr, s, aw, ah, scale, px, py, app.old_pic ? k : 1);
    return TRUE;
}

static gboolean on_view_press(GtkWidget *w, GdkEventButton *e, gpointer d)
{
    (void)w; (void)d;
    gtk_widget_grab_focus(app.view_area);
    if (e->button != 1) return FALSE;
    if (e->type == GDK_2BUTTON_PRESS) {
        if (app.fit) zoom_to(1 / pic_ratio(), e->x, e->y);
        else zoom_to(0, e->x, e->y);
        return TRUE;
    }
    if (e->type == GDK_BUTTON_PRESS) {
        app.dragging = TRUE;
        app.drag_x = e->x;
        app.drag_y = e->y;
    }
    return TRUE;
}

static gboolean on_view_release(GtkWidget *w, GdkEventButton *e, gpointer d)
{
    (void)w; (void)e; (void)d;
    app.dragging = FALSE;
    return TRUE;
}

static gboolean on_view_motion(GtkWidget *w, GdkEventMotion *e, gpointer d)
{
    (void)d;
    if (!app.dragging || app.fit) return FALSE;
    app.pan_x += e->x - app.drag_x;
    app.pan_y += e->y - app.drag_y;
    app.drag_x = e->x;
    app.drag_y = e->y;
    clamp_pan();
    gtk_widget_queue_draw(w);
    return TRUE;
}

static gboolean on_view_scroll(GtkWidget *w, GdkEventScroll *e, gpointer d)
{
    (void)w; (void)d;
    double dy = 0;
    if (e->direction == GDK_SCROLL_UP) dy = -1;
    else if (e->direction == GDK_SCROLL_DOWN) dy = 1;
    else if (e->direction == GDK_SCROLL_SMOOTH) gdk_event_get_scroll_deltas((GdkEvent *)e, NULL, &dy);
    if (dy == 0) return FALSE;
    zoom_to(view_scale() * pow(1.15, -dy), e->x, e->y);
    return TRUE;
}

/* ---- keys ---- */

static gboolean on_key(GtkWidget *w, GdkEventKey *e, gpointer d)
{
    (void)w; (void)d;
    guint k = e->keyval;
    gboolean ctrl = (e->state & GDK_CONTROL_MASK) != 0, shift = (e->state & GDK_SHIFT_MASK) != 0;
    const char *view = gtk_stack_get_visible_child_name(GTK_STACK(app.stack));
    gboolean typing = gtk_widget_has_focus(app.search) || GTK_IS_ENTRY(gtk_window_get_focus(GTK_WINDOW(app.win)));
    if (ctrl && (k == GDK_KEY_q || k == GDK_KEY_w)) {
        gtk_window_close(GTK_WINDOW(app.win));
        return TRUE;
    }
    if (k == GDK_KEY_F11) {
        if (app.fullscreen) gtk_window_unfullscreen(GTK_WINDOW(app.win));
        else gtk_window_fullscreen(GTK_WINDOW(app.win));
        return TRUE;
    }
    if (ctrl && k == GDK_KEY_f) {
        if (!g_strcmp0(view, "viewer")) leave_viewer();
        if (app.coll != COLL_LIBRARY) show_collection(COLL_LIBRARY, NULL);
        gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "grid");
        gtk_widget_grab_focus(app.search);
        return TRUE;
    }
    if (!g_strcmp0(view, "viewer")) {
        if (app.slideshow) {
            if (k == GDK_KEY_space) {
                app.paused = !app.paused;
                state_changed();
                return TRUE;
            }
            if (k == GDK_KEY_Escape || k == GDK_KEY_F5) {
                stop_slideshow();
                state_changed();
                return TRUE;
            }
        }
        switch (k) {
        case GDK_KEY_Escape:
            if (app.fullscreen) gtk_window_unfullscreen(GTK_WINDOW(app.win));
            else leave_viewer();
            return TRUE;
        case GDK_KEY_BackSpace: leave_viewer(); return TRUE;
        case GDK_KEY_Left: case GDK_KEY_Page_Up: step(-1); return TRUE;
        case GDK_KEY_Right: case GDK_KEY_Page_Down: case GDK_KEY_space: step(1); return TRUE;
        case GDK_KEY_Home: step(-app.cur); return TRUE;
        case GDK_KEY_End: if (app.view_list) step((int)app.view_list->len - 1 - app.cur); return TRUE;
        case GDK_KEY_plus: case GDK_KEY_equal: case GDK_KEY_KP_Add: zoom_by(1.25); return TRUE;
        case GDK_KEY_minus: case GDK_KEY_KP_Subtract: zoom_by(1 / 1.25); return TRUE;
        case GDK_KEY_0: case GDK_KEY_KP_0: zoom_to(0, 0, 0); return TRUE;
        case GDK_KEY_1: case GDK_KEY_KP_1: zoom_actual(); return TRUE;
        case GDK_KEY_r: case GDK_KEY_R: turn(shift ? -1 : 1); return TRUE;
        case GDK_KEY_f: case GDK_KEY_F: toggle_fav(); return TRUE;
        case GDK_KEY_i: case GDK_KEY_I: toggle_info(); return TRUE;
        case GDK_KEY_Delete: delete_photos(); return TRUE;
        case GDK_KEY_F5: start_slideshow(); return TRUE;
        default: return FALSE;
        }
    }
    if (typing) {
        if (k == GDK_KEY_Escape && *query()) {
            gtk_entry_set_text(GTK_ENTRY(app.search), "");
            return TRUE;
        }
        if (k == GDK_KEY_Down && gtk_widget_has_focus(app.search) && app.shown && app.shown->len) {
            GtkWidget *t = g_hash_table_lookup(app.tile_of, ((Photo *)app.shown->pdata[0])->path);
            if (t) gtk_widget_grab_focus(gtk_widget_get_parent(t));
            return TRUE;
        }
        return FALSE;
    }
    if (ctrl && k == GDK_KEY_z) {
        do_undo();
        return TRUE;
    }
    if (!g_strcmp0(view, "grid")) {
        if (ctrl && k == GDK_KEY_a) {
            for (guint i = 0; i < app.shown->len; i++) select_photo(app.shown->pdata[i], TRUE);
            update_selbar();
            return TRUE;
        }
        if (k == GDK_KEY_Escape) {
            if (g_hash_table_size(app.selected)) clear_selection();
            else if (app.coll != COLL_LIBRARY) on_back(NULL, NULL);
            else if (*query()) gtk_entry_set_text(GTK_ENTRY(app.search), "");
            return TRUE;
        }
        if (k == GDK_KEY_Delete) {
            delete_photos();
            return TRUE;
        }
        if (k == GDK_KEY_f || k == GDK_KEY_F) {
            toggle_fav();
            return TRUE;
        }
        if (k == GDK_KEY_F5) {
            on_slideshow(NULL, NULL);
            return TRUE;
        }
        if (k == GDK_KEY_space) {  /* Space on a photo selects it */
            GtkWidget *f = gtk_window_get_focus(GTK_WINDOW(app.win));
            if (f && GTK_IS_FLOW_BOX_CHILD(f)) {
                tile_clicked(tile_photo(gtk_bin_get_child(GTK_BIN(f))), TRUE, FALSE);
                return TRUE;
            }
        }
        /* Typing a letter starts a search. */
        if (!ctrl && e->string && g_unichar_isprint(gdk_keyval_to_unicode(k)) && k != GDK_KEY_space && app.coll == COLL_LIBRARY) {
            gtk_widget_grab_focus(app.search);
            gtk_editable_set_position(GTK_EDITABLE(app.search), -1);
            return gtk_widget_event(app.search, (GdkEvent *)e);
        }
    }
    if ((!g_strcmp0(view, "albums") || !g_strcmp0(view, "deleted")) && k == GDK_KEY_Escape) {
        show_collection(COLL_LIBRARY, NULL);
        return TRUE;
    }
    return FALSE;
}

static gboolean on_window_state(GtkWidget *w, GdkEventWindowState *e, gpointer d)
{
    (void)w; (void)d;
    app.fullscreen = (e->new_window_state & GDK_WINDOW_STATE_FULLSCREEN) != 0;
    GtkWidget *bar = gtk_window_get_titlebar(GTK_WINDOW(app.win));
    if (bar) gtk_widget_set_visible(bar, !app.fullscreen);
    if (!app.fullscreen && app.slideshow) stop_slideshow();
    state_changed();
    return FALSE;
}

/* ---- the library: scanning and following ~/Pictures ---- */

static void watch_folders(void);

typedef struct {
    GPtrArray *photos;
} ScanResult;

static gboolean scan_done(gpointer data)
{
    GPtrArray *photos = data;
    app.scanning = FALSE;
    if (g_atomic_int_get(&app.cancel)) {
        g_ptr_array_free(photos, TRUE);
        return G_SOURCE_REMOVE;
    }
    char *viewing = g_strdup(app.cur_path);
    gboolean in_viewer = !g_strcmp0(gtk_stack_get_visible_child_name(GTK_STACK(app.stack)), "viewer");
    gboolean viewing_extra = in_viewer && app.extra && current() == app.extra;
    /* Swap in the new list; everything that pointed into the old one is
     * made again from paths. */
    GPtrArray *old = app.all;
    app.all = photos;
    g_hash_table_remove_all(app.by_path);
    for (guint i = 0; i < app.all->len; i++) {
        Photo *p = app.all->pdata[i];
        g_hash_table_replace(app.by_path, p->path, p);
    }
    app.scanned = TRUE;
    if (app.view_list && app.view_list != app.shown) {
        g_ptr_array_free(app.view_list, TRUE);
        app.view_list = NULL;
    }
    if (app.shown) g_ptr_array_set_size(app.shown, 0);
    const char *page = gtk_stack_get_visible_child_name(GTK_STACK(app.stack));
    if (!g_strcmp0(page, "albums")) fill_albums();
    else if (!g_strcmp0(page, "deleted")) fill_deleted();
    if (in_viewer && !viewing_extra) {
        collect();
        app.view_list = g_ptr_array_copy(app.shown, NULL, NULL);
        int at = -1;
        for (guint i = 0; viewing && i < app.view_list->len; i++)
            if (!strcmp(((Photo *)app.view_list->pdata[i])->path, viewing)) at = (int)i;
        if (at >= 0) {
            app.cur = at;
            show_photo();
        } else if (app.view_list->len) {
            app.cur = MIN(app.cur, (int)app.view_list->len - 1);
            show_photo();
        } else {
            leave_viewer();
        }
    } else if (viewing_extra) {
        /* (viewing a file from outside: its list is just itself) */
        app.view_list = g_ptr_array_new();
        g_ptr_array_add(app.view_list, app.extra);
        app.cur = 0;
    } else {
        fill_grid();
    }
    if (old) g_ptr_array_free(old, TRUE);
    g_free(viewing);
    watch_folders();
    state_changed();
    if (app.rescan_again) {
        app.rescan_again = FALSE;
        rescan();
    }
    return G_SOURCE_REMOVE;
}

static gpointer scan_work(gpointer d)
{
    (void)d;
    char *index = g_build_filename(app.cache, "index", NULL);
    GPtrArray *photos = library_scan(app.root, index, &app.cancel);
    g_free(index);
    g_idle_add(scan_done, photos);
    return NULL;
}

static void rescan(void)
{
    if (app.scanning) {
        app.rescan_again = TRUE;
        return;
    }
    app.scanning = TRUE;
    g_thread_unref(g_thread_new("photos-scan", scan_work, NULL));
}

static gboolean rescan_soon_cb(gpointer d)
{
    (void)d;
    app.rescan_id = 0;
    rescan();
    return G_SOURCE_REMOVE;
}

static void on_folder_changed(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent ev, gpointer d)
{
    (void)m; (void)f; (void)o; (void)d;
    if (ev == G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT || ev == G_FILE_MONITOR_EVENT_DELETED ||
        ev == G_FILE_MONITOR_EVENT_CREATED || ev == G_FILE_MONITOR_EVENT_MOVED_IN || ev == G_FILE_MONITOR_EVENT_MOVED_OUT ||
        ev == G_FILE_MONITOR_EVENT_RENAMED) {
        if (app.rescan_id) g_source_remove(app.rescan_id);
        app.rescan_id = g_timeout_add(1200, rescan_soon_cb, NULL);  /* a copy of many photos: once at the end */
    }
}

static void watch_dir(const char *dir, int depth)
{
    if (app.monitors->len >= MAX_MONITORS || depth > LIB_MAX_DEPTH) return;
    GFile *f = g_file_new_for_path(dir);
    GFileMonitor *m = g_file_monitor_directory(f, G_FILE_MONITOR_WATCH_MOVES, NULL, NULL);
    g_object_unref(f);
    if (!m) return;
    g_signal_connect(m, "changed", G_CALLBACK(on_folder_changed), NULL);
    g_ptr_array_add(app.monitors, m);
    GDir *d = g_dir_open(dir, 0, NULL);
    if (!d) return;
    const char *name;
    while ((name = g_dir_read_name(d))) {
        if (name[0] == '.') continue;
        char *sub = g_build_filename(dir, name, NULL);
        struct stat st;
        if (lstat(sub, &st) == 0 && S_ISDIR(st.st_mode)) watch_dir(sub, depth + 1);
        g_free(sub);
    }
    g_dir_close(d);
}

static void watch_folders(void)
{
    g_ptr_array_set_size(app.monitors, 0);
    watch_dir(app.root, 0);
}

static void after_restore(void)
{
    fill_deleted();
    rescan();  /* the photos are back in Pictures: the library finds them */
}

/* ---- for tests: state out, commands in (PHOTOS_TEST) ---- */

static void json_str(GString *s, const char *v)
{
    g_string_append_c(s, '"');
    for (const char *c = v ? v : ""; *c; c++) {
        if (*c == '"' || *c == '\\') g_string_append_printf(s, "\\%c", *c);
        else if ((unsigned char)*c < 32) g_string_append_printf(s, "\\u%04x", *c);
        else g_string_append_c(s, *c);
    }
    g_string_append_c(s, '"');
}

static gboolean write_state(gpointer d)
{
    (void)d;
    app.state_id = 0;
    GString *s = g_string_new("{");
    g_string_append(s, "\"page\": ");
    json_str(s, gtk_stack_get_visible_child_name(GTK_STACK(app.stack)));
    g_string_append_printf(s, ", \"scanned\": %d, \"library\": %u, \"coll\": %d, \"shown\": %u, \"selected\": %u",
                           app.scanned && !app.scanning, app.all->len, app.coll, app.shown ? app.shown->len : 0,
                           g_hash_table_size(app.selected));
    g_string_append(s, ", \"heading\": ");
    json_str(s, gtk_label_get_text(GTK_LABEL(app.grid_head)));
    g_string_append(s, ", \"sub\": ");
    json_str(s, gtk_label_get_text(GTK_LABEL(app.grid_sub)));
    g_string_append(s, ", \"empty\": ");
    json_str(s, gtk_widget_get_visible(app.grid_empty) ? gtk_label_get_text(GTK_LABEL(app.grid_empty)) : "");
    g_string_append_printf(s, ", \"thumbs\": %u, \"wanted\": %u", g_hash_table_size(app.thumbs), g_hash_table_size(app.wanted));
    g_string_append(s, ", \"months\": [");
    int last = -1, first = 1;
    for (guint i = 0; app.shown && app.coll != COLL_ALBUM && i < app.shown->len; i++) {
        int m = photo_month(((Photo *)app.shown->pdata[i])->taken);
        if (m != last) {
            char *t = month_title(m);
            if (!first) g_string_append(s, ", ");
            json_str(s, t);
            g_free(t);
            first = 0;
            last = m;
        }
    }
    g_string_append(s, "], \"names\": [");
    for (guint i = 0; app.shown && i < app.shown->len && i < 200; i++) {
        if (i) g_string_append(s, ", ");
        json_str(s, ((Photo *)app.shown->pdata[i])->name);
    }
    g_string_append(s, "], \"favourites\": [");
    first = 1;
    for (guint i = 0; i < app.all->len; i++) {
        Photo *p = app.all->pdata[i];
        if (store_is_fav(app.store, p->rel)) {
            if (!first) g_string_append(s, ", ");
            json_str(s, p->name);
            first = 0;
        }
    }
    g_string_append(s, "], \"albums\": [");
    for (guint i = 0; i < app.store->albums->len; i++) {
        Album *a = app.store->albums->pdata[i];
        if (i) g_string_append(s, ", ");
        g_string_append(s, "{\"name\": ");
        json_str(s, a->name);
        g_string_append_printf(s, ", \"count\": %u}", a->items->len);
    }
    g_string_append_printf(s, "], \"deleted\": %u", app.store->gone->len);
    g_string_append(s, ", \"toast\": ");
    json_str(s, gtk_revealer_get_reveal_child(GTK_REVEALER(app.toast_rev)) ? gtk_label_get_text(GTK_LABEL(app.toast_label)) : "");
    g_string_append_printf(s, ", \"undo\": %d, \"fullscreen\": %d", gtk_widget_get_visible(app.toast_undo) && app.undo && app.undo->len,
                           app.fullscreen);
    Photo *p = current();
    if (!g_strcmp0(gtk_stack_get_visible_child_name(GTK_STACK(app.stack)), "viewer") && p) {
        g_string_append(s, ", \"viewer\": {\"name\": ");
        json_str(s, p->name);
        g_string_append_printf(s, ", \"index\": %d, \"count\": %u, \"loaded\": %d, \"pic\": [%d, %d], \"fit\": %d, \"scale\": %.3f, "
                                  "\"ratio\": %.3f, \"orientation\": %d, \"fav\": %d, \"info\": %d, \"slideshow\": %d, \"paused\": %d, \"why\": ",
                               app.cur, app.view_list->len, app.pic != NULL, app.pic ? cairo_image_surface_get_width(app.pic) : 0,
                               app.pic ? cairo_image_surface_get_height(app.pic) : 0, app.fit, view_scale(), pic_ratio(),
                               orientation_of(p), store_is_fav(app.store, p->rel),
                               gtk_widget_get_visible(app.info_rev), app.slideshow, app.paused);
        json_str(s, app.why);
        g_string_append_printf(s, ", \"area\": [%d, %d], \"msg_at\": [%d, %d]", gtk_widget_get_allocated_width(app.view_area),
                               gtk_widget_get_allocated_height(app.view_area),
                               gtk_widget_get_visible(app.view_msg_box) ? gtk_widget_get_allocated_width(app.view_msg_box) : 0,
                               gtk_widget_get_visible(app.view_msg_box) ? gtk_widget_get_allocated_height(app.view_msg_box) : 0);
        g_string_append(s, ", \"title\": ");
        json_str(s, gtk_window_get_title(GTK_WINDOW(app.win)));
        g_string_append(s, ", \"info_text\": ");
        GString *it = g_string_new(NULL);
        for (int r = 0; r < 9; r++) {
            GtkWidget *k = gtk_grid_get_child_at(GTK_GRID(app.info_grid), 0, r), *v = gtk_grid_get_child_at(GTK_GRID(app.info_grid), 1, r);
            if (k && v) g_string_append_printf(it, "%s: %s\n", gtk_label_get_text(GTK_LABEL(k)), gtk_label_get_text(GTK_LABEL(v)));
        }
        json_str(s, it->str);
        g_string_free(it, TRUE);
        g_string_append_c(s, '}');
    }
    g_string_append(s, "}\n");
    char *path = g_build_filename(app.test_dir, "state.json", NULL), *tmp = g_strconcat(path, ".tmp", NULL);
    if (g_file_set_contents(tmp, s->str, (gssize)s->len, NULL)) g_rename(tmp, path);
    g_free(path);
    g_free(tmp);
    g_string_free(s, TRUE);
    return G_SOURCE_REMOVE;
}

static void state_changed(void)
{
    if (app.test_dir && !app.state_id) app.state_id = g_timeout_add(30, write_state, NULL);
}

static void send_key(const char *name, gboolean ctrl, gboolean shift)
{
    guint kv = gdk_keyval_from_name(name);
    if (kv == GDK_KEY_VoidSymbol) return;
    GdkEvent *ev = gdk_event_new(GDK_KEY_PRESS);
    ev->key.window = g_object_ref(gtk_widget_get_window(app.win));
    ev->key.send_event = TRUE;
    ev->key.time = GDK_CURRENT_TIME;
    ev->key.keyval = kv;
    ev->key.state = (ctrl ? GDK_CONTROL_MASK : 0) | (shift ? GDK_SHIFT_MASK : 0);
    GdkKeymapKey *keys = NULL;
    gint n = 0;
    if (gdk_keymap_get_entries_for_keyval(gdk_keymap_get_for_display(gdk_display_get_default()), kv, &keys, &n) && n)
        ev->key.hardware_keycode = (guint16)keys[0].keycode;
    g_free(keys);
    gunichar u = gdk_keyval_to_unicode(kv);
    char buf[8] = {0};
    if (u) g_unichar_to_utf8(u, buf);
    ev->key.string = g_strdup(buf);
    ev->key.length = (gint)strlen(buf);
    gdk_event_set_device(ev, gdk_seat_get_keyboard(gdk_display_get_default_seat(gdk_display_get_default())));
    gtk_main_do_event(ev);
    gdk_event_free(ev);
}

static void test_command(char *line)
{
    g_strstrip(line);
    char **w = g_strsplit(line, " ", 2);
    const char *cmd = w[0], *arg = w[1] ? w[1] : "";
    if (!strcmp(cmd, "key")) {
        char **k = g_strsplit(arg, " ", -1);
        gboolean ctrl = FALSE, shift = FALSE;
        for (int i = 1; k[0] && k[i]; i++) {
            ctrl |= !strcmp(k[i], "ctrl");
            shift |= !strcmp(k[i], "shift");
        }
        if (k[0]) send_key(k[0], ctrl, shift);
        g_strfreev(k);
    } else if (!strcmp(cmd, "tile") || !strcmp(cmd, "ctrltile") || !strcmp(cmd, "shifttile")) {
        int i = atoi(arg);
        if (app.shown && i >= 0 && i < (int)app.shown->len) tile_clicked(app.shown->pdata[i], cmd[0] == 'c', cmd[0] == 's');
    } else if (!strcmp(cmd, "search")) {
        gtk_entry_set_text(GTK_ENTRY(app.search), arg);
    } else if (!strcmp(cmd, "page")) {
        if (!strcmp(arg, "albums")) show_albums();
        else if (!strcmp(arg, "deleted")) show_deleted();
        else if (!strcmp(arg, "favourites")) show_collection(COLL_FAVOURITES, NULL);
        else show_collection(COLL_LIBRARY, NULL);
    } else if (!strcmp(cmd, "album")) {
        show_collection(COLL_ALBUM, arg);
    } else if (!strcmp(cmd, "newalbum")) {
        new_album_named(arg);
    } else if (!strcmp(cmd, "addto")) {
        char *why = NULL;
        add_to_album(arg, TRUE, &why);
        if (why) toast(why, FALSE);
        g_free(why);
    } else if (!strcmp(cmd, "restore") && app.store->gone->len) {
        on_restore(NULL, app.store->gone->pdata[app.store->gone->len - 1]);
    } else if (!strcmp(cmd, "empty")) {
        empty_deleted();
    } else if (!strcmp(cmd, "undo")) {
        do_undo();
    } else if (!strcmp(cmd, "slideshow")) {
        on_slideshow(NULL, NULL);
    } else if (!strcmp(cmd, "zoomat")) {
        double s = 1, x = 0, y = 0;
        if (sscanf(arg, "%lf %lf %lf", &s, &x, &y) == 3) zoom_to(s, x, y);
    } else if (!strcmp(cmd, "pan")) {
        double dx = 0, dy = 0;
        if (sscanf(arg, "%lf %lf", &dx, &dy) == 2) {
            app.pan_x += dx;
            app.pan_y += dy;
            clamp_pan();
            gtk_widget_queue_draw(app.view_area);
        }
    }
    g_strfreev(w);
    state_changed();
}

static gboolean on_test_input(GIOChannel *ch, GIOCondition c, gpointer d)
{
    (void)c; (void)d;
    char *line = NULL;
    while (g_io_channel_read_line(ch, &line, NULL, NULL, NULL) == G_IO_STATUS_NORMAL && line) {
        test_command(line);
        g_free(line);
        line = NULL;
    }
    return G_SOURCE_CONTINUE;
}

static void test_setup(void)
{
    char *fifo = g_build_filename(app.test_dir, "cmd", NULL);
    int fd = open(fifo, O_RDWR | O_NONBLOCK | O_CLOEXEC);  /* RDWR: never an end of file */
    g_free(fifo);
    if (fd < 0) return;
    GIOChannel *ch = g_io_channel_unix_new(fd);
    g_io_channel_set_flags(ch, G_IO_FLAG_NONBLOCK, NULL);
    g_io_add_watch(ch, G_IO_IN, on_test_input, NULL);
}

/* ---- look ---- */

static const char *CSS =
    /* Fallback: NexaOS "Horizon". The person's theme overrides these. */
    "@define-color nexa_bg #0c1228; @define-color nexa_bg_deep #080c1e;"
    "@define-color nexa_surface #101732; @define-color nexa_surface_hi #222a46;"
    "@define-color nexa_hover #333a54; @define-color nexa_field #0f152d;"
    "@define-color nexa_border #363d5a; @define-color nexa_text #d9e0f6;"
    "@define-color nexa_text_strong #e6ecff; @define-color nexa_muted #8f9bc4;"
    "@define-color nexa_disabled #525b79; @define-color nexa_accent #4da3ff;"
    "@define-color nexa_accent_hover #6db4ff; @define-color nexa_on_accent #080c1e;"
    "@define-color nexa_selected #264779; @define-color nexa_selected_text #e6ecff;"
    "@define-color nexa_crit #ff5c7a; @define-color nexa_ok #5ee6a8; @define-color nexa_warn #ffb547;"
    "@define-color nexa_edge rgba(255, 255, 255, 0.13);"
    "* { font-family: Inter, sans-serif; }"
    "window, .page { background-color: @nexa_bg_deep; color: @nexa_text; }"
    ".heading { font-size: 15pt; font-weight: 700; color: @nexa_text_strong; }"
    ".month { font-size: 12pt; font-weight: 700; color: @nexa_text_strong; }"
    ".sub { color: @nexa_muted; font-size: 10pt; }"
    ".empty { color: @nexa_muted; font-size: 12pt; }"
    ".error { color: @nexa_crit; font-size: 8.5pt; }"
    ".name { color: @nexa_text_strong; font-weight: 600; }"
    "flowboxchild { padding: 0; border-radius: 10px; outline: none; background: none; }"
    "button { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text_strong;"
    "  border: 1px solid @nexa_border; border-radius: 10px; padding: 4px 12px; box-shadow: none; text-shadow: none;"
    "  transition: background-color 160ms cubic-bezier(0.22, 1, 0.36, 1); }"
    "button:hover { background-color: @nexa_hover; }"
    "button:disabled { color: @nexa_disabled; background-color: @nexa_surface; }"
    "button.primary { background-color: @nexa_accent; color: @nexa_on_accent; font-weight: 700; border-color: @nexa_accent; }"
    "button.primary:hover { background-color: @nexa_accent_hover; }"
    "button.danger { background-color: @nexa_crit; color: @nexa_on_accent; font-weight: 700; border-color: @nexa_crit; }"
    "button.danger-text { color: @nexa_crit; }"
    "button.nav { background-color: transparent; border-color: transparent; color: @nexa_muted; font-weight: 600; }"
    "button.nav:hover { background-color: @nexa_hover; color: @nexa_text_strong; }"
    "button.nav.on { background-color: @nexa_selected; color: @nexa_selected_text; }"
    "button.on { color: @nexa_accent; border-color: @nexa_accent; }"
    "button.card { background-color: @nexa_surface; border-radius: 16px; padding: 8px 8px 12px 8px; }"
    "button.card:hover { background-color: @nexa_surface_hi; }"
    ".card-name { color: @nexa_text_strong; font-weight: 600; font-size: 10pt; }"
    "button.menu-item { background-color: transparent; border-color: transparent; }"
    "button.menu-item:hover { background-color: @nexa_hover; }"
    "popover { background-color: @nexa_surface; border: 1px solid @nexa_border; border-radius: 10px; }"
    "entry { background-image: none; background-color: @nexa_field; color: @nexa_text; border: 1px solid @nexa_border;"
    "  border-radius: 6px; padding: 4px 8px; box-shadow: none; caret-color: @nexa_accent; }"
    "entry:focus { border-color: @nexa_accent; }"
    "entry image { color: @nexa_muted; }"
    ".selbar, .toast { background-color: @nexa_surface; border: 1px solid @nexa_border; border-radius: 16px; padding: 8px 12px;"
    "  margin: 12px; box-shadow: 0 4px 16px alpha(black, 0.4); }"
    ".gone { background-color: @nexa_surface; border-radius: 10px; padding: 8px 12px; }"
    ".viewtop { background-color: alpha(@nexa_bg_deep, 0.86); border-bottom: 1px solid @nexa_border; padding: 4px 12px; }"
    ".viewtop .name { font-size: 10pt; }"
    ".info { background-color: @nexa_surface; border-left: 1px solid @nexa_border; padding: 16px; }"
    ".info-key { color: @nexa_muted; font-size: 8.5pt; margin-right: 12px; }"
    ".info-value { color: @nexa_text_strong; }"
    ".viewmsg { color: @nexa_text_strong; background-color: alpha(@nexa_bg_deep, 0.86); border-radius: 10px; padding: 12px 16px; }"
    ".count { font-family: \"JetBrains Mono\", monospace; color: @nexa_muted; font-size: 8.5pt; }"
    "scrollbar slider { background-color: alpha(@nexa_text, 0.25); border-radius: 6px; min-width: 6px; min-height: 6px; }"
    "headerbar { background-image: none; background-color: @nexa_surface; color: @nexa_text_strong;"
    "  border-bottom: 1px solid @nexa_border; box-shadow: inset 0 1px @nexa_edge; min-height: 42px; padding: 0 8px; }"
    "headerbar .title { font-weight: 600; color: @nexa_text_strong; }"
    "headerbar button.titlebutton { background-color: transparent; color: @nexa_text; border: none; border-radius: 50%;"
    "  min-width: 26px; min-height: 26px; padding: 0; }"
    "headerbar button.titlebutton:hover { background-color: @nexa_hover; }"
    "headerbar button.titlebutton.close:hover { background-color: @nexa_crit; color: @nexa_on_accent; }"
    ".photos-dialog, .photos-dialog .dialog-action-area { background-color: @nexa_surface; color: @nexa_text; }"
    ".photos-dialog .dialog-action-area { padding: 8px 16px 16px 16px; }";

static GtkCssProvider *live_theme;

static void reload_theme(void)
{
    char *path = g_build_filename(g_get_user_config_dir(), "gtk-3.0", "gtk.css", NULL);
    gtk_css_provider_load_from_path(live_theme, path, NULL);
    g_free(path);
    if (app.win) gtk_widget_queue_draw(app.win);
}

static void on_theme_changed(GFileMonitor *m, GFile *f, GFile *other, GFileMonitorEvent ev, gpointer d)
{
    (void)m; (void)f; (void)other; (void)d;
    if (ev != G_FILE_MONITOR_EVENT_DELETED) reload_theme();
}

static void follow_theme(void)
{
    live_theme = gtk_css_provider_new();
    reload_theme();
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(live_theme),
                                              GTK_STYLE_PROVIDER_PRIORITY_USER + 1);
    char *path = g_build_filename(g_get_user_config_dir(), "gtk-3.0", "gtk.css", NULL);
    GFile *file = g_file_new_for_path(path);
    GFileMonitor *mon = g_file_monitor_file(file, G_FILE_MONITOR_NONE, NULL, NULL);
    if (mon) g_signal_connect(mon, "changed", G_CALLBACK(on_theme_changed), NULL);
    g_object_unref(file);
    g_free(path);
}

/* ---- building the window ---- */

static GtkWidget *page_box(void)
{
    GtkWidget *b = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    add_class(b, "page");
    return b;
}

static GtkWidget *build_grid_page(void)
{
    GtkWidget *overlay = gtk_overlay_new();
    GtkWidget *page = page_box();
    GtkWidget *head = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
    g_object_set(head, "margin-start", 24, "margin-end", 24, "margin-top", 16, "margin-bottom", 8, NULL);
    app.grid_back = button("‹ Albums", "Back to the albums (Esc)", G_CALLBACK(on_back), NULL, NULL);
    gtk_widget_set_valign(app.grid_back, GTK_ALIGN_CENTER);
    keep_hidden(app.grid_back);
    gtk_box_pack_start(GTK_BOX(head), app.grid_back, FALSE, FALSE, 0);
    GtkWidget *titles = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    app.grid_head = label("Library", "heading", 0);
    gtk_label_set_line_wrap(GTK_LABEL(app.grid_head), FALSE);
    gtk_label_set_ellipsize(GTK_LABEL(app.grid_head), PANGO_ELLIPSIZE_END);
    app.grid_sub = label("", "sub", 0);
    gtk_box_pack_start(GTK_BOX(titles), app.grid_head, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(titles), app.grid_sub, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(head), titles, TRUE, TRUE, 0);
    app.album_tools = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(app.album_tools), button("Rename", "Rename this album", G_CALLBACK(on_rename_album), NULL, NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.album_tools), button("Delete Album", "The photos stay in your library", G_CALLBACK(on_delete_album), NULL, "danger-text"),
                       FALSE, FALSE, 0);
    gtk_widget_set_valign(app.album_tools, GTK_ALIGN_CENTER);
    keep_hidden(app.album_tools);
    gtk_box_pack_start(GTK_BOX(head), app.album_tools, FALSE, FALSE, 0);
    app.slide_btn = button("▶ Slideshow", "Show these photos one after another (F5)", G_CALLBACK(on_slideshow), NULL, "primary");
    gtk_widget_set_valign(app.slide_btn, GTK_ALIGN_CENTER);
    gtk_box_pack_start(GTK_BOX(head), app.slide_btn, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(page), head, FALSE, FALSE, 0);

    app.grid_empty = label("", "empty", 0.5);
    gtk_label_set_justify(GTK_LABEL(app.grid_empty), GTK_JUSTIFY_CENTER);
    gtk_label_set_max_width_chars(GTK_LABEL(app.grid_empty), 50);
    gtk_widget_set_no_show_all(app.grid_empty, TRUE);
    g_object_set(app.grid_empty, "margin-top", 80, "margin-start", 24, "margin-end", 24, NULL);
    gtk_box_pack_start(GTK_BOX(page), app.grid_empty, FALSE, FALSE, 0);

    app.grid_scroll = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(app.grid_scroll), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    app.grid_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
    g_object_set(app.grid_box, "margin-start", 24, "margin-end", 24, "margin-top", 8, "margin-bottom", 96, NULL);
    gtk_container_add(GTK_CONTAINER(app.grid_scroll), app.grid_box);
    gtk_box_pack_start(GTK_BOX(page), app.grid_scroll, TRUE, TRUE, 0);
    gtk_container_add(GTK_CONTAINER(overlay), page);

    /* What to do with the selected photos */
    GtkWidget *bar = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    add_class(bar, "selbar");
    app.sel_label = label("", "name", 0);
    gtk_label_set_line_wrap(GTK_LABEL(app.sel_label), FALSE);
    gtk_box_pack_start(GTK_BOX(bar), app.sel_label, FALSE, FALSE, 8);
    gtk_box_pack_start(GTK_BOX(bar), button("Add to Album", "Put them in an album", G_CALLBACK(on_add_album), NULL, NULL), FALSE, FALSE, 0);
    app.sel_fav = button("Favourite", "Mark them as favourites (F)", G_CALLBACK(on_sel_fav), NULL, NULL);
    gtk_box_pack_start(GTK_BOX(bar), app.sel_fav, FALSE, FALSE, 0);
    app.sel_remove = button("Remove from Album", "The photos stay in your library", G_CALLBACK(on_sel_remove), NULL, NULL);
    keep_hidden(app.sel_remove);
    gtk_box_pack_start(GTK_BOX(bar), app.sel_remove, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(bar), button("Delete", "Move them to Deleted (Delete)", G_CALLBACK(on_sel_delete), NULL, "danger-text"), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(bar), button("Cancel", "Select nothing (Esc)", G_CALLBACK(on_sel_cancel), NULL, NULL), FALSE, FALSE, 0);
    app.selbar = gtk_revealer_new();
    gtk_revealer_set_transition_type(GTK_REVEALER(app.selbar), GTK_REVEALER_TRANSITION_TYPE_SLIDE_UP);
    gtk_revealer_set_transition_duration(GTK_REVEALER(app.selbar), 240);
    gtk_container_add(GTK_CONTAINER(app.selbar), bar);
    gtk_widget_set_halign(app.selbar, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(app.selbar, GTK_ALIGN_END);
    gtk_overlay_add_overlay(GTK_OVERLAY(overlay), app.selbar);
    return overlay;
}

static GtkWidget *build_albums_page(void)
{
    GtkWidget *page = page_box();
    GtkWidget *head = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
    g_object_set(head, "margin-start", 24, "margin-end", 24, "margin-top", 16, "margin-bottom", 8, NULL);
    GtkWidget *titles = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    gtk_box_pack_start(GTK_BOX(titles), label("Albums", "heading", 0), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(titles), label("Your photos stay where they are: an album only lists them.", "sub", 0), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(head), titles, TRUE, TRUE, 0);
    GtkWidget *nb = button("New Album", "Make an empty album", G_CALLBACK(on_new_album), NULL, "primary");
    gtk_widget_set_valign(nb, GTK_ALIGN_CENTER);
    gtk_box_pack_start(GTK_BOX(head), nb, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(page), head, FALSE, FALSE, 0);
    GtkWidget *scroll = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(scroll), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    app.albums_flow = gtk_flow_box_new();
    gtk_flow_box_set_selection_mode(GTK_FLOW_BOX(app.albums_flow), GTK_SELECTION_NONE);
    gtk_flow_box_set_homogeneous(GTK_FLOW_BOX(app.albums_flow), TRUE);
    gtk_flow_box_set_max_children_per_line(GTK_FLOW_BOX(app.albums_flow), 12);
    gtk_flow_box_set_column_spacing(GTK_FLOW_BOX(app.albums_flow), 16);
    gtk_flow_box_set_row_spacing(GTK_FLOW_BOX(app.albums_flow), 16);
    g_object_set(app.albums_flow, "margin", 24, "valign", GTK_ALIGN_START, NULL);
    gtk_container_add(GTK_CONTAINER(scroll), app.albums_flow);
    gtk_box_pack_start(GTK_BOX(page), scroll, TRUE, TRUE, 0);
    return page;
}

static GtkWidget *build_deleted_page(void)
{
    GtkWidget *page = page_box();
    GtkWidget *head = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
    g_object_set(head, "margin-start", 24, "margin-end", 24, "margin-top", 16, "margin-bottom", 8, NULL);
    GtkWidget *titles = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    gtk_box_pack_start(GTK_BOX(titles), label("Deleted", "heading", 0), FALSE, FALSE, 0);
    app.del_sub = label("", "sub", 0);
    gtk_box_pack_start(GTK_BOX(titles), app.del_sub, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(head), titles, TRUE, TRUE, 0);
    app.del_tools = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(app.del_tools), button("Put All Back", "Every photo back where it was", G_CALLBACK(on_restore_all), NULL, NULL),
                       FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.del_tools), button("Empty Deleted", "Delete them all for good", G_CALLBACK(on_empty), NULL, "danger"),
                       FALSE, FALSE, 0);
    gtk_widget_set_valign(app.del_tools, GTK_ALIGN_CENTER);
    keep_hidden(app.del_tools);
    gtk_box_pack_start(GTK_BOX(head), app.del_tools, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(page), head, FALSE, FALSE, 0);
    app.del_empty = label("Nothing here. Photos you delete wait here for 30 days, so you can put them back.", "empty", 0.5);
    gtk_label_set_justify(GTK_LABEL(app.del_empty), GTK_JUSTIFY_CENTER);
    gtk_label_set_max_width_chars(GTK_LABEL(app.del_empty), 50);
    g_object_set(app.del_empty, "margin-top", 80, "margin-start", 24, "margin-end", 24, NULL);
    gtk_widget_set_no_show_all(app.del_empty, TRUE);
    gtk_box_pack_start(GTK_BOX(page), app.del_empty, FALSE, FALSE, 0);
    GtkWidget *scroll = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(scroll), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    app.del_list = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
    g_object_set(app.del_list, "margin", 24, NULL);
    gtk_container_add(GTK_CONTAINER(scroll), app.del_list);
    gtk_box_pack_start(GTK_BOX(page), scroll, TRUE, TRUE, 0);
    return page;
}

static GtkWidget *build_viewer(void)
{
    GtkWidget *outer = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    add_class(outer, "page");
    GtkWidget *top = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    add_class(top, "viewtop");
    app.view_top = top;
    gtk_box_pack_start(GTK_BOX(top), button("‹ Back", "Back to the photos (Esc)", G_CALLBACK(on_view_back), NULL, NULL), FALSE, FALSE, 0);
    GtkWidget *titles = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    app.view_name = label("", "name", 0);
    gtk_label_set_line_wrap(GTK_LABEL(app.view_name), FALSE);
    gtk_label_set_ellipsize(GTK_LABEL(app.view_name), PANGO_ELLIPSIZE_MIDDLE);
    app.view_when = label("", "sub", 0);
    gtk_label_set_line_wrap(GTK_LABEL(app.view_when), FALSE);
    gtk_label_set_ellipsize(GTK_LABEL(app.view_when), PANGO_ELLIPSIZE_END);
    gtk_box_pack_start(GTK_BOX(titles), app.view_name, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(titles), app.view_when, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(top), titles, TRUE, TRUE, 4);
    app.view_count = label("", "count", 1);
    gtk_box_pack_start(GTK_BOX(top), app.view_count, FALSE, FALSE, 4);
    app.fav_btn = button("⬡ Favourite", "Favourite (F)", G_CALLBACK(on_fav_btn), NULL, NULL);
    gtk_box_pack_start(GTK_BOX(top), app.fav_btn, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(top), button("↺", "Turn left (Shift+R)", G_CALLBACK(on_turn_left), NULL, NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(top), button("↻", "Turn right (R)", G_CALLBACK(on_turn_right), NULL, NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(top), button("Details", "When, where and how it was taken (I)", G_CALLBACK(on_info_btn), NULL, NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(top), button("▶", "Slideshow (F5)", G_CALLBACK(on_slideshow), NULL, NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(top), button("Delete", "Move it to Deleted (Delete)", G_CALLBACK(on_view_delete), NULL, "danger-text"), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(outer), top, FALSE, FALSE, 0);

    GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
    GtkWidget *overlay = gtk_overlay_new();
    app.view_area = gtk_drawing_area_new();
    gtk_widget_set_can_focus(app.view_area, TRUE);
    gtk_widget_add_events(app.view_area, GDK_BUTTON_PRESS_MASK | GDK_BUTTON_RELEASE_MASK | GDK_POINTER_MOTION_MASK |
                                             GDK_SCROLL_MASK | GDK_SMOOTH_SCROLL_MASK);
    g_signal_connect(app.view_area, "draw", G_CALLBACK(draw_view), NULL);
    g_signal_connect(app.view_area, "button-press-event", G_CALLBACK(on_view_press), NULL);
    g_signal_connect(app.view_area, "button-release-event", G_CALLBACK(on_view_release), NULL);
    g_signal_connect(app.view_area, "motion-notify-event", G_CALLBACK(on_view_motion), NULL);
    g_signal_connect(app.view_area, "scroll-event", G_CALLBACK(on_view_scroll), NULL);
    gtk_container_add(GTK_CONTAINER(overlay), app.view_area);
    app.view_msg = label("", NULL, 0.5);
    gtk_label_set_max_width_chars(GTK_LABEL(app.view_msg), 46);
    gtk_label_set_justify(GTK_LABEL(app.view_msg), GTK_JUSTIFY_CENTER);
    GtkWidget *msgbox = gtk_event_box_new();  /* (a label draws no background) */
    add_class(msgbox, "viewmsg");
    gtk_container_add(GTK_CONTAINER(msgbox), app.view_msg);
    gtk_widget_set_halign(msgbox, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(msgbox, GTK_ALIGN_CENTER);
    gtk_widget_show(app.view_msg);
    keep_hidden(msgbox);
    app.view_msg_box = msgbox;
    gtk_overlay_add_overlay(GTK_OVERLAY(overlay), msgbox);
    gtk_box_pack_start(GTK_BOX(row), overlay, TRUE, TRUE, 0);

    /* Details: a fixed-width panel (a wrapping label would otherwise ask
     * for the width of its whole text). */
    GtkWidget *info = gtk_box_new(GTK_ORIENTATION_VERTICAL, 12);
    app.info_rev = info;
    add_class(info, "info");
    gtk_widget_set_size_request(info, 320, -1);
    gtk_widget_set_hexpand(info, FALSE);
    gtk_box_pack_start(GTK_BOX(info), label("Details", "month", 0), FALSE, FALSE, 0);
    app.info_grid = gtk_grid_new();
    gtk_grid_set_row_spacing(GTK_GRID(app.info_grid), 8);
    gtk_box_pack_start(GTK_BOX(info), app.info_grid, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(row), info, FALSE, FALSE, 0);
    keep_hidden(info);
    gtk_box_pack_start(GTK_BOX(outer), row, TRUE, TRUE, 0);
    return outer;
}

static void build_window(void)
{
    app.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_default_size(GTK_WINDOW(app.win), 1100, 700);
    gtk_window_set_icon_name(GTK_WINDOW(app.win), APP_ID);
    g_signal_connect(app.win, "realize", G_CALLBACK(fit_to_screen), NULL);
    GtkWidget *bar = gtk_header_bar_new();
    gtk_header_bar_set_show_close_button(GTK_HEADER_BAR(bar), FALSE);
    GtkWidget *nav = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    const char *names[3] = {"Library", "Albums", "Deleted"}, *tips[3] = {"All your photos", "Favourites and your albums",
                                                                         "Photos you deleted: put them back for 30 days"};
    GCallback cbs[3] = {G_CALLBACK(on_nav_library), G_CALLBACK(on_nav_albums), G_CALLBACK(on_nav_deleted)};
    for (int i = 0; i < 3; i++) {
        app.nav[i] = button(names[i], tips[i], cbs[i], NULL, "nav");
        gtk_widget_set_can_focus(app.nav[i], FALSE);
        gtk_box_pack_start(GTK_BOX(nav), app.nav[i], FALSE, FALSE, 0);
    }
    gtk_header_bar_pack_start(GTK_HEADER_BAR(bar), nav);
    gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), title_button("window-close-symbolic", "Close (Ctrl+Q)", "close", G_CALLBACK(on_close)));
    gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), title_button("window-maximize-symbolic", "Maximize", "maximize", G_CALLBACK(on_maximize)));
    gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), title_button("window-minimize-symbolic", "Minimize", "minimize", G_CALLBACK(on_minimize)));
    app.search = gtk_search_entry_new();
    gtk_entry_set_placeholder_text(GTK_ENTRY(app.search), "Search: a month, a folder, a camera…");
    gtk_widget_set_tooltip_text(app.search, "Search the library (Ctrl+F)");
    gtk_entry_set_width_chars(GTK_ENTRY(app.search), 28);
    g_signal_connect(app.search, "search-changed", G_CALLBACK(on_search), NULL);
    gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), app.search);
    gtk_window_set_titlebar(GTK_WINDOW(app.win), bar);
    gtk_window_set_title(GTK_WINDOW(app.win), "Photos");

    GtkWidget *overlay = gtk_overlay_new();
    app.stack = gtk_stack_new();
    gtk_stack_set_transition_type(GTK_STACK(app.stack), GTK_STACK_TRANSITION_TYPE_CROSSFADE);
    gtk_stack_set_transition_duration(GTK_STACK(app.stack), 160);
    gtk_stack_add_named(GTK_STACK(app.stack), build_grid_page(), "grid");
    gtk_stack_add_named(GTK_STACK(app.stack), build_albums_page(), "albums");
    gtk_stack_add_named(GTK_STACK(app.stack), build_deleted_page(), "deleted");
    gtk_stack_add_named(GTK_STACK(app.stack), build_viewer(), "viewer");
    gtk_container_add(GTK_CONTAINER(overlay), app.stack);

    GtkWidget *tbox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
    add_class(tbox, "toast");
    app.toast_label = label("", NULL, 0);
    gtk_label_set_max_width_chars(GTK_LABEL(app.toast_label), 60);
    gtk_box_pack_start(GTK_BOX(tbox), app.toast_label, TRUE, TRUE, 4);
    app.toast_undo = button("Undo", "Put them back (Ctrl+Z)", G_CALLBACK(on_undo), NULL, "primary");
    keep_hidden(app.toast_undo);
    gtk_box_pack_start(GTK_BOX(tbox), app.toast_undo, FALSE, FALSE, 0);
    app.toast_rev = gtk_revealer_new();
    gtk_revealer_set_transition_type(GTK_REVEALER(app.toast_rev), GTK_REVEALER_TRANSITION_TYPE_CROSSFADE);
    gtk_revealer_set_transition_duration(GTK_REVEALER(app.toast_rev), 240);
    gtk_container_add(GTK_CONTAINER(app.toast_rev), tbox);
    gtk_widget_set_halign(app.toast_rev, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(app.toast_rev, GTK_ALIGN_START);
    gtk_overlay_add_overlay(GTK_OVERLAY(overlay), app.toast_rev);
    gtk_overlay_set_overlay_pass_through(GTK_OVERLAY(overlay), app.toast_rev, FALSE);
    gtk_container_add(GTK_CONTAINER(app.win), overlay);

    g_signal_connect(app.win, "key-press-event", G_CALLBACK(on_key), NULL);
    g_signal_connect(app.win, "window-state-event", G_CALLBACK(on_window_state), NULL);
    g_signal_connect(app.win, "destroy", G_CALLBACK(gtk_main_quit), NULL);
}

/* A file given on the command line: the viewer, with its neighbours when it
 * is in the library. */
static void open_file(const char *arg)
{
    char *path = g_canonicalize_filename(arg, NULL);
    Photo *p = photo_read(path, app.root);
    if (!p) {
        toast("Photos can't open that file: it isn't a picture Photos knows, or it can't be read.", FALSE);
        g_free(path);
        return;
    }
    if (p->rel && app.scanned) {
        Photo *in = g_hash_table_lookup(app.by_path, p->path);
        photo_free(p);
        if (in) {
            collect();
            int i = shown_index(in);
            if (i >= 0) open_viewer(app.shown, i);
            g_free(path);
            return;
        }
        p = photo_read(path, app.root);
    }
    app.extra = p;
    GPtrArray *one = g_ptr_array_new();
    g_ptr_array_add(one, p);
    open_viewer(one, 0);
    g_ptr_array_free(one, TRUE);
    g_free(path);
}

static char *pending_file;

static gboolean first_scan_then_open(gpointer d)
{
    (void)d;
    if (!app.scanned) return G_SOURCE_CONTINUE;
    open_file(pending_file);
    g_clear_pointer(&pending_file, g_free);
    return G_SOURCE_REMOVE;
}

int main(int argc, char **argv)
{
    gtk_init(&argc, &argv);
    g_set_prgname(APP_ID);
    g_set_application_name("Photos");
    const char *pics = g_get_user_special_dir(G_USER_DIRECTORY_PICTURES);
    app.root = g_strdup(pics ? pics : g_build_filename(g_get_home_dir(), "Pictures", NULL));
    app.cache = g_build_filename(g_get_user_cache_dir(), APP_ID, NULL);
    app.data = g_build_filename(g_get_user_data_dir(), APP_ID, NULL);
    g_mkdir_with_parents(app.data, 0700);
    g_mkdir_with_parents(app.cache, 0700);
    app.store = store_load(app.data);
    if (store_purge(app.store, g_get_real_time() / G_USEC_PER_SEC)) save_store();
    app.all = g_ptr_array_new_with_free_func((GDestroyNotify)photo_free);
    app.by_path = g_hash_table_new(g_str_hash, g_str_equal);
    app.tile_of = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    app.selected = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    app.thumbs = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, thumb_free);
    app.wanted = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    app.monitors = g_ptr_array_new_with_free_func(g_object_unref);
    app.anchor = -1;
    app.fit = TRUE;
    app.animations = TRUE;
    app.test_dir = g_strdup(g_getenv("PHOTOS_TEST"));
    int threads = CLAMP((int)g_get_num_processors() - 1, 1, 4);
    app.pool = g_thread_pool_new(thumb_work, NULL, threads, FALSE, NULL);

    GtkCssProvider *css = gtk_css_provider_new();
    gtk_css_provider_load_from_data(css, CSS, -1, NULL);
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(css), GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    follow_theme();

    build_window();
    gtk_widget_show_all(app.win);
    app.tile_px = TILE * gtk_widget_get_scale_factor(app.win);
    if (app.tile_px > THUMB_PX) app.tile_px = THUMB_PX;
    set_nav(0);
    if (app.test_dir) test_setup();
    fill_grid();
    rescan();
    if (argc > 1) {
        pending_file = g_strdup(argv[1]);
        g_timeout_add(50, first_scan_then_open, NULL);
    }
    gtk_main();
    g_atomic_int_set(&app.cancel, 1);
    return 0;
}
