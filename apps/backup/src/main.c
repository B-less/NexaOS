/* Backup: a NexaOS app in C (GTK 3). Copies your home folder to a USB
 * drive, only what changed each time, and puts any of it back.
 *
 *   backup
 *
 * Back Up: pick a drive (USB sticks and drives under /media; one not open
 * yet is opened through the system broker), see when you last backed up,
 * Back Up Now. What to Back Up: your home's folders, each with a switch.
 * Put Back: a backup from the drive, its folders and files, Put Back
 * where they were (a file changed since stays, the old one beside it) or
 * into a folder; Check (reads every copy again); Delete.
 *
 * Installed, it runs confined by nexa-run to files.home and files.drives.
 * The engine (engine.c) is plain C with no GTK, unit-tested.
 *
 * BACKUP_TEST=<dir>: for tests only, writes <dir>/state.json and takes
 * commands from the pipe <dir>/cmd (tests/headless.py). */
#include <fcntl.h>
#include <glib/gstdio.h>
#include <gtk/gtk.h>
#include <math.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include "engine.h"

#define APP_ID "backup"
#define STALE_DAYS 7

typedef struct {
    char *path, *label;
    gboolean ro, mounted;
    char *broker_name;  /* not open yet: the broker's name for it */
} Drive;

typedef struct {
    GtkWidget *win, *stack, *nav[3];
    char *home, *conf;
    /* drives */
    GPtrArray *drives;  /* Drive* */
    int drive;          /* chosen, -1 none */
    char *want_drive;   /* remembered by path */
    GtkWidget *drive_list, *drive_empty;
    char *drives_key;
    /* status */
    GtkWidget *badge, *status_head, *status_sub, *backup_btn, *result, *problems;
    /* progress */
    GtkWidget *prog_box, *prog_bar, *prog_file, *prog_counts;
    gboolean busy;
    gint cancel;
    gint64 started;
    /* what to back up */
    GHashTable *off;    /* top-level names switched off */
    gboolean hidden_on;
    GtkWidget *choice_list;
    /* put back */
    GPtrArray *snaps;
    Snap *snap;
    char *folder;       /* the folder shown, inside the backup */
    GHashTable *ticked; /* paths */
    GtkWidget *snap_list, *snap_empty, *browse_box, *browse_list, *crumb, *put_btn, *put_where, *put_msg, *search;
    char *test_dir;
    guint state_id;
} App;

static App app;

static void state_changed(void);
static void fill_drives(void);
static void fill_status(void);
static void fill_snaps(void);
static void fill_browse(void);
static void say(GtkWidget *l, const char *text, gboolean bad);

/* ---- helpers ---- */

static void add_class(GtkWidget *w, const char *c) { gtk_style_context_add_class(gtk_widget_get_style_context(w), c); }

static void destroy_child(GtkWidget *w, gpointer d)
{
    (void)d;
    gtk_widget_destroy(w);
}

static GtkWidget *label(const char *text, const char *cls, float x)
{
    GtkWidget *l = gtk_label_new(text);
    if (cls) add_class(l, cls);
    gtk_label_set_xalign(GTK_LABEL(l), x);
    gtk_label_set_line_wrap(GTK_LABEL(l), TRUE);
    gtk_label_set_line_wrap_mode(GTK_LABEL(l), PANGO_WRAP_WORD_CHAR);
    return l;
}

static GtkWidget *button(const char *text, const char *tip, GCallback cb, gpointer data, const char *cls)
{
    GtkWidget *b = gtk_button_new_with_label(text);
    if (tip) gtk_widget_set_tooltip_text(b, tip);
    if (cls) add_class(b, cls);
    if (cb) g_signal_connect(b, "clicked", cb, data);
    return b;
}

static void keep_hidden(GtkWidget *w)
{
    gtk_widget_show_all(w);
    gtk_widget_hide(w);
    gtk_widget_set_no_show_all(w, TRUE);
}

static char *size_text(gint64 b) { return g_format_size((guint64)MAX(0, b)); }

static char *count_text(gint64 n, const char *one, const char *many)
{
    char num[32];
    g_snprintf(num, sizeof num, "%" G_GINT64_FORMAT, n);
    GString *s = g_string_new(NULL);
    int len = (int)strlen(num);
    for (int i = 0; i < len; i++) {
        if (i && (len - i) % 3 == 0) g_string_append_c(s, ',');
        g_string_append_c(s, num[i]);
    }
    g_string_append_printf(s, " %s", n == 1 ? one : many);
    return g_string_free(s, FALSE);
}

static char *when_text(gint64 when)
{
    GDateTime *d = g_date_time_new_from_unix_local(when), *now = g_date_time_new_now_local();
    if (!d) return g_strdup("");
    int days = (int)((g_date_time_to_unix(now) - when) / 86400);
    char *clock = g_date_time_format(d, "%H:%M"), *out;
    if (g_date_time_get_day_of_year(d) == g_date_time_get_day_of_year(now) && g_date_time_get_year(d) == g_date_time_get_year(now))
        out = g_strdup_printf("today, %s", clock);
    else if (days < 2)
        out = g_strdup_printf("yesterday, %s", clock);
    else if (days < 7)
        out = g_date_time_format(d, "%A, %H:%M");
    else
        out = g_date_time_format(d, "%e %B %Y, %H:%M");
    g_free(clock);
    g_date_time_unref(d);
    g_date_time_unref(now);
    return g_strstrip(out);
}

static Drive *current_drive(void)
{
    return app.drives && app.drive >= 0 && app.drive < (int)app.drives->len ? app.drives->pdata[app.drive] : NULL;
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

static GdkRGBA colour(GtkWidget *w, const char *name, const char *fallback)
{
    GdkRGBA c;
    if (!gtk_style_context_lookup_color(gtk_widget_get_style_context(w), name, &c)) gdk_rgba_parse(&c, fallback);
    return c;
}

/* ---- settings (~/.config/backup/settings.ini) ---- */

static void load_conf(void)
{
    GKeyFile *k = g_key_file_new();
    app.off = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    app.hidden_on = TRUE;
    if (g_key_file_load_from_file(k, app.conf, G_KEY_FILE_NONE, NULL)) {
        char **off = g_key_file_get_string_list(k, "choice", "off", NULL, NULL);
        for (int i = 0; off && off[i]; i++)
            if (*off[i] && !strchr(off[i], '/')) g_hash_table_add(app.off, g_strdup(off[i]));
        g_strfreev(off);
        GError *e = NULL;
        gboolean h = g_key_file_get_boolean(k, "choice", "hidden", &e);
        if (!e) app.hidden_on = h;
        g_clear_error(&e);
        app.want_drive = g_key_file_get_string(k, "drive", "path", NULL);
    }
    g_key_file_free(k);
}

static void save_conf(void)
{
    GKeyFile *k = g_key_file_new();
    GPtrArray *off = g_ptr_array_new();
    GHashTableIter it;
    gpointer key;
    g_hash_table_iter_init(&it, app.off);
    while (g_hash_table_iter_next(&it, &key, NULL)) g_ptr_array_add(off, key);
    g_ptr_array_sort(off, (GCompareFunc)strcmp);
    g_key_file_set_string_list(k, "choice", "off", (const char *const *)off->pdata, off->len);
    g_ptr_array_free(off, TRUE);
    g_key_file_set_boolean(k, "choice", "hidden", app.hidden_on);
    Drive *d = current_drive();
    if (d && d->mounted) g_key_file_set_string(k, "drive", "path", d->path);
    char *dir = g_path_get_dirname(app.conf);
    g_mkdir_with_parents(dir, 0700);
    g_free(dir);
    g_key_file_save_to_file(k, app.conf, NULL);
    g_key_file_free(k);
}

/* What the engine leaves out: the switched-off names, and hidden ones
 * when "Settings and app data" is off. */
static Choice make_choice(void)
{
    Choice c = {g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL)};
    GHashTableIter it;
    gpointer key;
    g_hash_table_iter_init(&it, app.off);
    while (g_hash_table_iter_next(&it, &key, NULL)) g_hash_table_add(c.off, g_strdup(key));
    if (!app.hidden_on) {
        GDir *d = g_dir_open(app.home, 0, NULL);
        const char *n;
        while (d && (n = g_dir_read_name(d)))
            if (n[0] == '.') g_hash_table_add(c.off, g_strdup(n));
        if (d) g_dir_close(d);
    }
    return c;
}

/* ---- drives ---- */

static void drive_free(Drive *d)
{
    g_free(d->path);
    g_free(d->label);
    g_free(d->broker_name);
    g_free(d);
}

