/* Video Player: a NexaOS app in C (GTK 3 + GStreamer).
 *
 * playbin decodes (every codec on the system: gst-libav, good/bad/ugly);
 * frames come back through an appsink as BGRx and are drawn with cairo,
 * scaled to fit, so the player needs no GL and no GTK video plugin.
 *
 *   video-player [FILE]     play FILE, or show the ~/Videos library
 *
 * Keys: Space play/pause, Left/Right -/+10 s, Up/Down volume, M mute,
 * F or double-click fullscreen, Esc leave fullscreen / back to library,
 * [ ] slower/faster, O open. Remembers where you stopped in each video.
 */
#include <gst/app/gstappsink.h>
#include <gst/gst.h>
#include <gtk/gtk.h>
#include <math.h>
#include <string.h>

#define APP_ID "video-player"
#define HIDE_AFTER_MS 2500
#define RESUME_FILE "positions"

typedef struct {
    GtkWidget *win, *stack, *library_list, *library_empty, *video_area, *controls, *revealer;
    GtkWidget *play_btn, *seek, *time_label, *volume, *speed_btn, *title_label, *full_btn;
    GstElement *playbin;
    GMutex frame_lock;
    cairo_surface_t *frame; /* latest decoded frame (BGRx) */
    int frame_w, frame_h;
    gboolean playing, fullscreen, seeking, ended;
    double rate;
    gint64 duration;
    char *uri;
    guint hide_id, tick_id;
    GKeyFile *positions;
} App;

static App app;

/* ---- helpers -------------------------------------------------------- */

static char *data_path(const char *name)
{
    char *dir = g_build_filename(g_get_user_data_dir(), APP_ID, NULL);
    g_mkdir_with_parents(dir, 0700);
    char *p = g_build_filename(dir, name, NULL);
    g_free(dir);
    return p;
}

static void format_time(gint64 ns, char *out, size_t n)
{
    if (ns < 0)
        ns = 0;
    gint64 s = ns / GST_SECOND;
    if (s >= 3600)
        g_snprintf(out, n, "%" G_GINT64_FORMAT ":%02d:%02d", s / 3600, (int)(s / 60 % 60), (int)(s % 60));
    else
        g_snprintf(out, n, "%d:%02d", (int)(s / 60), (int)(s % 60));
}

static gboolean is_video_name(const char *name)
{
    static const char *ext[] = {".mp4", ".mkv", ".webm", ".avi", ".mov", ".m4v", ".ogv", ".mpg", ".mpeg", ".wmv", ".flv", ".3gp", ".ts", NULL};
    char *lower = g_ascii_strdown(name, -1);
    gboolean yes = FALSE;
    for (int i = 0; ext[i] && !yes; i++)
        yes = g_str_has_suffix(lower, ext[i]);
    g_free(lower);
    return yes;
}

static void save_position(void)
{
    if (!app.uri || !app.playbin || !app.positions)
        return;
    gint64 pos = 0;
    if (!gst_element_query_position(app.playbin, GST_FORMAT_TIME, &pos))
        return;
    /* Near the start or the end: nothing to resume. */
    if (app.ended || pos < 15 * GST_SECOND || (app.duration > 0 && pos > app.duration - 20 * GST_SECOND))
        g_key_file_remove_key(app.positions, "positions", app.uri, NULL);
    else
        g_key_file_set_int64(app.positions, "positions", app.uri, pos);
    char *path = data_path(RESUME_FILE);
    g_key_file_save_to_file(app.positions, path, NULL);
    g_free(path);
}

/* ---- frames --------------------------------------------------------- */

static gboolean queue_redraw(gpointer data)
{
    (void)data;
    if (app.video_area)
        gtk_widget_queue_draw(app.video_area);
    return G_SOURCE_REMOVE;
}

static GstFlowReturn on_new_sample(GstAppSink *sink, gpointer data)
{
    (void)data;
    GstSample *sample = gst_app_sink_pull_sample(sink);
    if (!sample)
        return GST_FLOW_EOS;
    GstCaps *caps = gst_sample_get_caps(sample);
    GstStructure *st = gst_caps_get_structure(caps, 0);
    int w = 0, h = 0;
    gst_structure_get_int(st, "width", &w);
    gst_structure_get_int(st, "height", &h);
    GstBuffer *buf = gst_sample_get_buffer(sample);
    GstMapInfo map;
    if (w > 0 && h > 0 && gst_buffer_map(buf, &map, GST_MAP_READ)) {
        int stride = cairo_format_stride_for_width(CAIRO_FORMAT_RGB24, w);
        int src_stride = (int)(map.size / (gsize)h);
        g_mutex_lock(&app.frame_lock);
        if (!app.frame || app.frame_w != w || app.frame_h != h) {
            if (app.frame)
                cairo_surface_destroy(app.frame);
            app.frame = cairo_image_surface_create(CAIRO_FORMAT_RGB24, w, h);
            app.frame_w = w;
            app.frame_h = h;
        }
        cairo_surface_flush(app.frame);
        unsigned char *dst = cairo_image_surface_get_data(app.frame);
        int row = MIN(stride, src_stride);
        for (int y = 0; y < h; y++)
            memcpy(dst + (gsize)y * stride, map.data + (gsize)y * src_stride, row);
        cairo_surface_mark_dirty(app.frame);
        g_mutex_unlock(&app.frame_lock);
        gst_buffer_unmap(buf, &map);
        g_idle_add(queue_redraw, NULL);
    }
    gst_sample_unref(sample);
    return GST_FLOW_OK;
}