static char *unoctal(const char *s)
{
    GString *o = g_string_new(NULL);
    for (const char *c = s; *c; c++) {
        if (c[0] == '\\' && c[1] >= '0' && c[1] <= '3' && c[2] >= '0' && c[2] <= '7' && c[3] >= '0' && c[3] <= '7') {
            g_string_append_c(o, (char)((c[1] - '0') * 64 + (c[2] - '0') * 8 + (c[3] - '0')));
            c += 3;
        } else {
            g_string_append_c(o, *c);
        }
    }
    return g_string_free(o, FALSE);
}

static const char *media_dir(void)
{
    const char *m = g_getenv("BACKUP_MEDIA");  /* tests */
    return m && *m ? m : "/media";
}

/* Drives under /media (from /proc/mounts), and ones the broker knows but
 * hasn't opened (/run/nexa/drives.json). */
static GPtrArray *find_drives(void)
{
    GPtrArray *out = g_ptr_array_new_with_free_func((GDestroyNotify)drive_free);
    char *text = NULL;
    const char *media = media_dir();
    size_t ml = strlen(media);
    if (g_file_get_contents("/proc/mounts", &text, NULL, NULL)) {
        char **lines = g_strsplit(text, "\n", -1);
        for (int i = 0; lines[i]; i++) {
            char **f = g_strsplit(lines[i], " ", -1);
            if (g_strv_length(f) >= 4) {
                char *mp = unoctal(f[1]);
                if (!strncmp(mp, media, ml) && mp[ml] == '/' && mp[ml + 1] && !strchr(mp + ml + 1, '/')) {
                    Drive *d = g_new0(Drive, 1);
                    d->path = mp;
                    mp = NULL;
                    d->label = g_path_get_basename(d->path);
                    d->mounted = TRUE;
                    char **opts = g_strsplit(f[3], ",", -1);
                    for (int j = 0; opts[j]; j++) d->ro |= !strcmp(opts[j], "ro");
                    g_strfreev(opts);
                    g_ptr_array_add(out, d);
                }
                g_free(mp);
            }
            g_strfreev(f);
        }
        g_strfreev(lines);
        g_free(text);
    }
    const char *state = g_getenv("BACKUP_DRIVES_JSON");
    if (!state || !*state) state = "/run/nexa/drives.json";
    char *js = NULL;
    if (g_file_get_contents(state, &js, NULL, NULL)) {
        /* {"drives": [{"name": "sdb1", "label": "SANDISK", "mountpoint": null, ...}]}: a few fields, read by hand */
        for (char *p = strstr(js, "\"name\""); p; p = strstr(p + 6, "\"name\"")) {
            char *end = strchr(p, '}');
            if (!end) break;
            char *obj = g_strndup(p, (gsize)(end - p));
            char name[64] = "", lab[128] = "";
            sscanf(obj, "\"name\": \"%63[A-Za-z0-9_-]\"", name);
            char *lp = strstr(obj, "\"label\": \"");
            if (lp) sscanf(lp, "\"label\": \"%127[^\"]\"", lab);
            gboolean open = strstr(obj, "\"mountpoint\": \"") != NULL;
            if (*name && !open) {
                Drive *d = g_new0(Drive, 1);
                d->broker_name = g_strdup(name);
                d->label = g_strdup(*lab ? lab : name);
                g_ptr_array_add(out, d);
            }
            g_free(obj);
        }
        g_free(js);
    }
    return out;
}

/* One JSON line to the broker, one back (a few seconds at most). */
static char *broker(const char *req)
{
    const char *path = g_getenv("NEXA_BROKER_SOCKET");
    if (!path || !*path) path = "/run/nexa-broker.sock";
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    if (strlen(path) >= sizeof a.sun_path) return NULL;
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return NULL;
    memcpy(a.sun_path, path, strlen(path) + 1);
    struct timeval tv = {20, 0};
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
    GString *r = g_string_new(NULL);
    if (connect(fd, (struct sockaddr *)&a, sizeof a) == 0 && write(fd, req, strlen(req)) == (ssize_t)strlen(req)) {
        char buf[2048];
        ssize_t n;
        while (r->len < 65536 && (n = read(fd, buf, sizeof buf)) > 0) {
            g_string_append_len(r, buf, n);
            if (memchr(buf, '\n', (size_t)n)) break;
        }
    }
    close(fd);
    return g_string_free(r, r->len == 0);
}

static void on_open_drive(GtkButton *b, gpointer data)
{
    (void)b;
    Drive *d = data;
    char req[160];
    g_snprintf(req, sizeof req, "{\"action\": \"mount_drive\", \"args\": {\"name\": \"%s\"}}\n", d->broker_name);
    char *r = broker(req);
    if (!r || !strstr(r, "\"ok\": true")) say(app.result, "The drive couldn't be opened. Unplug it, plug it in again, and try once more.", TRUE);
    g_free(r);
    fill_drives();
}

static void on_drive_row(GtkListBox *box, GtkListBoxRow *row, gpointer d)
{
    (void)box; (void)d;
    int i = GPOINTER_TO_INT(g_object_get_data(G_OBJECT(row), "index"));
    Drive *dr = app.drives->pdata[i];
    if (!dr->mounted || app.busy) return;
    app.drive = i;
    save_conf();
    fill_drives();
    fill_status();
    fill_snaps();
}

static void fill_drives(void)
{
    GPtrArray *found = find_drives();
    /* the same choice as before, by path */
    char *chosen = current_drive() && current_drive()->mounted ? g_strdup(current_drive()->path) : g_strdup(app.want_drive);
    if (app.drives) g_ptr_array_free(app.drives, TRUE);
    app.drives = found;
    app.drive = -1;
    for (guint i = 0; i < found->len; i++) {
        Drive *d = found->pdata[i];
        if (d->mounted && chosen && !strcmp(d->path, chosen)) app.drive = (int)i;
    }
    for (guint i = 0; app.drive < 0 && i < found->len; i++)
        if (((Drive *)found->pdata[i])->mounted && !((Drive *)found->pdata[i])->ro) app.drive = (int)i;
    g_free(chosen);
    gtk_container_foreach(GTK_CONTAINER(app.drive_list), destroy_child, NULL);
    for (guint i = 0; i < found->len; i++) {
        Drive *d = found->pdata[i];
        GtkWidget *row = gtk_list_box_row_new(), *box = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
        g_object_set_data(G_OBJECT(row), "index", GINT_TO_POINTER(i));
        add_class(row, "drive");
        if ((int)i == app.drive) add_class(row, "chosen");
        g_object_set(box, "margin", 12, NULL);
        GtkWidget *mark = label((int)i == app.drive ? "⬢" : "⬡", "mark", 0.5);
        gtk_box_pack_start(GTK_BOX(box), mark, FALSE, FALSE, 0);
        GtkWidget *words = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
        gtk_box_pack_start(GTK_BOX(words), label(d->label, "name", 0), FALSE, FALSE, 0);
        char *sub;
        if (!d->mounted) {
            sub = g_strdup("Plugged in, not open yet");
        } else {
            guint64 fr = 0, tot = 0;
            drive_space(d->path, &fr, &tot);
            char *a = size_text((gint64)fr), *b = size_text((gint64)tot);
            GPtrArray *s = snaps_list(d->path);
            char *n = s->len ? count_text(s->len, "backup", "backups") : g_strdup("no backups yet");
            sub = g_strdup_printf("%s free of %s · %s%s", a, b, n, d->ro ? " · read-only" : "");
            g_free(a); g_free(b); g_free(n);
            g_ptr_array_free(s, TRUE);
        }
        gtk_box_pack_start(GTK_BOX(words), label(sub, "sub", 0), FALSE, FALSE, 0);
        g_free(sub);
        gtk_box_pack_start(GTK_BOX(box), words, TRUE, TRUE, 0);
        if (!d->mounted) {
            GtkWidget *b = button("Open", "Open the drive so Backup can use it", G_CALLBACK(on_open_drive), d, NULL);
            gtk_widget_set_valign(b, GTK_ALIGN_CENTER);
            gtk_box_pack_start(GTK_BOX(box), b, FALSE, FALSE, 0);
        }
        gtk_container_add(GTK_CONTAINER(row), box);
        gtk_container_add(GTK_CONTAINER(app.drive_list), row);
    }
    gtk_widget_show_all(app.drive_list);
    gtk_widget_set_visible(app.drive_empty, found->len == 0);
    state_changed();
}

/* Drives come and go: look every 2 s (cheap: two small files). */
static gboolean watch_drives(gpointer d)
{
    (void)d;
    if (app.busy) return G_SOURCE_CONTINUE;
    GPtrArray *now = find_drives();
    GString *key = g_string_new(NULL);
    for (guint i = 0; i < now->len; i++) {
        Drive *x = now->pdata[i];
        g_string_append_printf(key, "%s|%s|%d;", x->path ? x->path : "", x->label, x->mounted);
    }
    g_ptr_array_free(now, TRUE);
    if (g_strcmp0(key->str, app.drives_key)) {
        g_free(app.drives_key);
        app.drives_key = g_strdup(key->str);
        fill_drives();
        fill_status();
        fill_snaps();
    }
    g_string_free(key, TRUE);
    return G_SOURCE_CONTINUE;
}

/* ---- status ---- */

static int status_kind;  /* 0 never, 1 fine, 2 a while ago */
static int runs, put_runs;   /* finished backups / put-backs and checks (for tests) */

static gboolean draw_badge(GtkWidget *w, cairo_t *cr, gpointer d)
{
    (void)d;
    int W = gtk_widget_get_allocated_width(w), H = gtk_widget_get_allocated_height(w);
    double r = MIN(W, H) / 2.0 - 3;
    GdkRGBA ok = colour(w, "nexa_ok", "#5ee6a8"), warn = colour(w, "nexa_warn", "#ffb547"), muted = colour(w, "nexa_muted", "#8f9bc4");
    GdkRGBA c = status_kind == 1 ? ok : status_kind == 2 ? warn : muted;
    hex_path(cr, W / 2.0, H / 2.0, r);
    cairo_set_source_rgba(cr, c.red, c.green, c.blue, 0.16);
    cairo_fill_preserve(cr);
    gdk_cairo_set_source_rgba(cr, &c);
    cairo_set_line_width(cr, 3);
    cairo_stroke(cr);
    cairo_set_line_width(cr, 4);
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
    double cx = W / 2.0, cy = H / 2.0;
    if (status_kind == 1) {  /* a tick */
        cairo_move_to(cr, cx - r * 0.35, cy);
        cairo_line_to(cr, cx - r * 0.08, cy + r * 0.28);
        cairo_line_to(cr, cx + r * 0.4, cy - r * 0.3);
    } else {  /* an arrow into the drive */
        cairo_move_to(cr, cx, cy - r * 0.4);
        cairo_line_to(cr, cx, cy + r * 0.25);
        cairo_move_to(cr, cx - r * 0.25, cy);
        cairo_line_to(cr, cx, cy + r * 0.25);
        cairo_line_to(cr, cx + r * 0.25, cy);
    }
    cairo_stroke(cr);
    return TRUE;
}

static void fill_status(void)
{
    Drive *d = current_drive();
    GPtrArray *s = d && d->mounted ? snaps_list(d->path) : NULL;
    char *head, *sub;
    if (!d) {
        status_kind = 0;
        head = g_strdup("Plug in a USB drive");
        sub = g_strdup("Backup copies your home folder to it, and next time only what changed.");
    } else if (!s || !s->len) {
        status_kind = 0;
        head = g_strdup_printf("No backup on %s yet", d->label);
        sub = g_strdup("The first one copies everything, so it takes longest.");
    } else {
        Snap *last = s->pdata[0];
        gint64 age = g_get_real_time() / G_USEC_PER_SEC - last->when;
        status_kind = age > STALE_DAYS * 86400 ? 2 : 1;
        char *w = when_text(last->when), *f = count_text(last->files, "file", "files"), *b = size_text(last->bytes);
        head = g_strdup_printf("Last backup: %s", w);
        sub = g_strdup_printf("%s, %s, on %s%s", f, b, d->label, status_kind == 2 ? ". It's been a while: back up again soon." : "");
        g_free(w); g_free(f); g_free(b);
    }
    gtk_label_set_text(GTK_LABEL(app.status_head), head);
    gtk_label_set_text(GTK_LABEL(app.status_sub), sub);
    g_free(head);
    g_free(sub);
    if (s) g_ptr_array_free(s, TRUE);
    gtk_widget_set_sensitive(app.backup_btn, d && d->mounted && !d->ro && !app.busy);
    gtk_widget_queue_draw(app.badge);
    state_changed();
}

static void say(GtkWidget *l, const char *text, gboolean bad)
{
    gtk_label_set_text(GTK_LABEL(l), text ? text : "");
    GtkStyleContext *sc = gtk_widget_get_style_context(l);
    if (bad) gtk_style_context_add_class(sc, "bad");
    else gtk_style_context_remove_class(sc, "bad");
    gtk_widget_set_visible(l, text && *text);
    state_changed();
}

/* ---- work off the main thread, progress back on it ---- */

typedef struct {
    gint64 df, tf, db, tb;
    char *path;
} Tick;

static gint64 last_tick;

static gboolean show_tick(gpointer data)
{
    Tick *t = data;
    double k = t->tb > 0 ? (double)t->db / t->tb : t->tf > 0 ? (double)t->df / t->tf : 0;
    gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(app.prog_bar), CLAMP(k, 0, 1));
    gtk_label_set_text(GTK_LABEL(app.prog_file), t->path ? t->path : "");
    double secs = (g_get_monotonic_time() - app.started) / 1e6;
    char *a = count_text(t->df, "file", "files"), *b = size_text(t->db), *c = size_text(t->tb);
    char *rate = secs > 1 ? size_text((gint64)(t->db / secs)) : g_strdup("…");
    char *line = t->tb > 0 ? g_strdup_printf("%s of %" G_GINT64_FORMAT " · %s of %s · %s/s", a, t->tf, b, c, rate)
                           : g_strdup_printf("%s of %" G_GINT64_FORMAT, a, t->tf);
    gtk_label_set_text(GTK_LABEL(app.prog_counts), line);
    g_free(a); g_free(b); g_free(c); g_free(rate); g_free(line);
    g_free(t->path);
    g_free(t);
    state_changed();
    return G_SOURCE_REMOVE;
}

static void on_progress(gpointer d, gint64 df, gint64 tf, gint64 db, gint64 tb, const char *path)
{
    (void)d;
    gint64 now = g_get_monotonic_time();
    if (now - last_tick < 100000 && df < tf) return;  /* ten times a second is plenty */
    last_tick = now;
    Tick *t = g_new0(Tick, 1);
    *t = (Tick){df, tf, db, tb, g_strdup(path)};
    g_idle_add(show_tick, t);
}

static void set_busy(gboolean on, const char *what)
{
    app.busy = on;
    gtk_widget_set_visible(app.prog_box, on);
    if (on) {
        g_atomic_int_set(&app.cancel, 0);
        app.started = g_get_monotonic_time();
        gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(app.prog_bar), 0);
        gtk_label_set_text(GTK_LABEL(app.prog_file), what);
        gtk_label_set_text(GTK_LABEL(app.prog_counts), "");
    }
    for (int i = 0; i < 3; i++) gtk_widget_set_sensitive(app.nav[i], !on);  /* one job at a time */
    gtk_widget_set_sensitive(app.drive_list, !on);
    gtk_widget_set_sensitive(app.choice_list, !on);
    gtk_widget_set_sensitive(app.put_btn, !on);
    fill_status();
}

static void on_stop(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    g_atomic_int_set(&app.cancel, 1);
    gtk_label_set_text(GTK_LABEL(app.prog_file), "Stopping…");
}

/* ---- backing up ---- */

typedef struct {
    char *drive;
    Choice c;
    gint64 need, files;
    guint64 free_b;
    gboolean ok, checked;
    Result r;
    GError *err;
} BackJob;

static gboolean backup_done(gpointer data)
{
    BackJob *j = data;
    runs++;
    set_busy(FALSE, NULL);
    gtk_container_foreach(GTK_CONTAINER(app.problems), destroy_child, NULL);
    if (j->ok) {
        char *f = count_text(j->r.copied, "new or changed file", "new or changed files"), *b = size_text(j->r.copied_bytes);
        char *msg = j->r.copied ? g_strdup_printf("Backed up. Copied %s (%s).", f, b)
                                : g_strdup("Backed up. Nothing had changed, so nothing needed copying.");
        if (j->r.skipped) {
            char *m2 = g_strdup_printf("%s %" G_GINT64_FORMAT " couldn't be read and weren't backed up:", msg, j->r.skipped);
            g_free(msg);
            msg = m2;
            for (guint i = 0; i < j->r.problems->len && i < 20; i++)
                gtk_box_pack_start(GTK_BOX(app.problems), label(j->r.problems->pdata[i], "sub", 0), FALSE, FALSE, 0);
        }
        say(app.result, msg, j->r.skipped > 0);
        g_free(msg); g_free(f); g_free(b);
    } else if (j->checked && j->err && g_error_matches(j->err, g_quark_from_static_string("backup-space"), 1)) {
        say(app.result, j->err->message, TRUE);
    } else {
        char *msg = g_strdup_printf("%s The backups made before are all still there.", j->err ? j->err->message : "The backup stopped.");
        if (j->err && strstr(j->err->message, "Stopped")) {
            g_free(msg);
            msg = g_strdup(j->err->message);
        }
        say(app.result, msg, TRUE);
        g_free(msg);
    }
    gtk_widget_show_all(app.problems);
    result_clear(&j->r);
    g_clear_error(&j->err);
    g_hash_table_destroy(j->c.off);
    g_free(j->drive);
    g_free(j);
    fill_drives();
    fill_status();
    fill_snaps();
    return G_SOURCE_REMOVE;
}