/* A pointy-top hexagon, the NexaOS shape. */
static void hex_path(cairo_t *cr, double cx, double cy, double r)
{
    for (int i = 0; i < 6; i++) {
        double a = (-90 + 60 * i) * G_PI / 180.0;
        if (i == 0)
            cairo_move_to(cr, cx + r * cos(a), cy + r * sin(a));
        else
            cairo_line_to(cr, cx + r * cos(a), cy + r * sin(a));
    }
    cairo_close_path(cr);
}

static gboolean on_draw_video(GtkWidget *w, cairo_t *cr, gpointer data)
{
    (void)data;
    int aw = gtk_widget_get_allocated_width(w), ah = gtk_widget_get_allocated_height(w);
    cairo_set_source_rgb(cr, 0, 0, 0);
    cairo_paint(cr);
    g_mutex_lock(&app.frame_lock);
    if (app.frame) {
        double s = MIN((double)aw / app.frame_w, (double)ah / app.frame_h);
        double x = (aw - app.frame_w * s) / 2, y = (ah - app.frame_h * s) / 2;
        cairo_save(cr);
        cairo_translate(cr, x, y);
        cairo_scale(cr, s, s);
        cairo_set_source_surface(cr, app.frame, 0, 0);
        cairo_pattern_set_filter(cairo_get_source(cr), s < 1 ? CAIRO_FILTER_GOOD : CAIRO_FILTER_BILINEAR);
        cairo_paint(cr);
        cairo_restore(cr);
    }
    g_mutex_unlock(&app.frame_lock);
    if (!app.playing) {
        /* Big hexagon play button while paused, in the theme's colours. */
        GtkStyleContext *ctx = gtk_widget_get_style_context(w);
        GdkRGBA accent = {0.30, 0.64, 1.0, 1}, bg = {0.03, 0.05, 0.12, 1}, fg = {0.9, 0.93, 1.0, 1};
        gtk_style_context_lookup_color(ctx, "nexa_accent", &accent);
        gtk_style_context_lookup_color(ctx, "nexa_bg_deep", &bg);
        gtk_style_context_lookup_color(ctx, "nexa_text_strong", &fg);
        double cx = aw / 2.0, cy = ah / 2.0, r = 46;
        hex_path(cr, cx, cy, r);
        cairo_set_source_rgba(cr, bg.red, bg.green, bg.blue, 0.62);
        cairo_fill_preserve(cr);
        cairo_set_source_rgba(cr, accent.red, accent.green, accent.blue, 0.95);
        cairo_set_line_width(cr, 2.5);
        cairo_stroke(cr);
        cairo_move_to(cr, cx - 12, cy - 18);
        cairo_line_to(cr, cx + 20, cy);
        cairo_line_to(cr, cx - 12, cy + 18);
        cairo_close_path(cr);
        gdk_cairo_set_source_rgba(cr, &fg);
        cairo_fill(cr);
    }
    return TRUE;
}

/* ---- controls ------------------------------------------------------- */

static void update_play_button(void)
{
    gtk_button_set_label(GTK_BUTTON(app.play_btn), app.playing ? "❚❚" : "▶");
    gtk_widget_set_tooltip_text(app.play_btn, app.playing ? "Pause (Space)" : "Play (Space)");
    gtk_widget_queue_draw(app.video_area);
}

static void set_playing(gboolean on)
{
    if (!app.playbin)
        return;
    if (on && app.ended) {
        gst_element_seek_simple(app.playbin, GST_FORMAT_TIME, GST_SEEK_FLAG_FLUSH, 0);
        app.ended = FALSE;
    }
    gst_element_set_state(app.playbin, on ? GST_STATE_PLAYING : GST_STATE_PAUSED);
    app.playing = on;
    update_play_button();
}

static void seek_to(gint64 pos)
{
    if (!app.playbin)
        return;
    if (pos < 0)
        pos = 0;
    if (app.duration > 0 && pos > app.duration)
        pos = app.duration - GST_SECOND / 2;
    app.ended = FALSE;
    gst_element_seek(app.playbin, app.rate, GST_FORMAT_TIME, GST_SEEK_FLAG_FLUSH | GST_SEEK_FLAG_ACCURATE,
                     GST_SEEK_TYPE_SET, pos, GST_SEEK_TYPE_NONE, GST_CLOCK_TIME_NONE);
}

static void seek_by(gint64 delta)
{
    gint64 pos = 0;
    if (gst_element_query_position(app.playbin, GST_FORMAT_TIME, &pos))
        seek_to(pos + delta);
}

static gboolean on_tick(gpointer data)
{
    (void)data;
    if (!app.playbin)
        return G_SOURCE_CONTINUE;
    gint64 pos = 0, dur = 0;
    if (gst_element_query_duration(app.playbin, GST_FORMAT_TIME, &dur) && dur > 0)
        app.duration = dur;
    if (gst_element_query_position(app.playbin, GST_FORMAT_TIME, &pos) && !app.seeking && app.duration > 0) {
        gtk_range_set_range(GTK_RANGE(app.seek), 0, (double)app.duration / GST_SECOND);
        gtk_range_set_value(GTK_RANGE(app.seek), (double)pos / GST_SECOND);
    }
    char a[32], b[32], t[80];
    format_time(pos, a, sizeof a);
    format_time(app.duration, b, sizeof b);
    g_snprintf(t, sizeof t, "%s / %s", a, b);
    gtk_label_set_text(GTK_LABEL(app.time_label), t);
    return G_SOURCE_CONTINUE;
}

static gboolean hide_controls(gpointer data)
{
    (void)data;
    app.hide_id = 0;
    if (app.playing) {
        gtk_revealer_set_reveal_child(GTK_REVEALER(app.revealer), FALSE);
        GdkWindow *gw = gtk_widget_get_window(app.video_area);
        if (gw) {
            GdkCursor *none = gdk_cursor_new_from_name(gdk_window_get_display(gw), "none");
            gdk_window_set_cursor(gw, none);
            if (none)
                g_object_unref(none);
        }
    }
    return G_SOURCE_REMOVE;
}

static void show_controls(void)
{
    gtk_revealer_set_reveal_child(GTK_REVEALER(app.revealer), TRUE);
    GdkWindow *gw = gtk_widget_get_window(app.video_area);
    if (gw)
        gdk_window_set_cursor(gw, NULL);
    if (app.hide_id)
        g_source_remove(app.hide_id);
    app.hide_id = g_timeout_add(HIDE_AFTER_MS, hide_controls, NULL);
}

static void on_play_clicked(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    set_playing(!app.playing);
}

static gboolean on_seek_press(GtkWidget *w, GdkEvent *e, gpointer d)
{
    (void)w; (void)e; (void)d;
    app.seeking = TRUE;
    return FALSE;
}

static gboolean on_seek_release(GtkWidget *w, GdkEvent *e, gpointer d)
{
    (void)e; (void)d;
    app.seeking = FALSE;
    seek_to((gint64)(gtk_range_get_value(GTK_RANGE(w)) * GST_SECOND));
    return FALSE;
}

static void on_volume_changed(GtkScaleButton *b, double v, gpointer d)
{
    (void)b; (void)d;
    if (app.playbin)
        g_object_set(app.playbin, "volume", v * v, NULL); /* perceptual curve */
}

static void set_rate(double rate)
{
    app.rate = CLAMP(rate, 0.25, 3.0);
    char label[16];
    g_snprintf(label, sizeof label, "%.2gx", app.rate);
    gtk_button_set_label(GTK_BUTTON(app.speed_btn), label);
    seek_by(0);
}

static void on_speed_clicked(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    static const double steps[] = {1.0, 1.25, 1.5, 2.0, 0.5, 0.75};
    int i = 0;
    while (i < 6 && fabs(steps[i] - app.rate) > 0.01)
        i++;
    set_rate(steps[(i + 1) % 6]);
}

static void toggle_fullscreen(void)
{
    if (app.fullscreen)
        gtk_window_unfullscreen(GTK_WINDOW(app.win));
    else
        gtk_window_fullscreen(GTK_WINDOW(app.win));
}

static gboolean on_window_state(GtkWidget *w, GdkEventWindowState *e, gpointer d)
{
    (void)w; (void)d;
    app.fullscreen = (e->new_window_state & GDK_WINDOW_STATE_FULLSCREEN) != 0;
    gtk_button_set_label(GTK_BUTTON(app.full_btn), app.fullscreen ? "⤡" : "⤢");
    return FALSE;
}

static void on_full_clicked(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    toggle_fullscreen();
}

/* ---- playback ------------------------------------------------------- */

static void stop_playback(void)
{
    if (!app.playbin)
        return;
    save_position();
    gst_element_set_state(app.playbin, GST_STATE_NULL);
    app.playing = FALSE;
    g_mutex_lock(&app.frame_lock);
    if (app.frame) {
        cairo_surface_destroy(app.frame);
        app.frame = NULL;
    }
    g_mutex_unlock(&app.frame_lock);
}

static void show_library(void);