static gpointer backup_thread(gpointer data)
{
    BackJob *j = data;
    j->need = backup_estimate(app.home, j->drive, &j->c, &j->files, NULL);
    drive_space(j->drive, &j->free_b, NULL);
    j->checked = TRUE;
    if (j->need > 0 && (guint64)j->need + (guint64)j->need / 50 > j->free_b) {
        char *n = size_text(j->need), *f = size_text((gint64)j->free_b);
        g_set_error(&j->err, g_quark_from_static_string("backup-space"), 1,
                    "There isn't room on the drive: this backup needs about %s and %s is free. Free some space on the drive, "
                    "back up fewer folders, or use a bigger drive.", n, f);
        g_free(n);
        g_free(f);
        g_idle_add(backup_done, j);
        return NULL;
    }
    Job job = {&app.cancel, on_progress, NULL};
    j->ok = backup_run(app.home, j->drive, &j->c, g_get_real_time() / G_USEC_PER_SEC, &job, &j->r, &j->err);
    g_idle_add(backup_done, j);
    return NULL;
}

static void start_backup(void)
{
    Drive *d = current_drive();
    if (!d || !d->mounted || d->ro || app.busy) return;
    BackJob *j = g_new0(BackJob, 1);
    j->drive = g_strdup(d->path);
    j->c = make_choice();
    say(app.result, "", FALSE);
    set_busy(TRUE, "Looking at what changed…");
    g_thread_unref(g_thread_new("backup", backup_thread, j));
}

static void on_backup(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    start_backup();
}

/* ---- what to back up ---- */

static void on_choice_toggled(GtkSwitch *sw, GParamSpec *ps, gpointer d)
{
    (void)ps; (void)d;
    const char *name = g_object_get_data(G_OBJECT(sw), "name");
    gboolean on = gtk_switch_get_active(sw);
    if (!name) app.hidden_on = on;
    else if (on) g_hash_table_remove(app.off, name);
    else g_hash_table_add(app.off, g_strdup(name));
    save_conf();
    state_changed();
}

typedef struct {
    char *name;
    GtkWidget *label;
    gint64 bytes;
} SizeJob;

static gint64 tree_size(const char *path, int depth)
{
    struct stat st;
    if (lstat(path, &st) != 0) return 0;
    if (!S_ISDIR(st.st_mode) || depth > 40) return S_ISREG(st.st_mode) ? st.st_size : 0;
    gint64 n = 0;
    GDir *d = g_dir_open(path, 0, NULL);
    const char *c;
    while (d && (c = g_dir_read_name(d))) {
        char *p = g_build_filename(path, c, NULL);
        n += tree_size(p, depth + 1);
        g_free(p);
    }
    if (d) g_dir_close(d);
    return n;
}

static gboolean size_done(gpointer data)
{
    SizeJob *j = data;
    char *s = size_text(j->bytes);
    if (GTK_IS_LABEL(j->label)) gtk_label_set_text(GTK_LABEL(j->label), s);
    g_object_unref(j->label);
    g_free(s);
    g_free(j->name);
    g_free(j);
    return G_SOURCE_REMOVE;
}

static void size_work(gpointer data, gpointer user)
{
    (void)user;
    SizeJob *j = data;
    char *p = g_build_filename(app.home, j->name, NULL);
    j->bytes = tree_size(p, 0);
    g_free(p);
    g_idle_add(size_done, j);
}

static GThreadPool *size_pool;

static void choice_row(const char *name, const char *title, gboolean on, gboolean folder)
{
    GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
    add_class(row, "choice");
    GtkWidget *words = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    gtk_box_pack_start(GTK_BOX(words), label(title, "name", 0), FALSE, FALSE, 0);
    GtkWidget *sz = label(folder ? "Measuring…" : "", "sub", 0);
    gtk_box_pack_start(GTK_BOX(words), sz, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(row), words, TRUE, TRUE, 0);
    GtkWidget *sw = gtk_switch_new();
    gtk_switch_set_active(GTK_SWITCH(sw), on);
    gtk_widget_set_valign(sw, GTK_ALIGN_CENTER);
    if (name) g_object_set_data_full(G_OBJECT(sw), "name", g_strdup(name), g_free);
    g_signal_connect(sw, "notify::active", G_CALLBACK(on_choice_toggled), NULL);
    gtk_box_pack_start(GTK_BOX(row), sw, FALSE, FALSE, 0);
    gtk_container_add(GTK_CONTAINER(app.choice_list), row);
    if (folder && name) {
        SizeJob *j = g_new0(SizeJob, 1);
        j->name = g_strdup(name);
        j->label = g_object_ref(sz);
        g_thread_pool_push(size_pool, j, NULL);
    } else if (!name) {
        gtk_label_set_text(GTK_LABEL(sz), "Your apps' settings and data (the hidden files in your home)");
    }
}

static gint str_cmp(gconstpointer a, gconstpointer b) { return g_utf8_collate(*(char *const *)a, *(char *const *)b); }

static void fill_choice(void)
{
    gtk_container_foreach(GTK_CONTAINER(app.choice_list), destroy_child, NULL);
    GPtrArray *names = g_ptr_array_new_with_free_func(g_free);
    GDir *d = g_dir_open(app.home, 0, NULL);
    const char *n;
    while (d && (n = g_dir_read_name(d)))
        if (n[0] != '.' && !always_left_out(n)) g_ptr_array_add(names, g_strdup(n));
    if (d) g_dir_close(d);
    g_ptr_array_sort(names, str_cmp);
    for (guint i = 0; i < names->len; i++)
        choice_row(names->pdata[i], names->pdata[i], !g_hash_table_contains(app.off, names->pdata[i]), TRUE);
    choice_row(NULL, "Settings and app data", app.hidden_on, FALSE);
    g_ptr_array_free(names, TRUE);
    gtk_widget_show_all(app.choice_list);
}

/* ---- putting back ---- */

static void on_snap_row(GtkListBox *box, GtkListBoxRow *row, gpointer d)
{
    (void)box; (void)d;
    int i = GPOINTER_TO_INT(g_object_get_data(G_OBJECT(row), "index"));
    if (!app.snaps || i < 0 || i >= (int)app.snaps->len) return;
    app.snap = app.snaps->pdata[i];
    GError *e = NULL;
    if (!snap_load(app.snap, &e)) {
        say(app.put_msg, e->message, TRUE);
        g_clear_error(&e);
        app.snap = NULL;
    }
    g_free(app.folder);
    app.folder = g_strdup("");
    g_hash_table_remove_all(app.ticked);
    say(app.put_msg, "", FALSE);
    fill_browse();
}

static void fill_snaps(void)
{
    Drive *d = current_drive();
    if (app.snaps) g_ptr_array_free(app.snaps, TRUE);
    app.snaps = d && d->mounted ? snaps_list(d->path) : g_ptr_array_new_with_free_func((GDestroyNotify)snap_free);
    app.snap = NULL;
    gtk_container_foreach(GTK_CONTAINER(app.snap_list), destroy_child, NULL);
    for (guint i = 0; i < app.snaps->len; i++) {
        Snap *s = app.snaps->pdata[i];
        GtkWidget *row = gtk_list_box_row_new(), *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
        g_object_set_data(G_OBJECT(row), "index", GINT_TO_POINTER(i));
        g_object_set(box, "margin", 10, NULL);
        char *w = when_text(s->when), *f = count_text(s->files, "file", "files"), *b = size_text(s->bytes), *a = size_text(s->added);
        char *head = g_strdup_printf("%c%s", g_ascii_toupper(w[0]), w + 1), *sub = g_strdup_printf("%s · %s · %s new", f, b, a);
        gtk_box_pack_start(GTK_BOX(box), label(head, "name", 0), FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(box), label(sub, "sub", 0), FALSE, FALSE, 0);
        g_free(w); g_free(f); g_free(b); g_free(a); g_free(head); g_free(sub);
        gtk_container_add(GTK_CONTAINER(row), box);
        gtk_container_add(GTK_CONTAINER(app.snap_list), row);
    }
    gtk_widget_show_all(app.snap_list);
    gtk_label_set_text(GTK_LABEL(app.snap_empty), d ? "No backups on this drive yet." : "Plug in the drive your backups are on.");
    gtk_widget_set_visible(app.snap_empty, app.snaps->len == 0);
    fill_browse();
}

static void put_label(void)
{
    gtk_button_set_label(GTK_BUTTON(app.put_btn), g_hash_table_size(app.ticked) ? "Put Back Ticked" : "Put Back Everything");
}