static gboolean on_bus(GstBus *bus, GstMessage *msg, gpointer d)
{
    (void)bus; (void)d;
    switch (GST_MESSAGE_TYPE(msg)) {
    case GST_MESSAGE_EOS:
        app.ended = TRUE;
        app.playing = FALSE;
        gst_element_set_state(app.playbin, GST_STATE_PAUSED);
        update_play_button();
        show_controls();
        save_position();
        break;
    case GST_MESSAGE_ERROR: {
        GError *err = NULL;
        gst_message_parse_error(msg, &err, NULL);
        GtkWidget *dlg = gtk_message_dialog_new(GTK_WINDOW(app.win), GTK_DIALOG_MODAL, GTK_MESSAGE_OTHER,
                                                GTK_BUTTONS_CLOSE, "Can't play this video");
        gtk_message_dialog_format_secondary_text(GTK_MESSAGE_DIALOG(dlg), "%s", err ? err->message : "unknown error");
        gtk_dialog_run(GTK_DIALOG(dlg));
        gtk_widget_destroy(dlg);
        g_clear_error(&err);
        stop_playback();
        show_library();
        break;
    }
    default:
        break;
    }
    return TRUE;
}

static void play_uri(const char *uri)
{
    stop_playback();
    g_free(app.uri);
    app.uri = g_strdup(uri);
    app.duration = 0;
    app.ended = FALSE;
    app.rate = 1.0;
    gtk_button_set_label(GTK_BUTTON(app.speed_btn), "1x");

    char *path = g_filename_from_uri(uri, NULL, NULL);
    char *base = path ? g_path_get_basename(path) : g_strdup(uri);
    gtk_label_set_text(GTK_LABEL(app.title_label), base);
    gtk_window_set_title(GTK_WINDOW(app.win), base);

    g_object_set(app.playbin, "uri", uri, "suburi", NULL, NULL);
    /* A subtitle file next to the video with the same name: use it. */
    if (path) {
        char *dot = strrchr(path, '.');
        if (dot) {
            static const char *subs[] = {".srt", ".vtt", ".ass", ".ssa", NULL};
            for (int i = 0; subs[i]; i++) {
                char *cand = g_strdup_printf("%.*s%s", (int)(dot - path), path, subs[i]);
                if (g_file_test(cand, G_FILE_TEST_IS_REGULAR)) {
                    char *suburi = g_filename_to_uri(cand, NULL, NULL);
                    g_object_set(app.playbin, "suburi", suburi, NULL);
                    g_free(suburi);
                    g_free(cand);
                    break;
                }
                g_free(cand);
            }
        }
    }
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "player");
    gst_element_set_state(app.playbin, GST_STATE_PAUSED);
    gst_element_get_state(app.playbin, NULL, NULL, 5 * GST_SECOND);
    gint64 resume = app.positions ? g_key_file_get_int64(app.positions, "positions", uri, NULL) : 0;
    if (resume > 0)
        seek_to(resume);
    set_playing(TRUE);
    show_controls();
    g_free(base);
    g_free(path);
}

static void play_file(const char *path)
{
    char *abs = g_canonicalize_filename(path, NULL);
    char *uri = g_filename_to_uri(abs, NULL, NULL);
    if (uri)
        play_uri(uri);
    g_free(uri);
    g_free(abs);
}

/* ---- library -------------------------------------------------------- */

static void on_row_activated(GtkListBox *box, GtkListBoxRow *row, gpointer d)
{
    (void)box; (void)d;
    const char *path = g_object_get_data(G_OBJECT(row), "path");
    if (path)
        play_file(path);
}

static gint by_name(gconstpointer a, gconstpointer b)
{
    return g_utf8_collate(*(const char *const *)a, *(const char *const *)b);
}

static void scan(const char *dir, GPtrArray *out, int depth)
{
    GDir *d = g_dir_open(dir, 0, NULL);
    if (!d)
        return;
    const char *name;
    while ((name = g_dir_read_name(d)) && out->len < 2000) {
        if (name[0] == '.')
            continue;
        char *p = g_build_filename(dir, name, NULL);
        if (depth < 4 && g_file_test(p, G_FILE_TEST_IS_DIR) && !g_file_test(p, G_FILE_TEST_IS_SYMLINK))
            scan(p, out, depth + 1);
        else if (is_video_name(name))
            g_ptr_array_add(out, g_strdup(p));
        g_free(p);
    }
    g_dir_close(d);
}

static void fill_library(void)
{
    GList *rows = gtk_container_get_children(GTK_CONTAINER(app.library_list));
    for (GList *l = rows; l; l = l->next)
        gtk_widget_destroy(GTK_WIDGET(l->data));
    g_list_free(rows);

    const char *videos = g_get_user_special_dir(G_USER_DIRECTORY_VIDEOS);
    char *fallback = g_build_filename(g_get_home_dir(), "Videos", NULL);
    GPtrArray *found = g_ptr_array_new_with_free_func(g_free);
    scan(videos ? videos : fallback, found, 0);
    char *downloads = g_build_filename(g_get_home_dir(), "Downloads", NULL);
    scan(downloads, found, 3);
    g_ptr_array_sort(found, by_name);

    for (guint i = 0; i < found->len; i++) {
        const char *p = g_ptr_array_index(found, i);
        GtkWidget *row = gtk_list_box_row_new();
        GtkWidget *box = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
        gtk_widget_set_margin_start(box, 14);
        gtk_widget_set_margin_end(box, 14);
        gtk_widget_set_margin_top(box, 10);
        gtk_widget_set_margin_bottom(box, 10);
        GtkWidget *glyph = gtk_label_new("⬢");
        gtk_style_context_add_class(gtk_widget_get_style_context(glyph), "glyph");
        char *base = g_path_get_basename(p);
        GtkWidget *name = gtk_label_new(base);
        gtk_label_set_xalign(GTK_LABEL(name), 0);
        gtk_label_set_ellipsize(GTK_LABEL(name), PANGO_ELLIPSIZE_MIDDLE);
        gtk_style_context_add_class(gtk_widget_get_style_context(name), "name");
        char *dir = g_path_get_dirname(p);
        const char *home = g_get_home_dir();
        char *where = g_str_has_prefix(dir, home) ? g_strconcat("~", dir + strlen(home), NULL) : g_strdup(dir);
        GtkWidget *loc = gtk_label_new(where);
        gtk_style_context_add_class(gtk_widget_get_style_context(loc), "dim");
        gboolean resumable = app.positions && g_key_file_has_key(app.positions, "positions", g_filename_to_uri(p, NULL, NULL), NULL);
        gtk_box_pack_start(GTK_BOX(box), glyph, FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(box), name, TRUE, TRUE, 0);
        if (resumable) {
            GtkWidget *pill = gtk_label_new("Resume");
            gtk_style_context_add_class(gtk_widget_get_style_context(pill), "pill");
            gtk_box_pack_start(GTK_BOX(box), pill, FALSE, FALSE, 0);
        }
        gtk_box_pack_start(GTK_BOX(box), loc, FALSE, FALSE, 0);
        gtk_container_add(GTK_CONTAINER(row), box);
        g_object_set_data_full(G_OBJECT(row), "path", g_strdup(p), g_free);
        gtk_container_add(GTK_CONTAINER(app.library_list), row);
        g_free(base);
        g_free(dir);
        g_free(where);
    }
    gtk_widget_set_visible(app.library_empty, found->len == 0);
    gtk_widget_show_all(app.library_list);
    g_ptr_array_free(found, TRUE);
    g_free(fallback);
    g_free(downloads);
}

static void show_library(void)
{
    stop_playback();
    if (app.fullscreen)
        gtk_window_unfullscreen(GTK_WINDOW(app.win));
    gtk_window_set_title(GTK_WINDOW(app.win), "Video Player");
    fill_library();
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "library");
}

static void on_back_clicked(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    show_library();
}

static void open_dialog(void)
{
    GtkFileChooserNative *fc = gtk_file_chooser_native_new("Open a video", GTK_WINDOW(app.win),
                                                           GTK_FILE_CHOOSER_ACTION_OPEN, "_Open", "_Cancel");
    GtkFileFilter *f = gtk_file_filter_new();
    gtk_file_filter_set_name(f, "Videos");
    gtk_file_filter_add_mime_type(f, "video/*");
    gtk_file_chooser_add_filter(GTK_FILE_CHOOSER(fc), f);
    const char *videos = g_get_user_special_dir(G_USER_DIRECTORY_VIDEOS);
    if (videos)
        gtk_file_chooser_set_current_folder(GTK_FILE_CHOOSER(fc), videos);
    if (gtk_native_dialog_run(GTK_NATIVE_DIALOG(fc)) == GTK_RESPONSE_ACCEPT) {
        char *path = gtk_file_chooser_get_filename(GTK_FILE_CHOOSER(fc));
        if (path)
            play_file(path);
        g_free(path);
    }
    g_object_unref(fc);
}

static void on_open_clicked(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    open_dialog();
}

static void on_drop(GtkWidget *w, GdkDragContext *ctx, int x, int y, GtkSelectionData *sel, guint info, guint time, gpointer d)
{
    (void)w; (void)x; (void)y; (void)info; (void)d;
    char **uris = gtk_selection_data_get_uris(sel);
    if (uris && uris[0])
        play_uri(uris[0]);
    g_strfreev(uris);
    gtk_drag_finish(ctx, TRUE, FALSE, time);
}

/* ---- input ---------------------------------------------------------- */