static void on_tick(GtkToggleButton *t, gpointer d)
{
    (void)d;
    const char *p = g_object_get_data(G_OBJECT(t), "path");
    if (gtk_toggle_button_get_active(t)) g_hash_table_add(app.ticked, g_strdup(p));
    else g_hash_table_remove(app.ticked, p);
    char *n = count_text(g_hash_table_size(app.ticked), "item ticked", "items ticked");
    put_label();
    say(app.put_msg, g_hash_table_size(app.ticked) ? n : "", FALSE);
    g_free(n);
}

static void go_folder(const char *f)
{
    g_free(app.folder);
    app.folder = g_strdup(f);
    fill_browse();
}

static void on_open_folder(GtkButton *b, gpointer d)
{
    (void)d;
    go_folder(g_object_get_data(G_OBJECT(b), "path"));
}

static void on_up(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    char *p = g_path_get_dirname(app.folder);
    go_folder(!strcmp(p, ".") ? "" : p);
    g_free(p);
}

static void on_search(GtkSearchEntry *e, gpointer d)
{
    (void)e; (void)d;
    fill_browse();
}

static void fill_browse(void)
{
    gtk_container_foreach(GTK_CONTAINER(app.browse_list), destroy_child, NULL);
    gtk_widget_set_visible(app.browse_box, app.snap != NULL);
    put_label();
    if (!app.snap) {
        state_changed();
        return;
    }
    const char *q = gtk_entry_get_text(GTK_ENTRY(app.search));
    char *folded = g_utf8_casefold(q, -1);
    char *crumb = *app.folder ? g_strdup_printf("Home / %s", app.folder) : g_strdup("Home");
    gtk_label_set_text(GTK_LABEL(app.crumb), *q ? "Search results" : crumb);
    g_free(crumb);
    size_t fl = strlen(app.folder);
    int shown = 0;
    for (guint i = 0; i < app.snap->items->len && shown < 500; i++) {
        Item *it = app.snap->items->pdata[i];
        const char *rest;
        if (*q) {
            char *f = g_utf8_casefold(it->path, -1);
            gboolean hit = strstr(f, folded) != NULL && it->kind != 'D';
            g_free(f);
            if (!hit) continue;
            rest = it->path;
        } else {
            if (fl && (strncmp(it->path, app.folder, fl) || it->path[fl] != '/')) continue;
            rest = it->path + (fl ? fl + 1 : 0);
            if (!*rest || strchr(rest, '/')) continue;  /* only what is right in this folder */
        }
        GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        add_class(row, "entry-row");
        GtkWidget *tick = gtk_check_button_new();
        g_object_set_data_full(G_OBJECT(tick), "path", g_strdup(it->path), g_free);
        gtk_toggle_button_set_active(GTK_TOGGLE_BUTTON(tick), g_hash_table_contains(app.ticked, it->path));
        g_signal_connect(tick, "toggled", G_CALLBACK(on_tick), NULL);
        gtk_box_pack_start(GTK_BOX(row), tick, FALSE, FALSE, 0);
        if (it->kind == 'D' && !*q) {
            GtkWidget *b = button(rest, "Open this folder", G_CALLBACK(on_open_folder), NULL, "folder");
            g_object_set_data_full(G_OBJECT(b), "path", g_strdup(it->path), g_free);
            gtk_widget_set_halign(gtk_bin_get_child(GTK_BIN(b)), GTK_ALIGN_START);
            gtk_box_pack_start(GTK_BOX(row), b, TRUE, TRUE, 0);
        } else {
            GtkWidget *n = label(rest, NULL, 0);
            gtk_label_set_line_wrap(GTK_LABEL(n), FALSE);
            gtk_label_set_ellipsize(GTK_LABEL(n), PANGO_ELLIPSIZE_MIDDLE);
            gtk_box_pack_start(GTK_BOX(row), n, TRUE, TRUE, 0);
            char *sz = it->kind == 'F' ? size_text(it->size) : g_strdup("link");
            gtk_box_pack_start(GTK_BOX(row), label(sz, "sub", 1), FALSE, FALSE, 0);
            g_free(sz);
        }
        gtk_container_add(GTK_CONTAINER(app.browse_list), row);
        shown++;
    }
    if (!shown) gtk_container_add(GTK_CONTAINER(app.browse_list), label(*q ? "Nothing in this backup matches." : "This folder is empty.", "sub", 0));
    g_free(folded);
    gtk_widget_show_all(app.browse_list);
    state_changed();
}

typedef struct {
    char *drive, *dest, **paths;
    Snap *snap;
    gint64 restored;
    GPtrArray *problems;
    gboolean ok;
    GError *err;
    int mode;  /* 0 put back, 1 check */
    gint64 bad;
} PutJob;

static gboolean put_done(gpointer data)
{
    PutJob *j = data;
    put_runs++;
    set_busy(FALSE, NULL);
    if (j->mode == 1) {
        char *m = j->bad < 0 ? g_strdup(j->err ? j->err->message : "This backup can't be read.")
                 : j->bad ? g_strdup_printf("%" G_GINT64_FORMAT " copies on the drive are damaged or missing: those files can't be put back "
                                            "from this backup. Back up again to make fresh copies.", j->bad)
                          : g_strdup("Checked: every file in this backup reads back exactly as it was saved.");
        say(app.put_msg, m, j->bad != 0);
        g_free(m);
    } else if (j->ok) {
        char *n = count_text(j->restored, "item", "items");
        char *m = j->problems->len ? g_strdup_printf("Put back %s. %u couldn't be: %s", n, j->problems->len, (char *)j->problems->pdata[0])
                 : j->dest ? g_strdup_printf("Put back %s into %s.", n, j->dest)
                           : g_strdup_printf("Put back %s where they were. Files changed since were kept, the old ones beside them as “(restored)”.", n);
        say(app.put_msg, m, j->problems->len > 0);
        g_free(m);
        g_free(n);
    } else {
        say(app.put_msg, j->err ? j->err->message : "Putting back stopped.", TRUE);
    }
    g_ptr_array_free(j->problems, TRUE);
    g_strfreev(j->paths);
    g_clear_error(&j->err);
    g_free(j->drive);
    g_free(j->dest);
    g_free(j);
    return G_SOURCE_REMOVE;
}

static gpointer put_thread(gpointer data)
{
    PutJob *j = data;
    Job job = {&app.cancel, on_progress, NULL};
    if (j->mode == 1) j->bad = check_snap(j->drive, j->snap, &job, &j->err);
    else j->ok = restore_run(j->drive, j->snap, (const char *const *)j->paths, app.home, j->dest, &job, &j->restored, j->problems, &j->err);
    g_idle_add(put_done, j);
    return NULL;
}

static void start_put(int mode)
{
    Drive *d = current_drive();
    if (!d || !app.snap || app.busy) return;
    PutJob *j = g_new0(PutJob, 1);
    j->drive = g_strdup(d->path);
    j->snap = app.snap;
    j->mode = mode;
    j->problems = g_ptr_array_new_with_free_func(g_free);
    GPtrArray *p = g_ptr_array_new();
    GHashTableIter it;
    gpointer k;
    g_hash_table_iter_init(&it, app.ticked);
    while (g_hash_table_iter_next(&it, &k, NULL)) g_ptr_array_add(p, g_strdup(k));
    g_ptr_array_add(p, NULL);
    j->paths = (char **)g_ptr_array_free(p, FALSE);
    if (mode == 0 && gtk_combo_box_get_active(GTK_COMBO_BOX(app.put_where)) == 1) {
        char *date = g_strdup(app.snap->name), *name = g_strdup_printf("Restored %s", date);
        j->dest = g_build_filename(app.home, name, NULL);
        g_free(date);
        g_free(name);
    }
    say(app.put_msg, "", FALSE);
    set_busy(TRUE, mode == 1 ? "Reading every copy again…" : "Putting back…");
    g_thread_unref(g_thread_new("backup-put", put_thread, j));
}

static void on_put(GtkButton *b, gpointer d) { (void)b; (void)d; start_put(0); }
static void on_check(GtkButton *b, gpointer d) { (void)b; (void)d; start_put(1); }

static gboolean confirm(const char *title, const char *text, const char *action)
{
    if (app.test_dir) return TRUE;  /* (tests answer for the person) */
    GtkWidget *d = gtk_message_dialog_new(GTK_WINDOW(app.win), GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT, GTK_MESSAGE_OTHER,
                                          GTK_BUTTONS_NONE, "%s", title);
    gtk_message_dialog_format_secondary_text(GTK_MESSAGE_DIALOG(d), "%s", text);
    gtk_dialog_add_button(GTK_DIALOG(d), "Cancel", GTK_RESPONSE_CANCEL);
    add_class(gtk_dialog_add_button(GTK_DIALOG(d), action, GTK_RESPONSE_OK), "danger");
    gtk_dialog_set_default_response(GTK_DIALOG(d), GTK_RESPONSE_CANCEL);
    gboolean yes = gtk_dialog_run(GTK_DIALOG(d)) == GTK_RESPONSE_OK;
    gtk_widget_destroy(d);
    return yes;
}