static gboolean on_key(GtkWidget *w, GdkEventKey *e, gpointer d)
{
    (void)w; (void)d;
    gboolean in_player = g_strcmp0(gtk_stack_get_visible_child_name(GTK_STACK(app.stack)), "player") == 0;
    if (e->keyval == GDK_KEY_o && (e->state & GDK_CONTROL_MASK)) {
        open_dialog();
        return TRUE;
    }
    if (!in_player)
        return FALSE;
    double vol = gtk_scale_button_get_value(GTK_SCALE_BUTTON(app.volume));
    switch (e->keyval) {
    case GDK_KEY_space: case GDK_KEY_k: set_playing(!app.playing); break;
    case GDK_KEY_Left: seek_by(-10 * GST_SECOND); break;
    case GDK_KEY_Right: seek_by(10 * GST_SECOND); break;
    case GDK_KEY_Up: gtk_scale_button_set_value(GTK_SCALE_BUTTON(app.volume), MIN(1.0, vol + 0.05)); break;
    case GDK_KEY_Down: gtk_scale_button_set_value(GTK_SCALE_BUTTON(app.volume), MAX(0.0, vol - 0.05)); break;
    case GDK_KEY_m: { gboolean mute; g_object_get(app.playbin, "mute", &mute, NULL); g_object_set(app.playbin, "mute", !mute, NULL); break; }
    case GDK_KEY_f: case GDK_KEY_F11: toggle_fullscreen(); break;
    case GDK_KEY_bracketleft: set_rate(app.rate - 0.25); break;
    case GDK_KEY_bracketright: set_rate(app.rate + 0.25); break;
    case GDK_KEY_o: open_dialog(); break;
    case GDK_KEY_Escape: if (app.fullscreen) toggle_fullscreen(); else show_library(); break;
    default: return FALSE;
    }
    show_controls();
    return TRUE;
}

static gboolean on_video_click(GtkWidget *w, GdkEventButton *e, gpointer d)
{
    (void)w; (void)d;
    if (e->type == GDK_2BUTTON_PRESS)
        toggle_fullscreen();
    else if (e->type == GDK_BUTTON_PRESS && e->button == 1)
        set_playing(!app.playing);
    show_controls();
    return TRUE;
}

static gboolean on_motion(GtkWidget *w, GdkEventMotion *e, gpointer d)
{
    (void)w; (void)e; (void)d;
    show_controls();
    return FALSE;
}

/* ---- look ----------------------------------------------------------- */

static const char *CSS =
    /* Fallback: NexaOS "Horizon". The person's theme (~/.config/gtk-3.0/gtk.css)
     * overrides these, so the app follows the theme they picked. */
    "@define-color nexa_bg #0c1228; @define-color nexa_bg_deep #080c1e;"
    "@define-color nexa_surface #101732; @define-color nexa_surface_hi #222a46;"
    "@define-color nexa_hover #333a54; @define-color nexa_border #363d5a;"
    "@define-color nexa_text #d9e0f6; @define-color nexa_text_strong #e6ecff;"
    "@define-color nexa_muted #8f9bc4; @define-color nexa_accent #4da3ff;"
    "@define-color nexa_accent_hover #6db4ff; @define-color nexa_on_accent #080c1e;"
    "@define-color nexa_selected #264779;"
    "* { font-family: Inter, sans-serif; }"
    "window { background-color: @nexa_bg_deep; color: @nexa_text; }"
    ".name { color: @nexa_text_strong; font-size: 11pt; }"
    ".library { background-color: @nexa_bg_deep; }"
    ".library list { background-color: transparent; }"
    ".library row { border-radius: 10px; margin: 2px 12px; }"
    ".library row:hover { background-color: @nexa_hover; }"
    ".library row:selected { background-color: @nexa_selected; }"
    ".heading { font-size: 15pt; font-weight: 700; color: @nexa_text_strong; }"
    ".sub { color: @nexa_muted; }"
    ".glyph { color: @nexa_accent; font-size: 12pt; }"
    ".dim { color: @nexa_muted; font-size: 8.5pt; }"
    ".pill { color: @nexa_accent; border: 1px solid alpha(@nexa_accent, 0.5); border-radius: 10px; padding: 0 8px; font-size: 8.5pt; }"
    ".controls { background-color: alpha(@nexa_bg_deep, 0.86); border-top: 1px solid @nexa_border; padding: 4px 12px; }"
    ".controls button, .primary { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text_strong;"
    "  border: 1px solid @nexa_border; border-radius: 10px; padding: 4px 12px; box-shadow: none; text-shadow: none; }"
    ".controls button:hover { background-color: @nexa_hover; }"
    ".primary { background-color: @nexa_accent; color: @nexa_on_accent; font-weight: 700; border-color: @nexa_accent; }"
    ".primary:hover { background-color: @nexa_accent_hover; }"
    ".play { min-width: 44px; font-size: 12pt; }"
    ".title { font-weight: 600; color: @nexa_text_strong; }"
    ".time { font-family: \"JetBrains Mono\", monospace; color: @nexa_text; }"
    "scale trough { background-color: alpha(@nexa_text, 0.14); border-radius: 3px; min-height: 5px; }"
    "scale highlight { background-color: @nexa_accent; border-radius: 3px; }"
    "scale slider { background-color: @nexa_text_strong; border-radius: 50%; min-width: 14px; min-height: 14px; border: none; box-shadow: none; }";

/* ---- build the window ----------------------------------------------- */

static GtkWidget *button(const char *label, const char *tip, GCallback cb, const char *cls)
{
    GtkWidget *b = gtk_button_new_with_label(label);
    gtk_widget_set_tooltip_text(b, tip);
    gtk_widget_set_can_focus(b, FALSE);
    if (cls)
        gtk_style_context_add_class(gtk_widget_get_style_context(b), cls);
    g_signal_connect(b, "clicked", cb, NULL);
    return b;
}