static void on_delete_snap(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    Drive *dr = current_drive();
    if (!dr || !app.snap || app.busy) return;
    char *w = when_text(app.snap->when), *t = g_strdup_printf("Delete the backup from %s?", w);
    if (confirm(t, "Files it shares with your other backups stay; the rest of it is gone for good.", "Delete Backup")) {
        GError *e = NULL;
        if (snap_delete(dr->path, app.snap->name, &e)) say(app.put_msg, "That backup is deleted.", FALSE);
        else say(app.put_msg, e->message, TRUE);
        g_clear_error(&e);
        fill_snaps();
        fill_status();
        fill_drives();
    }
    g_free(w);
    g_free(t);
}

/* ---- window buttons and look ---- */

static gboolean wm_run(const char *const *argv)
{
    char *out = NULL;
    int status = 0;
    gboolean ok = g_spawn_sync(NULL, (char **)argv, NULL, G_SPAWN_SEARCH_PATH | G_SPAWN_STDERR_TO_DEV_NULL, NULL, NULL, &out, NULL,
                               &status, NULL) &&
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

static void on_close(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    gtk_window_close(GTK_WINDOW(app.win));
}

static GtkWidget *title_button(const char *icon, const char *tip, const char *role, GCallback cb)
{
    GtkWidget *b = gtk_button_new_from_icon_name(icon, GTK_ICON_SIZE_MENU);
    gtk_widget_set_tooltip_text(b, tip);
    gtk_widget_set_can_focus(b, FALSE);
    gtk_widget_set_valign(b, GTK_ALIGN_CENTER);
    add_class(b, "titlebutton");
    add_class(b, role);
    g_signal_connect(b, "clicked", cb, NULL);
    return b;
}

static const char *CSS =
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
    ".title { font-size: 12pt; font-weight: 700; color: @nexa_text_strong; }"
    ".name { color: @nexa_text_strong; font-weight: 600; }"
    ".sub { color: @nexa_muted; }"
    ".bad { color: @nexa_crit; }"
    ".mark { color: @nexa_accent; font-size: 15pt; }"
    ".card { background-color: @nexa_surface; border-radius: 16px; padding: 16px; }"
    "list { background-color: transparent; }"
    "row.drive, .choice, .entry-row { background-color: @nexa_surface; border-radius: 10px; margin: 0 0 8px 0; }"
    ".choice, .entry-row { padding: 8px 12px; }"
    "row.drive:hover, row:hover { background-color: @nexa_surface_hi; }"
    "row.drive.chosen { box-shadow: inset 0 0 0 2px @nexa_accent; }"
    "row:selected { background-color: @nexa_selected; }"
    "button { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text_strong;"
    "  border: 1px solid @nexa_border; border-radius: 10px; padding: 4px 12px; box-shadow: none; text-shadow: none;"
    "  transition: background-color 160ms cubic-bezier(0.22, 1, 0.36, 1); }"
    "button:hover { background-color: @nexa_hover; }"
    "button:disabled { color: @nexa_disabled; background-color: @nexa_surface; }"
    "button.primary { background-color: @nexa_accent; color: @nexa_on_accent; font-weight: 700; border-color: @nexa_accent; padding: 8px 20px; }"
    "button.primary:hover { background-color: @nexa_accent_hover; }"
    "button.primary:disabled { background-color: @nexa_surface_hi; color: @nexa_disabled; border-color: @nexa_border; }"
    "button.danger, button.danger-text { color: @nexa_crit; }"
    "button.danger { background-color: @nexa_crit; color: @nexa_on_accent; font-weight: 700; }"
    "button.folder { background-color: transparent; border-color: transparent; font-weight: 600; }"
    "button.nav { background-color: transparent; border-color: transparent; color: @nexa_muted; font-weight: 600; }"
    "button.nav:hover { background-color: @nexa_hover; color: @nexa_text_strong; }"
    "button.nav.on { background-color: @nexa_selected; color: @nexa_selected_text; }"
    "progressbar trough { background-color: alpha(@nexa_text, 0.12); border-radius: 6px; min-height: 8px; }"
    "progressbar progress { background-color: @nexa_accent; border-radius: 6px; min-height: 8px; }"
    "switch { background-image: none; background-color: @nexa_surface_hi; border: 1px solid @nexa_border; border-radius: 14px; }"
    "switch:checked { background-color: @nexa_accent; }"
    "switch slider { background-color: @nexa_text_strong; border-radius: 50%; border: none; }"
    "check { background-image: none; background-color: @nexa_field; border: 1px solid @nexa_border; border-radius: 6px; color: @nexa_on_accent; }"
    "check:checked { background-color: @nexa_accent; border-color: @nexa_accent; }"
    "entry, combobox button { background-image: none; background-color: @nexa_field; color: @nexa_text; border: 1px solid @nexa_border;"
    "  border-radius: 6px; padding: 4px 8px; box-shadow: none; }"
    "scrollbar slider { background-color: alpha(@nexa_text, 0.25); border-radius: 6px; min-width: 6px; min-height: 6px; }"
    "headerbar { background-image: none; background-color: @nexa_surface; color: @nexa_text_strong;"
    "  border-bottom: 1px solid @nexa_border; box-shadow: inset 0 1px @nexa_edge; min-height: 42px; padding: 0 8px; }"
    "headerbar .title { font-weight: 600; font-size: 10pt; }"
    "headerbar button.titlebutton { background-color: transparent; color: @nexa_text; border: none; border-radius: 50%;"
    "  min-width: 26px; min-height: 26px; padding: 0; }"
    "headerbar button.titlebutton:hover { background-color: @nexa_hover; }"
    "headerbar button.titlebutton.close:hover { background-color: @nexa_crit; color: @nexa_on_accent; }";

static GtkCssProvider *live_theme;

static void reload_theme(void)
{
    char *p = g_build_filename(g_get_user_config_dir(), "gtk-3.0", "gtk.css", NULL);
    gtk_css_provider_load_from_path(live_theme, p, NULL);
    g_free(p);
    if (app.win) gtk_widget_queue_draw(app.win);
}

static void on_theme(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent ev, gpointer d)
{
    (void)m; (void)f; (void)o; (void)d;
    if (ev != G_FILE_MONITOR_EVENT_DELETED) reload_theme();
}

static void follow_theme(void)
{
    live_theme = gtk_css_provider_new();
    reload_theme();
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(live_theme), GTK_STYLE_PROVIDER_PRIORITY_USER + 1);
    char *p = g_build_filename(g_get_user_config_dir(), "gtk-3.0", "gtk.css", NULL);
    GFile *f = g_file_new_for_path(p);
    GFileMonitor *m = g_file_monitor_file(f, G_FILE_MONITOR_NONE, NULL, NULL);
    if (m) g_signal_connect(m, "changed", G_CALLBACK(on_theme), NULL);
    g_object_unref(f);
    g_free(p);
}

/* ---- pages ---- */

static void show_page(int i)
{
    static const char *names[3] = {"backup", "choice", "put"};
    for (int k = 0; k < 3; k++) {
        GtkStyleContext *sc = gtk_widget_get_style_context(app.nav[k]);
        if (k == i) gtk_style_context_add_class(sc, "on");
        else gtk_style_context_remove_class(sc, "on");
    }
    if (i == 1) fill_choice();
    gtk_stack_set_visible_child_name(GTK_STACK(app.stack), names[i]);
    state_changed();
}

static void on_nav(GtkButton *b, gpointer d)
{
    (void)b;
    show_page(GPOINTER_TO_INT(d));
}

static GtkWidget *scroller(GtkWidget *child)
{
    GtkWidget *s = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(s), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    gtk_container_add(GTK_CONTAINER(s), child);
    return s;
}

static GtkWidget *page(void)
{
    GtkWidget *p = gtk_box_new(GTK_ORIENTATION_VERTICAL, 16);
    add_class(p, "page");
    g_object_set(p, "margin", 24, NULL);
    return p;
}