static GtkWidget *build_library(void)
{
    GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    gtk_style_context_add_class(gtk_widget_get_style_context(box), "library");
    GtkWidget *head = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
    gtk_widget_set_margin_start(head, 24);
    gtk_widget_set_margin_end(head, 24);
    gtk_widget_set_margin_top(head, 22);
    GtkWidget *titles = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    GtkWidget *h = gtk_label_new("Videos");
    gtk_style_context_add_class(gtk_widget_get_style_context(h), "heading");
    gtk_label_set_xalign(GTK_LABEL(h), 0);
    GtkWidget *s = gtk_label_new("From ~/Videos and ~/Downloads. Drop a file here to play it.");
    gtk_style_context_add_class(gtk_widget_get_style_context(s), "sub");
    gtk_label_set_xalign(GTK_LABEL(s), 0);
    gtk_box_pack_start(GTK_BOX(titles), h, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(titles), s, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(head), titles, TRUE, TRUE, 0);
    GtkWidget *open = button("Open…", "Open a video (Ctrl+O)", G_CALLBACK(on_open_clicked), "primary");
    gtk_widget_set_valign(open, GTK_ALIGN_CENTER);
    gtk_box_pack_start(GTK_BOX(head), open, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(box), head, FALSE, FALSE, 8);

    app.library_empty = gtk_label_new("No videos yet. Put some in ~/Videos, or press Open.");
    gtk_style_context_add_class(gtk_widget_get_style_context(app.library_empty), "sub");
    gtk_widget_set_no_show_all(app.library_empty, TRUE);
    gtk_widget_set_margin_top(app.library_empty, 60);
    gtk_box_pack_start(GTK_BOX(box), app.library_empty, FALSE, FALSE, 0);

    GtkWidget *scroller = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(scroller), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    app.library_list = gtk_list_box_new();
    gtk_list_box_set_activate_on_single_click(GTK_LIST_BOX(app.library_list), TRUE);
    g_signal_connect(app.library_list, "row-activated", G_CALLBACK(on_row_activated), NULL);
    gtk_container_add(GTK_CONTAINER(scroller), app.library_list);
    gtk_box_pack_start(GTK_BOX(box), scroller, TRUE, TRUE, 0);
    return box;
}

static GtkWidget *build_player(void)
{
    GtkWidget *overlay = gtk_overlay_new();
    app.video_area = gtk_drawing_area_new();
    gtk_widget_add_events(app.video_area, GDK_BUTTON_PRESS_MASK | GDK_POINTER_MOTION_MASK);
    g_signal_connect(app.video_area, "draw", G_CALLBACK(on_draw_video), NULL);
    g_signal_connect(app.video_area, "button-press-event", G_CALLBACK(on_video_click), NULL);
    g_signal_connect(app.video_area, "motion-notify-event", G_CALLBACK(on_motion), NULL);
    gtk_container_add(GTK_CONTAINER(overlay), app.video_area);

    app.controls = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_style_context_add_class(gtk_widget_get_style_context(app.controls), "controls");
    gtk_box_pack_start(GTK_BOX(app.controls), button("‹", "Back to the library (Esc)", G_CALLBACK(on_back_clicked), NULL), FALSE, FALSE, 0);
    app.play_btn = button("▶", "Play (Space)", G_CALLBACK(on_play_clicked), "play");
    gtk_box_pack_start(GTK_BOX(app.controls), app.play_btn, FALSE, FALSE, 0);
    app.time_label = gtk_label_new("0:00 / 0:00");
    gtk_style_context_add_class(gtk_widget_get_style_context(app.time_label), "time");
    gtk_box_pack_start(GTK_BOX(app.controls), app.time_label, FALSE, FALSE, 4);
    app.seek = gtk_scale_new_with_range(GTK_ORIENTATION_HORIZONTAL, 0, 1, 1);
    gtk_scale_set_draw_value(GTK_SCALE(app.seek), FALSE);
    gtk_widget_set_can_focus(app.seek, FALSE);
    g_signal_connect(app.seek, "button-press-event", G_CALLBACK(on_seek_press), NULL);
    g_signal_connect(app.seek, "button-release-event", G_CALLBACK(on_seek_release), NULL);
    gtk_box_pack_start(GTK_BOX(app.controls), app.seek, TRUE, TRUE, 0);
    app.title_label = gtk_label_new("");
    gtk_style_context_add_class(gtk_widget_get_style_context(app.title_label), "title");
    gtk_label_set_ellipsize(GTK_LABEL(app.title_label), PANGO_ELLIPSIZE_MIDDLE);
    gtk_label_set_max_width_chars(GTK_LABEL(app.title_label), 28);
    gtk_box_pack_start(GTK_BOX(app.controls), app.title_label, FALSE, FALSE, 6);
    app.speed_btn = button("1x", "Speed ([ and ])", G_CALLBACK(on_speed_clicked), NULL);
    gtk_box_pack_start(GTK_BOX(app.controls), app.speed_btn, FALSE, FALSE, 0);
    app.volume = gtk_volume_button_new();
    gtk_scale_button_set_value(GTK_SCALE_BUTTON(app.volume), 0.85);
    gtk_widget_set_can_focus(app.volume, FALSE);
    g_signal_connect(app.volume, "value-changed", G_CALLBACK(on_volume_changed), NULL);
    gtk_box_pack_start(GTK_BOX(app.controls), app.volume, FALSE, FALSE, 0);
    app.full_btn = button("⤢", "Fullscreen (F)", G_CALLBACK(on_full_clicked), NULL);
    gtk_box_pack_start(GTK_BOX(app.controls), app.full_btn, FALSE, FALSE, 0);

    app.revealer = gtk_revealer_new();
    gtk_revealer_set_transition_type(GTK_REVEALER(app.revealer), GTK_REVEALER_TRANSITION_TYPE_CROSSFADE);
    gtk_revealer_set_transition_duration(GTK_REVEALER(app.revealer), 240);
    gtk_container_add(GTK_CONTAINER(app.revealer), app.controls);
    gtk_widget_set_valign(app.revealer, GTK_ALIGN_END);
    gtk_overlay_add_overlay(GTK_OVERLAY(overlay), app.revealer);
    return overlay;
}