static GtkWidget *build_backup_page(void)
{
    GtkWidget *p = page();
    GtkWidget *card = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 16);
    add_class(card, "card");
    app.badge = gtk_drawing_area_new();
    gtk_widget_set_size_request(app.badge, 72, 72);
    g_signal_connect(app.badge, "draw", G_CALLBACK(draw_badge), NULL);
    gtk_box_pack_start(GTK_BOX(card), app.badge, FALSE, FALSE, 0);
    GtkWidget *words = gtk_box_new(GTK_ORIENTATION_VERTICAL, 4);
    gtk_widget_set_valign(words, GTK_ALIGN_CENTER);
    app.status_head = label("", "heading", 0);
    app.status_sub = label("", "sub", 0);
    gtk_box_pack_start(GTK_BOX(words), app.status_head, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(words), app.status_sub, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(card), words, TRUE, TRUE, 0);
    app.backup_btn = button("Back Up Now", "Copy what's new or changed to the drive", G_CALLBACK(on_backup), NULL, "primary");
    gtk_widget_set_valign(app.backup_btn, GTK_ALIGN_CENTER);
    gtk_box_pack_start(GTK_BOX(card), app.backup_btn, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(p), card, FALSE, FALSE, 0);

    app.prog_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    add_class(app.prog_box, "card");
    GtkWidget *prow = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
    app.prog_bar = gtk_progress_bar_new();
    gtk_widget_set_valign(app.prog_bar, GTK_ALIGN_CENTER);
    gtk_box_pack_start(GTK_BOX(prow), app.prog_bar, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(prow), button("Stop", "Stop: the backups made before stay as they are", G_CALLBACK(on_stop), NULL, NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.prog_box), prow, FALSE, FALSE, 0);
    app.prog_file = label("", "name", 0);
    gtk_label_set_line_wrap(GTK_LABEL(app.prog_file), FALSE);
    gtk_label_set_ellipsize(GTK_LABEL(app.prog_file), PANGO_ELLIPSIZE_MIDDLE);
    app.prog_counts = label("", "sub", 0);
    gtk_box_pack_start(GTK_BOX(app.prog_box), app.prog_file, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.prog_box), app.prog_counts, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(p), app.prog_box, FALSE, FALSE, 0);
    keep_hidden(app.prog_box);
    app.result = label("", NULL, 0);
    keep_hidden(app.result);
    gtk_box_pack_start(GTK_BOX(p), app.result, FALSE, FALSE, 0);
    app.problems = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    gtk_box_pack_start(GTK_BOX(p), app.problems, FALSE, FALSE, 0);

    gtk_box_pack_start(GTK_BOX(p), label("Drives", "title", 0), FALSE, FALSE, 0);
    app.drive_list = gtk_list_box_new();
    gtk_list_box_set_selection_mode(GTK_LIST_BOX(app.drive_list), GTK_SELECTION_NONE);
    gtk_list_box_set_activate_on_single_click(GTK_LIST_BOX(app.drive_list), TRUE);
    g_signal_connect(app.drive_list, "row-activated", G_CALLBACK(on_drive_row), NULL);
    gtk_box_pack_start(GTK_BOX(p), app.drive_list, FALSE, FALSE, 0);
    app.drive_empty = label("No drive yet. Plug in a USB stick or drive: it shows up here by itself.", "sub", 0);
    keep_hidden(app.drive_empty);
    gtk_box_pack_start(GTK_BOX(p), app.drive_empty, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(p), label("Backups aren't encrypted: keep the drive somewhere safe. Old backups are thinned out by "
                                         "themselves: every one from the last 2 days, then one a day, a week and a month.", "sub", 0),
                       FALSE, FALSE, 0);
    return scroller(p);
}

static GtkWidget *build_choice_page(void)
{
    GtkWidget *p = page();
    gtk_box_pack_start(GTK_BOX(p), label("What to Back Up", "heading", 0), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(p), label("The folders in your home. Always left out: caches, the Trash, secret keys and passwords, "
                                         "browser profiles and other drives.", "sub", 0), FALSE, FALSE, 0);
    app.choice_list = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_box_pack_start(GTK_BOX(p), app.choice_list, FALSE, FALSE, 0);
    return scroller(p);
}

static GtkWidget *build_put_page(void)
{
    GtkWidget *outer = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 16);
    add_class(outer, "page");
    g_object_set(outer, "margin", 24, NULL);
    GtkWidget *left = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
    gtk_widget_set_size_request(left, 280, -1);
    gtk_box_pack_start(GTK_BOX(left), label("Backups on this drive", "title", 0), FALSE, FALSE, 0);
    app.snap_list = gtk_list_box_new();
    g_signal_connect(app.snap_list, "row-activated", G_CALLBACK(on_snap_row), NULL);
    gtk_list_box_set_activate_on_single_click(GTK_LIST_BOX(app.snap_list), TRUE);
    gtk_box_pack_start(GTK_BOX(left), scroller(app.snap_list), TRUE, TRUE, 0);
    app.snap_empty = label("", "sub", 0);
    keep_hidden(app.snap_empty);
    gtk_box_pack_start(GTK_BOX(left), app.snap_empty, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(outer), left, FALSE, FALSE, 0);

    app.browse_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
    GtkWidget *bar = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(bar), button("‹ Up", "The folder above", G_CALLBACK(on_up), NULL, NULL), FALSE, FALSE, 0);
    app.crumb = label("Home", "name", 0);
    gtk_label_set_line_wrap(GTK_LABEL(app.crumb), FALSE);
    gtk_label_set_ellipsize(GTK_LABEL(app.crumb), PANGO_ELLIPSIZE_START);
    gtk_box_pack_start(GTK_BOX(bar), app.crumb, TRUE, TRUE, 0);
    app.search = gtk_search_entry_new();
    gtk_entry_set_placeholder_text(GTK_ENTRY(app.search), "Find a file in this backup");
    g_signal_connect(app.search, "search-changed", G_CALLBACK(on_search), NULL);
    gtk_box_pack_start(GTK_BOX(bar), app.search, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.browse_box), bar, FALSE, FALSE, 0);
    app.browse_list = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_box_pack_start(GTK_BOX(app.browse_box), scroller(app.browse_list), TRUE, TRUE, 0);
    GtkWidget *act = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    app.put_where = gtk_combo_box_text_new();
    gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(app.put_where), "Where they were");
    gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(app.put_where), "Into a new folder in my home");
    gtk_combo_box_set_active(GTK_COMBO_BOX(app.put_where), 0);
    gtk_box_pack_start(GTK_BOX(act), app.put_where, FALSE, FALSE, 0);
    app.put_btn = button("Put Back Everything", "Copy them back from the drive", G_CALLBACK(on_put), NULL, "primary");
    gtk_box_pack_start(GTK_BOX(act), app.put_btn, FALSE, FALSE, 0);
    gtk_box_pack_end(GTK_BOX(act), button("Delete This Backup", NULL, G_CALLBACK(on_delete_snap), NULL, "danger-text"), FALSE, FALSE, 0);
    gtk_box_pack_end(GTK_BOX(act), button("Check", "Read every copy in this backup again", G_CALLBACK(on_check), NULL, NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.browse_box), act, FALSE, FALSE, 0);
    app.put_msg = label("", NULL, 0);
    keep_hidden(app.put_msg);
    gtk_box_pack_start(GTK_BOX(app.browse_box), app.put_msg, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(outer), app.browse_box, TRUE, TRUE, 0);
    keep_hidden(app.browse_box);
    return outer;
}

/* ---- for tests: state out, commands in (BACKUP_TEST) ---- */

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
    GString *s = g_string_new("{\"page\": ");
    json_str(s, gtk_stack_get_visible_child_name(GTK_STACK(app.stack)));
    g_string_append_printf(s, ", \"busy\": %d, \"drive\": %d, \"drives\": [", app.busy, app.drive);
    for (guint i = 0; app.drives && i < app.drives->len; i++) {
        Drive *dr = app.drives->pdata[i];
        g_string_append(s, i ? ", {\"label\": " : "{\"label\": ");
        json_str(s, dr->label);
        g_string_append_printf(s, ", \"mounted\": %d, \"ro\": %d}", dr->mounted, dr->ro);
    }
    g_string_append(s, "], \"head\": ");
    json_str(s, gtk_label_get_text(GTK_LABEL(app.status_head)));
    g_string_append(s, ", \"sub\": ");
    json_str(s, gtk_label_get_text(GTK_LABEL(app.status_sub)));
    g_string_append_printf(s, ", \"status\": %d, \"can_back_up\": %d, \"result\": ", status_kind, gtk_widget_get_sensitive(app.backup_btn));
    json_str(s, gtk_widget_get_visible(app.result) ? gtk_label_get_text(GTK_LABEL(app.result)) : "");
    g_string_append(s, ", \"progress\": ");
    json_str(s, app.busy ? gtk_label_get_text(GTK_LABEL(app.prog_counts)) : "");
    g_string_append_printf(s, ", \"snaps\": %u, \"snap\": ", app.snaps ? app.snaps->len : 0);
    json_str(s, app.snap ? app.snap->name : "");
    g_string_append(s, ", \"folder\": ");
    json_str(s, app.folder);
    g_string_append(s, ", \"entries\": [");
    GList *rows = gtk_container_get_children(GTK_CONTAINER(app.browse_list));
    int first = 1;
    for (GList *r = rows; r; r = r->next) {
        if (!GTK_IS_BOX(r->data)) continue;
        GList *kids = gtk_container_get_children(GTK_CONTAINER(r->data));
        const char *p = kids ? g_object_get_data(G_OBJECT(kids->data), "path") : NULL;
        if (p) {
            if (!first) g_string_append(s, ", ");
            json_str(s, p);
            first = 0;
        }
        g_list_free(kids);
    }
    g_list_free(rows);
    g_string_append(s, "], \"put_btn\": ");
    json_str(s, gtk_button_get_label(GTK_BUTTON(app.put_btn)));
    g_string_append(s, ", \"put_msg\": ");
    json_str(s, gtk_widget_get_visible(app.put_msg) ? gtk_label_get_text(GTK_LABEL(app.put_msg)) : "");
    g_string_append(s, ", \"off\": [");
    GHashTableIter it;
    gpointer k;
    first = 1;
    g_hash_table_iter_init(&it, app.off);
    while (g_hash_table_iter_next(&it, &k, NULL)) {
        if (!first) g_string_append(s, ", ");
        json_str(s, k);
        first = 0;
    }
    g_string_append_printf(s, "], \"hidden_on\": %d, \"runs\": %d, \"puts\": %d}\n", app.hidden_on, runs, put_runs);
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