static gboolean on_delete(GtkWidget *w, GdkEvent *e, gpointer d)
{
    (void)w; (void)e; (void)d;
    stop_playback();
    return FALSE;
}

int main(int argc, char **argv)
{
    gst_init(&argc, &argv);
    gtk_init(&argc, &argv);
    g_set_prgname(APP_ID);
    g_set_application_name("Video Player");
    g_mutex_init(&app.frame_lock);
    app.rate = 1.0;

    app.positions = g_key_file_new();
    char *pos_path = data_path(RESUME_FILE);
    g_key_file_load_from_file(app.positions, pos_path, G_KEY_FILE_NONE, NULL);
    g_free(pos_path);

    app.playbin = gst_element_factory_make("playbin", "player");
    GstElement *sink = gst_element_factory_make("appsink", "frames");
    if (!app.playbin || !sink) {
        g_printerr("video-player: GStreamer's playbin/appsink are missing (gst-plugins-base)\n");
        return 1;
    }
    GstCaps *caps = gst_caps_from_string("video/x-raw,format=BGRx");
    g_object_set(sink, "caps", caps, "max-buffers", 2, "drop", TRUE, "sync", TRUE, NULL);
    gst_caps_unref(caps);
    GstAppSinkCallbacks cbs = {0};
    cbs.new_sample = on_new_sample;
    gst_app_sink_set_callbacks(GST_APP_SINK(sink), &cbs, NULL, NULL);
    g_object_set(app.playbin, "video-sink", sink, "volume", 0.85 * 0.85, NULL);
    GstBus *bus = gst_element_get_bus(app.playbin);
    gst_bus_add_watch(bus, on_bus, NULL);
    gst_object_unref(bus);

    GtkCssProvider *css = gtk_css_provider_new();
    gtk_css_provider_load_from_data(css, CSS, -1, NULL);
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(css),
                                              GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);

    app.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(app.win), "Video Player");
    gtk_window_set_default_size(GTK_WINDOW(app.win), 960, 600);
    gtk_window_set_icon_name(GTK_WINDOW(app.win), APP_ID);
    app.stack = gtk_stack_new();
    gtk_stack_set_transition_type(GTK_STACK(app.stack), GTK_STACK_TRANSITION_TYPE_CROSSFADE);
    gtk_stack_add_named(GTK_STACK(app.stack), build_library(), "library");
    gtk_stack_add_named(GTK_STACK(app.stack), build_player(), "player");
    gtk_container_add(GTK_CONTAINER(app.win), app.stack);

    gtk_drag_dest_set(app.win, GTK_DEST_DEFAULT_ALL, NULL, 0, GDK_ACTION_COPY);
    gtk_drag_dest_add_uri_targets(app.win);
    g_signal_connect(app.win, "drag-data-received", G_CALLBACK(on_drop), NULL);
    g_signal_connect(app.win, "key-press-event", G_CALLBACK(on_key), NULL);
    g_signal_connect(app.win, "window-state-event", G_CALLBACK(on_window_state), NULL);
    g_signal_connect(app.win, "delete-event", G_CALLBACK(on_delete), NULL);
    g_signal_connect(app.win, "destroy", G_CALLBACK(gtk_main_quit), NULL);

    app.tick_id = g_timeout_add(250, on_tick, NULL);
    gtk_widget_show_all(app.win);
    fill_library();
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "library");
    if (argc > 1) {
        if (strstr(argv[1], "://"))
            play_uri(argv[1]);
        else
            play_file(argv[1]);
    }
    gtk_main();

    gst_element_set_state(app.playbin, GST_STATE_NULL);
    gst_object_unref(app.playbin);
    g_key_file_free(app.positions);
    return 0;
}