static void set_switch(const char *name, gboolean on)
{
    GList *rows = gtk_container_get_children(GTK_CONTAINER(app.choice_list));
    for (GList *r = rows; r; r = r->next) {
        GList *kids = gtk_container_get_children(GTK_CONTAINER(r->data));
        for (GList *k = kids; k; k = k->next)
            if (GTK_IS_SWITCH(k->data)) {
                const char *n = g_object_get_data(G_OBJECT(k->data), "name");
                if ((!n && !strcmp(name, ".")) || (n && !strcmp(n, name))) gtk_switch_set_active(GTK_SWITCH(k->data), on);
            }
        g_list_free(kids);
    }
    g_list_free(rows);
}

static void test_command(char *line)
{
    g_strstrip(line);
    char **w = g_strsplit(line, " ", 2);
    const char *c = w[0], *a = w[1] ? w[1] : "";
    if (!strcmp(c, "page")) show_page(atoi(a));
    else if (!strcmp(c, "drive")) {
        int i = atoi(a);
        if (app.drives && i >= 0 && i < (int)app.drives->len) {
            if (((Drive *)app.drives->pdata[i])->mounted) {
                app.drive = i;
                save_conf();
                fill_drives();
                fill_status();
                fill_snaps();
            } else {
                on_open_drive(NULL, app.drives->pdata[i]);
            }
        }
    } else if (!strcmp(c, "backup")) start_backup();
    else if (!strcmp(c, "stop")) on_stop(NULL, NULL);
    else if (!strcmp(c, "off")) set_switch(a, FALSE);
    else if (!strcmp(c, "on")) set_switch(a, TRUE);
    else if (!strcmp(c, "snap")) {
        GList *rows = gtk_container_get_children(GTK_CONTAINER(app.snap_list));
        GtkWidget *r = g_list_nth_data(rows, (guint)atoi(a));
        if (r) on_snap_row(NULL, GTK_LIST_BOX_ROW(r), NULL);
        g_list_free(rows);
    } else if (!strcmp(c, "folder")) go_folder(a);
    else if (!strcmp(c, "tick")) {
        g_hash_table_add(app.ticked, g_strdup(a));
        fill_browse();
    } else if (!strcmp(c, "where")) gtk_combo_box_set_active(GTK_COMBO_BOX(app.put_where), atoi(a));
    else if (!strcmp(c, "put")) start_put(0);
    else if (!strcmp(c, "check")) start_put(1);
    else if (!strcmp(c, "delete")) on_delete_snap(NULL, NULL);
    else if (!strcmp(c, "search")) gtk_entry_set_text(GTK_ENTRY(app.search), a);
    else if (!strcmp(c, "rescan")) watch_drives(NULL);
    g_strfreev(w);
    state_changed();
}

static gboolean on_test_input(GIOChannel *ch, GIOCondition cond, gpointer d)
{
    (void)cond; (void)d;
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
    int fd = open(fifo, O_RDWR | O_NONBLOCK | O_CLOEXEC);
    g_free(fifo);
    if (fd < 0) return;
    GIOChannel *ch = g_io_channel_unix_new(fd);
    g_io_channel_set_flags(ch, G_IO_FLAG_NONBLOCK, NULL);
    g_io_add_watch(ch, G_IO_IN, on_test_input, NULL);
}

/* ---- the window ---- */

static gboolean on_delete_event(GtkWidget *w, GdkEvent *e, gpointer d)
{
    (void)w; (void)e; (void)d;
    if (!app.busy || app.test_dir) return FALSE;
    if (confirm("Stop the backup?", "The backups made before stay as they are; this one won't be listed.", "Stop and Close")) {
        g_atomic_int_set(&app.cancel, 1);
        return FALSE;
    }
    return TRUE;
}

static gboolean on_key(GtkWidget *w, GdkEventKey *e, gpointer d)
{
    (void)w; (void)d;
    if ((e->state & GDK_CONTROL_MASK) && (e->keyval == GDK_KEY_q || e->keyval == GDK_KEY_w)) {
        gtk_window_close(GTK_WINDOW(app.win));
        return TRUE;
    }
    if ((e->state & GDK_MOD1_MASK) && e->keyval >= GDK_KEY_1 && e->keyval <= GDK_KEY_3) {
        if (!app.busy) show_page((int)(e->keyval - GDK_KEY_1));
        return TRUE;
    }
    if (e->keyval == GDK_KEY_Escape && app.busy) {
        on_stop(NULL, NULL);
        return TRUE;
    }
    return FALSE;
}

int main(int argc, char **argv)
{
    gtk_init(&argc, &argv);
    g_set_prgname(APP_ID);
    g_set_application_name("Backup");
    app.home = g_strdup(g_get_home_dir());
    app.conf = g_build_filename(g_get_user_config_dir(), APP_ID, "settings.ini", NULL);
    app.ticked = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    app.folder = g_strdup("");
    app.drive = -1;
    app.test_dir = g_strdup(g_getenv("BACKUP_TEST"));
    load_conf();
    size_pool = g_thread_pool_new(size_work, NULL, 2, FALSE, NULL);

    GtkCssProvider *css = gtk_css_provider_new();
    gtk_css_provider_load_from_data(css, CSS, -1, NULL);
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(css), GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    follow_theme();

    app.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_default_size(GTK_WINDOW(app.win), 900, 640);
    gtk_window_set_icon_name(GTK_WINDOW(app.win), APP_ID);
    GtkWidget *bar = gtk_header_bar_new();
    gtk_header_bar_set_show_close_button(GTK_HEADER_BAR(bar), FALSE);
    GtkWidget *nav = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 4);
    const char *names[3] = {"Back Up", "What to Back Up", "Put Back"};
    const char *tips[3] = {"Back up to a drive (Alt+1)", "Which folders (Alt+2)", "Get files back from a backup (Alt+3)"};
    for (int i = 0; i < 3; i++) {
        app.nav[i] = button(names[i], tips[i], G_CALLBACK(on_nav), GINT_TO_POINTER(i), "nav");
        gtk_widget_set_can_focus(app.nav[i], FALSE);
        gtk_box_pack_start(GTK_BOX(nav), app.nav[i], FALSE, FALSE, 0);
    }
    gtk_header_bar_pack_start(GTK_HEADER_BAR(bar), nav);
    gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), title_button("window-close-symbolic", "Close (Ctrl+Q)", "close", G_CALLBACK(on_close)));
    gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), title_button("window-minimize-symbolic", "Minimize", "minimize", G_CALLBACK(on_minimize)));
    gtk_window_set_titlebar(GTK_WINDOW(app.win), bar);
    gtk_window_set_title(GTK_WINDOW(app.win), "Backup");
    app.stack = gtk_stack_new();
    gtk_stack_set_transition_type(GTK_STACK(app.stack), GTK_STACK_TRANSITION_TYPE_CROSSFADE);
    gtk_stack_set_transition_duration(GTK_STACK(app.stack), 160);
    gtk_stack_add_named(GTK_STACK(app.stack), build_backup_page(), "backup");
    gtk_stack_add_named(GTK_STACK(app.stack), build_choice_page(), "choice");
    gtk_stack_add_named(GTK_STACK(app.stack), build_put_page(), "put");
    gtk_container_add(GTK_CONTAINER(app.win), app.stack);
    g_signal_connect(app.win, "key-press-event", G_CALLBACK(on_key), NULL);
    g_signal_connect(app.win, "delete-event", G_CALLBACK(on_delete_event), NULL);
    g_signal_connect(app.win, "destroy", G_CALLBACK(gtk_main_quit), NULL);
    gtk_widget_show_all(app.win);
    if (app.test_dir) test_setup();
    fill_drives();
    fill_status();
    fill_snaps();
    show_page(0);
    g_timeout_add_seconds(2, watch_drives, NULL);
    gtk_main();
    g_atomic_int_set(&app.cancel, 1);
    return 0;
}
