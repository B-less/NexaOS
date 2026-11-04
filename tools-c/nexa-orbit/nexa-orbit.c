/* nexa-orbit: NexaOS's app rail on the left edge of the screen (replaces
 * the Python Dock). Design: TODOnexa_identity.md, section "Orbit".
 *
 * A layer-shell surface (GTK3 + gtk-layer-shell, drawn with cairo): the
 * NexaOS mark (opens Nexus), the pinned apps, then running apps that aren't
 * pinned, each in a hexagon frame whose outline shows the app's state:
 * running, how busy (CPU of all its processes), in front, waiting on you,
 * starting, minimised. Window changes come from the compositor's event
 * socket; activity from /proc every 2 s while apps run.
 *
 *   nexa-orbit            run (one per session)
 *   pkill -USR1 nexa-orbit   Super+O: take the keyboard / give it back
 *
 * NEXA_ORBIT_STATE=<file>: write what is shown there as JSON lines after
 * every change (for tests). */
#include <dirent.h>
#include <errno.h>
#include <gio/gdesktopappinfo.h>
#include <glib-unix.h>
#include <gtk-layer-shell.h>
#include <gtk/gtk.h>
#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include "model.h"
#include "wm.h"

#define MAX_ITEMS 48
#define RAIL_W 60.0     /* width of the rail */
#define EDGE 8.0        /* gap between the rail and the screen edge */
#define CELL 54.0       /* step between icons */
#define HEX_R 24.0      /* frame radius, centre to point */
#define ICON 30         /* icon size inside the frame */
#define POINT 18.0      /* height of the rail's pointed ends */
#define END_PAD 10.0    /* room between a pointed end and the first/last frame */
#define DIVIDER 14.0    /* extra room for the line between pinned and running */
#define LABEL_ROOM 240  /* room to the right of the rail for the name */
#define GAP 4.0         /* room kept between the rail and the windows */
#define FOLD_LINE 4.0   /* width of the line of light when folded */
#define FOLD_HIT 6      /* how far from the edge the folded line takes the pointer */
#define LAUNCH_S 8.0    /* stop showing "starting" if no window comes by then */
#define KEY_IDLE_S 20   /* give the keyboard back after this long without a key */
#define MAX_PROCS 8192

typedef struct { double r, g, b; } rgb;

struct item {
    char id[ORBIT_ID_LEN];      /* desktop id without ".desktop"; "" = the mark */
    char name[128];
    GDesktopAppInfo *info;      /* NULL: the mark, or a window of an unknown app */
    GdkPixbuf *pix;
    int pinned, running, focused, urgent, minimized, nwin;
    double cpu, cpu_shown;      /* percent of one core */
    unsigned long long ticks;
    gint64 ticks_at;            /* when ticks was read (us), 0 = never */
    gint64 launched;            /* us, 0 = not starting */
};

static struct {
    GtkWidget *win, *area;
    struct item it[MAX_ITEMS];
    int n, npinned_shown;
    struct wm_win wins[WM_MAX_WIN];
    int win_item[WM_MAX_WIN];    /* each window's item, -1 if none */
    int nwins;
    struct orbit_conf conf;
    struct { rgb bg, surface, text, muted, accent, accent2, warn; double opacity; char icon_theme[128]; } c;
    int animations;
    int hover, kfocus, keyboard, menu_open, pointer_in;
    double fold_t, fold_target;  /* 0 = open, 1 = folded */
    guint anim_id, refresh_id, fold_id, key_idle_id;
    int ev_fd;
    guint ev_watch;
    GHashTable *wmclass;         /* lowercased StartupWMClass -> desktop id */
    int width, height;
    char conf_path[600], theme_path[600];
} O = {.hover = -1, .kfocus = -1, .ev_fd = -1};

static void refresh(void);
static void relayout(void);
static void start_anim(void);

static double now_s(void) { return g_get_monotonic_time() / 1e6; }

/* ---------- theme ---------- */

static rgb hexcol(const char *s, rgb fb)
{
    unsigned v;
    if (!s || sscanf(s, "#%6x", &v) != 1) return fb;
    return (rgb){((v >> 16) & 255) / 255.0, ((v >> 8) & 255) / 255.0, (v & 255) / 255.0};
}

static void set_rgb(cairo_t *cr, rgb c, double a) { cairo_set_source_rgba(cr, c.r, c.g, c.b, a); }

static rgb mix(rgb a, rgb b, double t) { return (rgb){a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t}; }

static void load_icons(void);

static void load_theme(void)
{
    /* Horizon, the default theme, until the file says otherwise. */
    O.c.bg = hexcol("#080c1e", O.c.bg);
    O.c.surface = hexcol("#141c3a", O.c.surface);
    O.c.text = hexcol("#e6ecff", O.c.text);
    O.c.muted = hexcol("#8f9bc4", O.c.muted);
    O.c.accent = hexcol("#4da3ff", O.c.accent);
    O.c.accent2 = hexcol("#8a6bff", O.c.accent2);
    O.c.warn = hexcol("#ffb547", O.c.warn);
    O.c.opacity = 0.78;
    O.c.icon_theme[0] = 0;
    char *text = NULL;
    if (g_file_get_contents(O.theme_path, &text, NULL, NULL)) {
        char **lines = g_strsplit(text, "\n", -1);
        for (char **l = lines; *l; l++) {
            char *eq = strchr(*l, '=');
            if (!eq || **l == '#') continue;
            *eq = 0;
            const char *k = *l, *v = eq + 1;
            if (!strcmp(k, "bg")) O.c.bg = hexcol(v, O.c.bg);
            else if (!strcmp(k, "surface")) O.c.surface = hexcol(v, O.c.surface);
            else if (!strcmp(k, "text")) O.c.text = hexcol(v, O.c.text);
            else if (!strcmp(k, "muted")) O.c.muted = hexcol(v, O.c.muted);
            else if (!strcmp(k, "accent")) O.c.accent = hexcol(v, O.c.accent);
            else if (!strcmp(k, "accent2")) O.c.accent2 = hexcol(v, O.c.accent2);
            else if (!strcmp(k, "warn")) O.c.warn = hexcol(v, O.c.warn);
            else if (!strcmp(k, "opacity")) O.c.opacity = fmin(1.0, fmax(0.3, g_ascii_strtod(v, NULL)));
            else if (!strcmp(k, "icon_theme") && id_ok(v)) snprintf(O.c.icon_theme, sizeof O.c.icon_theme, "%s", v);
        }
        g_strfreev(lines);
        g_free(text);
    }
    if (O.c.icon_theme[0]) {
        GtkSettings *s = gtk_settings_get_default();
        gchar *cur = NULL;
        g_object_get(s, "gtk-icon-theme-name", &cur, NULL);
        if (!cur || strcmp(cur, O.c.icon_theme)) g_object_set(s, "gtk-icon-theme-name", O.c.icon_theme, NULL);
        g_free(cur);
    }
}

static void read_animations(void)
{
    gboolean on = TRUE;
    g_object_get(gtk_settings_get_default(), "gtk-enable-animations", &on, NULL);
    O.animations = on;
}

/* ---------- apps ---------- */

static void index_wmclass(void)
{
    if (O.wmclass) g_hash_table_destroy(O.wmclass);
    O.wmclass = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    GList *all = g_app_info_get_all();
    for (GList *l = all; l; l = l->next) {
        if (!G_IS_DESKTOP_APP_INFO(l->data)) continue;
        const char *wm = g_desktop_app_info_get_startup_wm_class(l->data), *id = g_app_info_get_id(l->data);
        if (wm && id && g_str_has_suffix(id, ".desktop")) {
            g_hash_table_insert(O.wmclass, g_ascii_strdown(wm, -1), g_strndup(id, strlen(id) - 8));
        }
    }
    g_list_free_full(all, g_object_unref);
}

static GDesktopAppInfo *app_info(const char *id)
{
    if (!id_ok(id)) return NULL;
    char name[ORBIT_ID_LEN + 9];
    snprintf(name, sizeof name, "%.127s.desktop", id);
    return g_desktop_app_info_new(name);
}

/* The desktop id a window belongs to (its own app_id if it has none). */
static void window_app(const char *app_id, char *out, size_t cap)
{
    GDesktopAppInfo *i = app_info(app_id);
    const char *al = alias_of(app_id), *wm;
    if (i) {
        snprintf(out, cap, "%s", app_id);
    } else if (al && (i = app_info(al))) {
        snprintf(out, cap, "%s", al);
    } else if (O.wmclass && (wm = g_hash_table_lookup(O.wmclass, app_id))) {
        snprintf(out, cap, "%s", wm);
    } else {
        snprintf(out, cap, "%s", app_id);
    }
    if (i) g_object_unref(i);
}

static int find_item(const char *id)
{
    for (int i = 1; i < O.n; i++)
        if (!strcmp(O.it[i].id, id)) return i;
    return -1;
}

static GdkPixbuf *load_icon(GDesktopAppInfo *info)
{
    int scale = O.area ? gtk_widget_get_scale_factor(O.area) : 1;
    GtkIconTheme *theme = gtk_icon_theme_get_default();
    GIcon *icon = info ? g_app_info_get_icon(G_APP_INFO(info)) : NULL;
    GtkIconInfo *ii = NULL;
    if (icon) ii = gtk_icon_theme_lookup_by_gicon_for_scale(theme, icon, ICON, scale, GTK_ICON_LOOKUP_FORCE_SIZE);
    if (!ii) ii = gtk_icon_theme_lookup_icon_for_scale(theme, "application-x-executable", ICON, scale, GTK_ICON_LOOKUP_FORCE_SIZE);
    GdkPixbuf *p = ii ? gtk_icon_info_load_icon(ii, NULL) : NULL;
    if (ii) g_object_unref(ii);
    return p;
}

static void load_icons(void)
{
    for (int i = 1; i < O.n; i++) {
        if (O.it[i].pix) g_object_unref(O.it[i].pix);
        O.it[i].pix = load_icon(O.it[i].info);
    }
    if (O.area) gtk_widget_queue_draw(O.area);
}

static void item_clear(struct item *it)
{
    if (it->info) g_object_unref(it->info);
    if (it->pix) g_object_unref(it->pix);
    memset(it, 0, sizeof *it);
}

/* Carry an item's own state (icon, activity, starting) into the new list. */
static void add_item(struct item *out, int *n, struct item *old, int nold, const char *id, GDesktopAppInfo *info, int pinned)
{
    if (*n >= MAX_ITEMS) {
        if (info) g_object_unref(info);
        return;
    }
    struct item *it = &out[(*n)++];
    for (int k = 0; k < nold; k++)
        if (old[k].id[0] && !strcmp(old[k].id, id)) {
            *it = old[k];
            old[k].id[0] = 0;  /* taken: not freed below */
            old[k].info = NULL;
            old[k].pix = NULL;
            if (it->info) g_object_unref(it->info);
            it->info = info;
            it->pinned = pinned;
            if (info) snprintf(it->name, sizeof it->name, "%s", g_app_info_get_name(G_APP_INFO(info)));
            return;
        }
    memset(it, 0, sizeof *it);
    snprintf(it->id, sizeof it->id, "%s", id);
    it->info = info;
    it->pinned = pinned;
    snprintf(it->name, sizeof it->name, "%s", info ? g_app_info_get_name(G_APP_INFO(info)) : id);
    it->pix = load_icon(info);
}

static int cmp_str(const void *a, const void *b) { return strcmp(a, b); }

static void rebuild(void)
{
    static struct item old[MAX_ITEMS], fresh[MAX_ITEMS];
    int nold = O.n, n = 0;
    memcpy(old, O.it, sizeof old);
    /* the mark */
    memset(&fresh[n], 0, sizeof fresh[n]);
    snprintf(fresh[n].name, sizeof fresh[n].name, "Nexus");
    n++;
    for (int p = 0; p < O.conf.npinned; p++) {
        GDesktopAppInfo *info = app_info(O.conf.pinned[p]);
        if (info) add_item(fresh, &n, old + 1, nold - 1, O.conf.pinned[p], info, 1);  /* not installed: skipped */
    }
    O.npinned_shown = n - 1;
    /* running apps that aren't pinned, by name of their desktop id */
    static char extra[WM_MAX_WIN][ORBIT_ID_LEN];
    int ne = 0;
    for (int w = 0; w < O.nwins; w++) {
        char id[ORBIT_ID_LEN];
        window_app(O.wins[w].app_id, id, sizeof id);
        if (!id_ok(id) || conf_find(&O.conf, id) >= 0) continue;
        int dup = 0;
        for (int e = 0; e < ne; e++) dup |= !strcmp(extra[e], id);
        if (!dup && ne < WM_MAX_WIN) snprintf(extra[ne++], ORBIT_ID_LEN, "%s", id);
    }
    qsort(extra, (size_t)ne, ORBIT_ID_LEN, cmp_str);
    for (int e = 0; e < ne; e++) add_item(fresh, &n, old + 1, nold - 1, extra[e], app_info(extra[e]), 0);
    for (int k = 1; k < nold; k++)
        if (old[k].id[0]) item_clear(&old[k]);
    memcpy(O.it, fresh, sizeof fresh);
    O.n = n;
}

static void write_state(void);

static void apply_windows(void)
{
    for (int i = 0; i < O.n; i++) O.it[i].running = O.it[i].focused = O.it[i].urgent = O.it[i].minimized = O.it[i].nwin = 0;
    int minimized[MAX_ITEMS] = {0};
    for (int w = 0; w < O.nwins; w++) {
        char id[ORBIT_ID_LEN];
        window_app(O.wins[w].app_id, id, sizeof id);
        int i = O.win_item[w] = find_item(id);
        if (i < 0) continue;
        struct item *it = &O.it[i];
        it->running = 1;
        it->nwin++;
        it->focused |= O.wins[w].focused;
        it->urgent |= O.wins[w].urgent;
        minimized[i] += O.wins[w].minimized;
        it->launched = 0;  /* its window is here */
    }
    for (int i = 0; i < O.n; i++) {
        O.it[i].minimized = O.it[i].running && minimized[i] == O.it[i].nwin;
        if (!O.it[i].running) O.it[i].cpu = O.it[i].cpu_shown = 0, O.it[i].ticks_at = 0;
    }
}

static void refresh(void)
{
    int n = wm_windows(O.wins, WM_MAX_WIN);
    O.nwins = n < 0 ? 0 : n;
    int before = O.n;
    char ids_before[MAX_ITEMS * 64] = "";
    for (int i = 1; i < O.n; i++) g_strlcat(ids_before, O.it[i].id, sizeof ids_before), g_strlcat(ids_before, ",", sizeof ids_before);
    rebuild();
    apply_windows();
    char ids_after[MAX_ITEMS * 64] = "";
    for (int i = 1; i < O.n; i++) g_strlcat(ids_after, O.it[i].id, sizeof ids_after), g_strlcat(ids_after, ",", sizeof ids_after);
    if (before != O.n || strcmp(ids_before, ids_after)) {
        if (O.hover >= O.n) O.hover = -1;
        if (O.kfocus >= O.n) O.kfocus = O.n - 1;
        relayout();
    }
    start_anim();
    if (O.area) gtk_widget_queue_draw(O.area);
    write_state();
}

static gboolean refresh_later_cb(gpointer d)
{
    O.refresh_id = 0;
    refresh();
    return G_SOURCE_REMOVE;
}

/* Events come in bursts (a new window: several): look once they settle. */
static void refresh_soon(void)
{
    if (!O.refresh_id) O.refresh_id = g_timeout_add(100, refresh_later_cb, NULL);
}

/* ---------- activity ---------- */

static void sample_activity(void)
{
    static int pid[MAX_PROCS], ppid[MAX_PROCS];
    static unsigned long long ticks[MAX_PROCS];
    int np = 0;
    DIR *d = opendir("/proc");
    if (!d) return;
    struct dirent *e;
    while ((e = readdir(d)) && np < MAX_PROCS) {
        if (e->d_name[0] < '1' || e->d_name[0] > '9') continue;
        char path[300], line[1024];
        snprintf(path, sizeof path, "/proc/%s/stat", e->d_name);
        FILE *f = fopen(path, "re");
        if (!f) continue;
        int ok = fgets(line, sizeof line, f) != NULL;
        fclose(f);
        if (ok && parse_stat(line, &ppid[np], &ticks[np]) == 0) pid[np++] = atoi(e->d_name);
    }
    closedir(d);
    /* window pids -> items */
    GHashTable *owner = g_hash_table_new(g_direct_hash, g_direct_equal);
    for (int w = 0; w < O.nwins; w++)
        if (O.win_item[w] > 0) g_hash_table_insert(owner, GINT_TO_POINTER(O.wins[w].pid), GINT_TO_POINTER(O.win_item[w]));
    GHashTable *parent = g_hash_table_new(g_direct_hash, g_direct_equal);
    for (int p = 0; p < np; p++) g_hash_table_insert(parent, GINT_TO_POINTER(pid[p]), GINT_TO_POINTER(ppid[p]));
    unsigned long long sum[MAX_ITEMS] = {0};
    for (int p = 0; p < np; p++) {
        int cur = pid[p];
        for (int depth = 0; depth < 32 && cur > 1; depth++) {
            int k = GPOINTER_TO_INT(g_hash_table_lookup(owner, GINT_TO_POINTER(cur)));
            if (k > 0) {
                sum[k] += ticks[p];
                break;
            }
            cur = GPOINTER_TO_INT(g_hash_table_lookup(parent, GINT_TO_POINTER(cur)));
        }
    }
    g_hash_table_destroy(parent);
    g_hash_table_destroy(owner);
    gint64 t = g_get_monotonic_time();
    long clk = sysconf(_SC_CLK_TCK);
    for (int i = 1; i < O.n; i++) {
        struct item *it = &O.it[i];
        if (!it->running) continue;
        if (it->ticks_at) it->cpu = cpu_percent(it->ticks, sum[i], (t - it->ticks_at) / 1e6, clk);
        it->ticks = sum[i];
        it->ticks_at = t;
        if (!O.animations) it->cpu_shown = it->cpu;
    }
}

static gboolean every_2s(gpointer d)
{
    int any = 0;
    double t = now_s();
    for (int i = 1; i < O.n; i++) {
        any |= O.it[i].running;
        if (O.it[i].launched && t - O.it[i].launched / 1e6 > LAUNCH_S) {
            O.it[i].launched = 0;
            gtk_widget_queue_draw(O.area);
        }
    }
    if (O.ev_fd < 0) refresh();  /* no event socket: look every 2 s */
    if (any && O.fold_t < 0.99) {
        sample_activity();
        start_anim();
        gtk_widget_queue_draw(O.area);
        write_state();
    }
    return G_SOURCE_CONTINUE;
}

/* ---------- launching, and how apps end (for Why) ---------- */

static char *exits_path(void)
{
    const char *st = g_getenv("XDG_STATE_HOME");
    return st && *st ? g_build_filename(st, "nexa", "exits.jsonl", NULL)
                     : g_build_filename(g_get_home_dir(), ".local", "state", "nexa", "exits.jsonl", NULL);
}

struct watched { char app[64]; gint64 started; };

/* Only abnormal ends are written (a signal or an error code), at most 200
 * kept: the same file and format as nexa_common.exits. */
static void child_done(GPid pid, gint status, gpointer data)
{
    struct watched *w = data;
    char entry[200];
    double ran = (g_get_monotonic_time() - w->started) / 1e6;
    long at = (long)(g_get_real_time() / 1000000);
    int wrote = 0;
    if (WIFSIGNALED(status))
        wrote = snprintf(entry, sizeof entry, "{\"app\": \"%s\", \"at\": %ld, \"pid\": %d, \"ran_s\": %.1f, \"signal\": %d}\n", w->app, at,
                         pid, ran, WTERMSIG(status));
    else if (WIFEXITED(status) && WEXITSTATUS(status) != 0)
        wrote = snprintf(entry, sizeof entry, "{\"app\": \"%s\", \"at\": %ld, \"code\": %d, \"pid\": %d, \"ran_s\": %.1f}\n", w->app, at,
                         WEXITSTATUS(status), pid, ran);
    if (wrote > 0) {
        char *p = exits_path(), *dir = g_path_get_dirname(p);
        g_mkdir_with_parents(dir, 0700);
        FILE *f = fopen(p, "ae");
        if (f) {
            fputs(entry, f);
            long size = ftell(f);
            fclose(f);
            if (size > 200 * 120) {  /* trim back to the last 200 */
                char *text = NULL;
                if (g_file_get_contents(p, &text, NULL, NULL)) {
                    char **lines = g_strsplit(text, "\n", -1);
                    int nl = (int)g_strv_length(lines), from = nl > 201 ? nl - 201 : 0;
                    char *kept = g_strjoinv("\n", lines + from);
                    g_file_set_contents(p, kept, -1, NULL);
                    g_free(kept);
                    g_strfreev(lines);
                    g_free(text);
                }
            }
        }
        g_free(dir);
        g_free(p);
    }
    g_spawn_close_pid(pid);
    g_free(w);
}

static void own_session(gpointer d) { setsid(); }

static void launch(struct item *it)
{
    if (!it->info) return;
    const char *exec = g_desktop_app_info_get_string(it->info, "Exec");
    char clean[2048];
    gchar **argv = NULL;
    if (!exec || exec_strip(exec, clean, sizeof clean) < 0 || !g_shell_parse_argv(clean, NULL, &argv, NULL)) {
        g_free((gpointer)exec);
        return;
    }
    gchar *dir = g_desktop_app_info_get_string(it->info, "Path");
    GPid pid;
    GError *err = NULL;
    if (g_spawn_async(dir && *dir ? dir : NULL, argv, NULL,
                      G_SPAWN_SEARCH_PATH | G_SPAWN_DO_NOT_REAP_CHILD | G_SPAWN_STDOUT_TO_DEV_NULL | G_SPAWN_STDERR_TO_DEV_NULL,
                      own_session, NULL, &pid, &err)) {
        struct watched *w = g_new0(struct watched, 1);
        gchar *base = g_path_get_basename(argv[0]);
        size_t k = 0;
        for (const char *c = base; *c && k + 1 < sizeof w->app; c++)  /* JSON-safe */
            w->app[k++] = (g_ascii_isalnum(*c) || *c == '.' || *c == '_' || *c == '-') ? *c : '_';
        g_free(base);
        w->started = g_get_monotonic_time();
        g_child_watch_add(pid, child_done, w);
        it->launched = g_get_monotonic_time();
        start_anim();
    } else {
        g_printerr("nexa-orbit: couldn't start %s: %s\n", it->id, err->message);
        g_error_free(err);
    }
    g_free(dir);
    g_strfreev(argv);
    g_free((gpointer)exec);
}

static void spawn_simple(const char *cmd)
{
    gchar *argv[] = {(gchar *)cmd, NULL};
    g_spawn_async(NULL, argv, NULL, G_SPAWN_SEARCH_PATH | G_SPAWN_STDOUT_TO_DEV_NULL | G_SPAWN_STDERR_TO_DEV_NULL, own_session, NULL,
                  NULL, NULL);
}

/* Bring the app forward. Already in front with several windows: the one
 * longest behind, so clicks cycle through them. */
static void focus_item(int i)
{
    const struct wm_win *recent = NULL, *oldest = NULL;
    int front = 0, count = 0;
    for (int w = 0; w < O.nwins; w++) {
        if (O.win_item[w] != i) continue;
        const struct wm_win *x = &O.wins[w];
        count++;
        front |= x->focused;
        if (!recent || x->rank < recent->rank) recent = x;
        if (!oldest || x->rank >= oldest->rank) oldest = x;
    }
    if (recent) wm_focus(front && count > 1 ? oldest : recent);
    refresh_soon();
}

static void activate(int i)
{
    if (i < 0 || i >= O.n) return;
    if (i == 0) spawn_simple("nexa-nexus");
    else if (O.it[i].running) focus_item(i);
    else launch(&O.it[i]);
    gtk_widget_queue_draw(O.area);
}

/* ---------- layout ---------- */

static double rail_h(void)
{
    double h = 2 * (POINT + END_PAD) + O.n * CELL;
    if (O.n - 1 > O.npinned_shown) h += DIVIDER;
    return h;
}

static double item_y(int i)
{
    double y = 3 + POINT + END_PAD + i * CELL + CELL / 2;
    if (i > O.npinned_shown) y += DIVIDER;
    return y;
}

static int item_at(double x, double y)
{
    if (x > EDGE + RAIL_W) return -1;
    for (int i = 0; i < O.n; i++)
        if (fabs(y - item_y(i)) <= CELL / 2) return i;
    return -1;
}

static void set_input_region(void)
{
    if (!gtk_widget_get_realized(O.win)) return;
    cairo_rectangle_int_t r = {0, 0, (int)(EDGE + RAIL_W), O.height};
    if (O.conf.fold && O.fold_t > 0.5) r.width = FOLD_HIT;
    cairo_region_t *reg = cairo_region_create_rectangle(&r);
    gtk_widget_input_shape_combine_region(O.win, reg);
    cairo_region_destroy(reg);
}

static void on_realize(GtkWidget *w, gpointer d) { set_input_region(); }
static void on_size(GtkWidget *w, GdkRectangle *a, gpointer d) { set_input_region(); }

static void relayout(void)
{
    O.width = (int)(EDGE + RAIL_W) + LABEL_ROOM;
    O.height = (int)ceil(rail_h()) + 6;
    gtk_widget_set_size_request(O.area, O.width, O.height);
    gtk_window_resize(GTK_WINDOW(O.win), O.width, O.height);
    gtk_layer_set_exclusive_zone(GTK_WINDOW(O.win), O.conf.fold ? 0 : (int)(EDGE + RAIL_W + GAP));
    set_input_region();
    write_state();
}

/* ---------- drawing ---------- */

static void hex_point(double cx, double cy, double r, int k, double *x, double *y)
{
    double a = (-90 + 60 * (k % 6)) * G_PI / 180;
    *x = cx + r * cos(a);
    *y = cy + r * sin(a);
}

static void hex_path(cairo_t *cr, double cx, double cy, double r)
{
    for (int k = 0; k < 6; k++) {
        double x, y;
        hex_point(cx, cy, r, k, &x, &y);
        if (k) cairo_line_to(cr, x, y);
        else cairo_move_to(cr, x, y);
    }
    cairo_close_path(cr);
}

/* The part of the frame from t0 to t1 (fractions of its perimeter,
 * clockwise from the top point; t1 may pass 1 and wrap). */
static void hex_arc(cairo_t *cr, double cx, double cy, double r, double t0, double t1)
{
    double x, y, x2, y2;
    for (double t = t0;;) {
        double tw = fmod(t, 1.0) * 6;
        int k = (int)tw;
        double f = tw - k;
        hex_point(cx, cy, r, k, &x, &y);
        hex_point(cx, cy, r, k + 1, &x2, &y2);
        double px = x + (x2 - x) * f, py = y + (y2 - y) * f;
        if (t == t0) cairo_move_to(cr, px, py);
        else cairo_line_to(cr, px, py);
        if (t >= t1) break;
        double next = (floor(t * 6 + 1e-9) + 1) / 6;
        t = next < t1 ? next : t1;
    }
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

/* The rail: one tall Hexa cell, pointed at both ends. */
static void rail_path(cairo_t *cr, double x, double y, double w, double h)
{
    cairo_move_to(cr, x + w / 2, y);
    cairo_line_to(cr, x + w, y + POINT);
    cairo_line_to(cr, x + w, y + h - POINT);
    cairo_line_to(cr, x + w / 2, y + h);
    cairo_line_to(cr, x, y + h - POINT);
    cairo_line_to(cr, x, y + POINT);
    cairo_close_path(cr);
}

/* The NexaOS mark: hexagon outline, ring and core dot. */
static void draw_mark(cairo_t *cr, double cx, double cy, double r, int hot)
{
    set_rgb(cr, hot ? mix(O.c.accent, O.c.text, 0.35) : O.c.accent, 1);
    cairo_set_line_width(cr, fmax(2.0, r * 0.09));
    hex_path(cr, cx, cy, r * 0.9);
    cairo_stroke(cr);
    cairo_set_line_width(cr, fmax(1.0, r * 0.05));
    cairo_arc(cr, cx, cy, r * 0.38, 0, 2 * G_PI);
    cairo_stroke(cr);
    cairo_arc(cr, cx, cy, r * 0.2, 0, 2 * G_PI);
    cairo_fill(cr);
}

static void draw_label(cairo_t *cr, const char *text, double x, double cy)
{
    PangoLayout *l = pango_cairo_create_layout(cr);
    PangoFontDescription *fd = pango_font_description_from_string("Inter 10");
    pango_layout_set_font_description(l, fd);
    pango_font_description_free(fd);
    pango_layout_set_text(l, text, -1);
    pango_layout_set_width(l, (LABEL_ROOM - 40) * PANGO_SCALE);
    pango_layout_set_ellipsize(l, PANGO_ELLIPSIZE_END);
    int tw, th;
    pango_layout_get_pixel_size(l, &tw, &th);
    double bw = tw + 22, bh = th + 10;
    /* a small hexagon point toward the rail, then the capsule */
    set_rgb(cr, O.c.surface, 0.96);
    cairo_move_to(cr, x - 6, cy);
    cairo_line_to(cr, x + 2, cy - 6);
    cairo_line_to(cr, x + 2, cy + 6);
    cairo_close_path(cr);
    cairo_fill(cr);
    rounded(cr, x, cy - bh / 2, bw, bh, bh / 2);
    cairo_fill_preserve(cr);
    set_rgb(cr, O.c.accent, 0.35);
    cairo_set_line_width(cr, 1);
    cairo_stroke(cr);
    set_rgb(cr, O.c.text, 1);
    cairo_move_to(cr, x + 11, cy - th / 2.0);
    pango_cairo_show_layout(cr, l);
    g_object_unref(l);
}

static void draw_item(cairo_t *cr, int i, double cx, double cy, double t)
{
    struct item *it = &O.it[i];
    int hot = i == O.hover || i == O.kfocus;
    if (i == 0) {
        draw_mark(cr, cx, cy, HEX_R * 0.78, hot);
    } else {
        /* the frame */
        hex_path(cr, cx, cy, HEX_R);
        if (it->focused) set_rgb(cr, O.c.accent, hot ? 0.36 : 0.26);
        else if (hot) set_rgb(cr, O.c.accent, 0.2);
        else set_rgb(cr, O.c.surface, 0.55);
        cairo_fill(cr);
        if (it->pix) {
            int scale = gtk_widget_get_scale_factor(O.area);
            cairo_save(cr);
            cairo_translate(cr, cx - ICON / 2.0, cy - ICON / 2.0);
            cairo_scale(cr, 1.0 / scale, 1.0 / scale);
            gdk_cairo_set_source_pixbuf(cr, it->pix, 0, 0);
            cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_GOOD);
            cairo_paint_with_alpha(cr, it->minimized ? 0.7 : 1.0);
            cairo_restore(cr);
        }
        cairo_set_line_join(cr, CAIRO_LINE_JOIN_ROUND);
        cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
        if (it->urgent) {
            double dash[] = {6, 4};
            cairo_set_dash(cr, dash, 2, O.animations ? -fmod(t * 12, 10) : 0);
            hex_path(cr, cx, cy, HEX_R + 1);
            set_rgb(cr, O.c.warn, 0.95);
            cairo_set_line_width(cr, 2);
            cairo_stroke(cr);
            cairo_set_dash(cr, NULL, 0, 0);
        } else if (it->running) {
            hex_path(cr, cx, cy, HEX_R);
            set_rgb(cr, O.c.accent, it->minimized ? 0.3 : 0.6);
            cairo_set_line_width(cr, 1.5);
            cairo_stroke(cr);
            double busy = fmin(1.0, it->cpu_shown / 100.0);
            if (busy > 0.02) {
                hex_arc(cr, cx, cy, HEX_R, 0, fmax(busy, 0.06));
                set_rgb(cr, mix(O.c.accent, O.c.text, 0.45), 1);
                cairo_set_line_width(cr, 2.6);
                cairo_stroke(cr);
            }
        } else if (it->launched) {
            if (O.animations) {
                double s = fmod(t * 0.9, 1.0);
                hex_arc(cr, cx, cy, HEX_R, s, s + 0.22);
            } else {
                double dash[] = {3, 5};
                cairo_set_dash(cr, dash, 2, 0);
                hex_path(cr, cx, cy, HEX_R);
            }
            set_rgb(cr, mix(O.c.accent, O.c.text, 0.3), 1);
            cairo_set_line_width(cr, 2.2);
            cairo_stroke(cr);
            cairo_set_dash(cr, NULL, 0, 0);
        }
    }
    if (i == O.kfocus) {  /* keyboard focus: a second, outer outline */
        hex_path(cr, cx, cy, HEX_R + 4.5);
        set_rgb(cr, O.c.text, 0.9);
        cairo_set_line_width(cr, 2);
        cairo_stroke(cr);
    }
}

static gboolean draw(GtkWidget *w, cairo_t *cr, gpointer d)
{
    double t = now_s(), H = rail_h();
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    cairo_set_source_rgba(cr, 0, 0, 0, 0);
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);

    /* folded: a line of light on the edge, the rail slid out of sight */
    if (O.fold_t > 0.001) {
        cairo_pattern_t *g = cairo_pattern_create_linear(0, 3, 0, 3 + H);
        cairo_pattern_add_color_stop_rgba(g, 0, O.c.accent.r, O.c.accent.g, O.c.accent.b, 0);
        cairo_pattern_add_color_stop_rgba(g, 0.5, O.c.accent.r, O.c.accent.g, O.c.accent.b, 0.85 * O.fold_t);
        cairo_pattern_add_color_stop_rgba(g, 1, O.c.accent2.r, O.c.accent2.g, O.c.accent2.b, 0);
        rounded(cr, 0, 3 + H * 0.15, FOLD_LINE, H * 0.7, FOLD_LINE / 2);
        cairo_set_source(cr, g);
        cairo_fill(cr);
        cairo_pattern_destroy(g);
    }
    if (O.fold_t > 0.999) return TRUE;
    cairo_save(cr);
    double slide = (EDGE + RAIL_W + 4) * O.fold_t;
    cairo_translate(cr, -slide, 0);
    cairo_push_group(cr);

    rail_path(cr, EDGE, 3, RAIL_W, H);
    set_rgb(cr, O.c.bg, fmax(O.c.opacity, 0.6));
    cairo_fill_preserve(cr);
    set_rgb(cr, O.c.accent, 0.35);
    cairo_set_line_width(cr, 1);
    cairo_set_line_join(cr, CAIRO_LINE_JOIN_ROUND);
    cairo_stroke(cr);

    double cx = EDGE + RAIL_W / 2;
    for (int i = 0; i < O.n; i++) draw_item(cr, i, cx, item_y(i), t);
    if (O.n - 1 > O.npinned_shown) {
        double y = item_y(O.npinned_shown) + CELL / 2 + DIVIDER / 2;
        set_rgb(cr, O.c.muted, 0.5);
        cairo_set_line_width(cr, 1);
        cairo_move_to(cr, cx - 12, y);
        cairo_line_to(cr, cx + 12, y);
        cairo_stroke(cr);
    }
    int show = O.kfocus >= 0 ? O.kfocus : O.hover;
    if (show >= 0 && show < O.n && O.fold_t < 0.01) {
        char text[200];
        const struct item *it = &O.it[show];
        if (it->urgent) snprintf(text, sizeof text, "%s: wants you", it->name);
        else if (it->launched) snprintf(text, sizeof text, "%s: starting", it->name);
        else if (it->running && it->nwin > 1) snprintf(text, sizeof text, "%s: %d windows", it->name, it->nwin);
        else snprintf(text, sizeof text, "%s", it->name);
        draw_label(cr, text, EDGE + RAIL_W + 10, item_y(show));
    }
    cairo_pop_group_to_source(cr);
    cairo_paint_with_alpha(cr, 1 - O.fold_t);
    cairo_restore(cr);
    return TRUE;
}

/* ---------- animation ---------- */

static int animating(void)
{
    if (fabs(O.fold_t - O.fold_target) > 0.001) return 2;
    if (O.fold_t > 0.99) return 0;
    int moving = 0;
    for (int i = 1; i < O.n; i++) {
        if (O.animations && (O.it[i].launched || O.it[i].urgent)) moving = 1;
        if (fabs(O.it[i].cpu_shown - O.it[i].cpu) > 0.5) moving = 1;
    }
    return moving;
}

static gboolean anim_tick(gpointer d)
{
    int was = animating();
    if (!O.animations) O.fold_t = O.fold_target;
    else if (O.fold_t < O.fold_target) O.fold_t = fmin(O.fold_target, O.fold_t + 0.09);
    else if (O.fold_t > O.fold_target) O.fold_t = fmax(O.fold_target, O.fold_t - 0.09);
    for (int i = 1; i < O.n; i++) O.it[i].cpu_shown += (O.it[i].cpu - O.it[i].cpu_shown) * 0.18;
    set_input_region();
    gtk_widget_queue_draw(O.area);
    int now = animating();
    if (!now) {
        O.anim_id = 0;
        return G_SOURCE_REMOVE;
    }
    if (now != was) {  /* frame rate changes: fold at 60, the rest at 20 */
        O.anim_id = g_timeout_add(now == 2 ? 16 : 50, anim_tick, NULL);
        return G_SOURCE_REMOVE;
    }
    return G_SOURCE_CONTINUE;
}

static void start_anim(void)
{
    int a = animating();
    if (!O.anim_id && a) O.anim_id = g_timeout_add(a == 2 ? 16 : 50, anim_tick, NULL);
}

static void set_folded(int folded)
{
    O.fold_target = O.conf.fold && folded ? 1 : 0;
    start_anim();
    write_state();
}

static gboolean fold_later(gpointer d)
{
    O.fold_id = 0;
    if (!O.pointer_in && !O.menu_open && !O.keyboard) set_folded(1);
    return G_SOURCE_REMOVE;
}

/* ---------- state for tests ---------- */

static void write_state(void)
{
    const char *path = g_getenv("NEXA_ORBIT_STATE");
    if (!path) return;
    GString *s = g_string_new(NULL);
    g_string_append_printf(s, "{\"surface\": [%d, %d], \"folded\": %d, \"keyboard\": %d, \"kfocus\": %d, \"hover\": %d, "
                           "\"accent\": \"#%02x%02x%02x\", \"animations\": %d, \"exclusive\": %d}\n",
                           O.width, O.height, O.fold_target > 0.5, O.keyboard, O.kfocus, O.hover, (int)lround(O.c.accent.r * 255),
                           (int)lround(O.c.accent.g * 255), (int)lround(O.c.accent.b * 255), O.animations,
                           gtk_layer_get_exclusive_zone(GTK_WINDOW(O.win)));
    for (int i = 0; i < O.n; i++) {
        const struct item *it = &O.it[i];
        g_string_append_printf(s, "{\"id\": \"%s\", \"y\": %.0f, \"pinned\": %d, \"running\": %d, \"focused\": %d, \"urgent\": %d, "
                               "\"minimized\": %d, \"windows\": %d, \"cpu\": %.1f, \"starting\": %d, \"icon\": %d}\n",
                               i ? it->id : "nexus", item_y(i), it->pinned, it->running, it->focused, it->urgent, it->minimized, it->nwin,
                               it->cpu, it->launched != 0, it->pix != NULL);
    }
    char *tmp = g_strconcat(path, ".tmp", NULL);
    if (g_file_set_contents(tmp, s->str, (gssize)s->len, NULL)) rename(tmp, path);
    g_free(tmp);
    g_string_free(s, TRUE);
}

/* ---------- config ---------- */

static void load_conf(void)
{
    char *text = NULL;
    g_file_get_contents(O.conf_path, &text, NULL, NULL);
    conf_parse(&O.conf, text);
    g_free(text);
}

static void save_conf(void)
{
    char buf[ORBIT_MAX_PINNED * ORBIT_ID_LEN + 256];
    if (conf_format(&O.conf, buf, sizeof buf) < 0) return;
    char *dir = g_path_get_dirname(O.conf_path);
    g_mkdir_with_parents(dir, 0700);
    g_free(dir);
    g_file_set_contents(O.conf_path, buf, -1, NULL);  /* atomic: written beside it, renamed */
}

static void file_changed(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent ev, gpointer which)
{
    if (ev != G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT && ev != G_FILE_MONITOR_EVENT_CREATED && ev != G_FILE_MONITOR_EVENT_DELETED &&
        ev != G_FILE_MONITOR_EVENT_RENAMED && ev != G_FILE_MONITOR_EVENT_MOVED_IN)
        return;
    if (GPOINTER_TO_INT(which) == 1) {
        load_theme();
        load_icons();
        write_state();
    } else {
        int fold = O.conf.fold;
        load_conf();
        if (fold != O.conf.fold) set_folded(!O.pointer_in);
        refresh();
        relayout();
    }
}

static GFileMonitor *watch_file(const char *path, int which)
{
    /* the folder: the files are replaced by rename */
    char *dir = g_path_get_dirname(path);
    g_mkdir_with_parents(dir, 0700);
    GFile *f = g_file_new_for_path(path);
    GFileMonitor *m = g_file_monitor_file(f, G_FILE_MONITOR_WATCH_MOVES, NULL, NULL);
    if (m) g_signal_connect(m, "changed", G_CALLBACK(file_changed), GINT_TO_POINTER(which));
    g_object_unref(f);
    g_free(dir);
    return m;
}

/* ---------- menu ---------- */

/* The menu's app by id: windows may come and go while it is open. */
static char menu_id[ORBIT_ID_LEN];

static void on_menu_open(GtkMenuItem *mi, gpointer d)
{
    int i = find_item(menu_id);
    if (i > 0) launch(&O.it[i]);
}
static void on_menu_pin(GtkMenuItem *mi, gpointer d)
{
    int i = find_item(menu_id);
    if (i <= 0) return;
    struct item *it = &O.it[i];
    if (it->pinned) conf_unpin(&O.conf, it->id);
    else conf_pin(&O.conf, it->id);
    save_conf();
    refresh();
}
static void on_menu_close(GtkMenuItem *mi, gpointer d)
{
    int i = find_item(menu_id);
    for (int w = 0; w < O.nwins && i > 0; w++)
        if (O.win_item[w] == i) wm_close(&O.wins[w]);
    refresh_soon();
}

static void give_keyboard_back(void);

static gboolean destroy_later(gpointer w)
{
    gtk_widget_destroy(w);
    return G_SOURCE_REMOVE;
}

static void menu_done(GtkMenuShell *m, gpointer d)
{
    /* after the chosen item has run */
    g_idle_add(destroy_later, m);
    O.menu_open = 0;
    if (O.keyboard) give_keyboard_back();
    if (!O.pointer_in && O.conf.fold && !O.fold_id) O.fold_id = g_timeout_add(700, fold_later, NULL);
}

static void add_menu_item(GtkWidget *menu, const char *label, GCallback cb)
{
    GtkWidget *mi = gtk_menu_item_new_with_label(label);
    g_signal_connect(mi, "activate", cb, NULL);
    gtk_menu_shell_append(GTK_MENU_SHELL(menu), mi);
}

static void show_menu(int i, GdkEvent *ev)
{
    if (i <= 0 || i >= O.n) return;
    struct item *it = &O.it[i];
    snprintf(menu_id, sizeof menu_id, "%s", it->id);
    GtkWidget *menu = gtk_menu_new();
    if (it->info) add_menu_item(menu, it->running ? "New window" : "Open", G_CALLBACK(on_menu_open));
    if (it->info) add_menu_item(menu, it->pinned ? "Unpin from Orbit" : "Pin to Orbit", G_CALLBACK(on_menu_pin));
    if (it->running) add_menu_item(menu, it->nwin > 1 ? "Close all windows" : "Close window", G_CALLBACK(on_menu_close));
    gtk_widget_show_all(menu);
    g_signal_connect(menu, "deactivate", G_CALLBACK(menu_done), NULL);
    gtk_menu_attach_to_widget(GTK_MENU(menu), O.area, NULL);
    O.menu_open = 1;
    GdkRectangle r = {(int)(EDGE + RAIL_W - 4), (int)item_y(i) - 1, 2, 2};
    gtk_menu_popup_at_rect(GTK_MENU(menu), gtk_widget_get_window(O.area), &r, GDK_GRAVITY_EAST, GDK_GRAVITY_WEST, ev);
    if (!ev) gtk_menu_shell_select_first(GTK_MENU_SHELL(menu), FALSE);
}

/* ---------- pointer ---------- */

static gboolean on_motion(GtkWidget *w, GdkEventMotion *e, gpointer d)
{
    int h = O.fold_t > 0.5 ? -1 : item_at(e->x, e->y);
    if (h != O.hover) {
        O.hover = h;
        gtk_widget_queue_draw(O.area);
        write_state();
    }
    return FALSE;
}

static gboolean on_enter(GtkWidget *w, GdkEventCrossing *e, gpointer d)
{
    O.pointer_in = 1;
    if (O.fold_id) g_source_remove(O.fold_id), O.fold_id = 0;
    set_folded(0);
    /* a pointer that arrives and stays still sends no motion */
    int h = O.fold_t > 0.5 ? -1 : item_at(e->x, e->y);
    if (h != O.hover) {
        O.hover = h;
        gtk_widget_queue_draw(O.area);
        write_state();
    }
    return FALSE;
}

static gboolean on_leave(GtkWidget *w, GdkEventCrossing *e, gpointer d)
{
    if (e->detail == GDK_NOTIFY_INFERIOR) return FALSE;
    O.pointer_in = 0;
    O.hover = -1;
    gtk_widget_queue_draw(O.area);
    write_state();
    if (O.conf.fold && !O.menu_open && !O.fold_id) O.fold_id = g_timeout_add(700, fold_later, NULL);
    return FALSE;
}

static gboolean on_button(GtkWidget *w, GdkEventButton *e, gpointer d)
{
    if (e->type != GDK_BUTTON_PRESS) return FALSE;
    if (O.fold_t > 0.5) {  /* the folded line: a click opens it too */
        set_folded(0);
        return TRUE;
    }
    int i = item_at(e->x, e->y);
    if (i < 0) return FALSE;
    if (e->button == 1) activate(i);
    else if (e->button == 2 && i > 0) launch(&O.it[i]);
    else if (e->button == 3) show_menu(i, (GdkEvent *)e);
    return TRUE;
}

/* ---------- keyboard (Super+O) ---------- */

static gboolean key_idle(gpointer d)
{
    O.key_idle_id = 0;
    give_keyboard_back();
    return G_SOURCE_REMOVE;
}

static void key_activity(void)
{
    if (O.key_idle_id) g_source_remove(O.key_idle_id);
    O.key_idle_id = g_timeout_add_seconds(KEY_IDLE_S, key_idle, NULL);
}

static void take_keyboard(void)
{
    O.keyboard = 1;
    O.kfocus = O.n > 1 ? 1 : 0;
    gtk_layer_set_keyboard_mode(GTK_WINDOW(O.win), GTK_LAYER_SHELL_KEYBOARD_MODE_EXCLUSIVE);
    set_folded(0);
    key_activity();
    gtk_widget_queue_draw(O.area);
    write_state();
}

static void give_keyboard_back(void)
{
    if (!O.keyboard) return;
    O.keyboard = 0;
    O.kfocus = -1;
    if (O.key_idle_id) g_source_remove(O.key_idle_id), O.key_idle_id = 0;
    gtk_layer_set_keyboard_mode(GTK_WINDOW(O.win), GTK_LAYER_SHELL_KEYBOARD_MODE_NONE);
    if (!O.pointer_in) set_folded(1);
    gtk_widget_queue_draw(O.area);
    write_state();
}

static gboolean on_usr1(gpointer d)
{
    if (O.keyboard) give_keyboard_back();
    else take_keyboard();
    return G_SOURCE_CONTINUE;
}

static gboolean on_key(GtkWidget *w, GdkEventKey *e, gpointer d)
{
    if (!O.keyboard) return FALSE;
    key_activity();
    guint k = e->keyval;
    int shift = (e->state & GDK_SHIFT_MASK) != 0;
    if (k == GDK_KEY_Escape) {
        give_keyboard_back();
    } else if (k == GDK_KEY_Down || (k == GDK_KEY_Tab && !shift)) {
        O.kfocus = (O.kfocus + 1) % O.n;
    } else if (k == GDK_KEY_Up || k == GDK_KEY_ISO_Left_Tab || (k == GDK_KEY_Tab && shift)) {
        O.kfocus = (O.kfocus + O.n - 1) % O.n;
    } else if (k == GDK_KEY_Home) {
        O.kfocus = 0;
    } else if (k == GDK_KEY_End) {
        O.kfocus = O.n - 1;
    } else if (k == GDK_KEY_Return || k == GDK_KEY_KP_Enter || k == GDK_KEY_space) {
        int i = O.kfocus;
        give_keyboard_back();
        activate(i);
    } else if (k == GDK_KEY_Menu || (k == GDK_KEY_F10 && shift)) {
        show_menu(O.kfocus, NULL);
    } else {
        return FALSE;
    }
    gtk_widget_queue_draw(O.area);
    write_state();
    return TRUE;
}

/* ---------- compositor events ---------- */

static gboolean on_wm_event(gint fd, GIOCondition c, gpointer d)
{
    if ((c & (G_IO_HUP | G_IO_ERR)) || wm_event_read(fd) < 0) {
        close(fd);
        O.ev_fd = -1;  /* every_2s polls from now on */
        O.ev_watch = 0;
        return G_SOURCE_REMOVE;
    }
    refresh_soon();
    return G_SOURCE_CONTINUE;
}

static void connect_wm(void)
{
    O.ev_fd = wm_event_fd();
    if (O.ev_fd >= 0) O.ev_watch = g_unix_fd_add(O.ev_fd, G_IO_IN | G_IO_HUP | G_IO_ERR, on_wm_event, NULL);
}

/* ---------- start ---------- */

static void config_path(char *out, size_t cap, const char *name)
{
    const char *cfg = g_getenv("XDG_CONFIG_HOME");
    if (cfg && *cfg) snprintf(out, cap, "%s/nexa/%s", cfg, name);
    else snprintf(out, cap, "%s/.config/nexa/%s", g_get_home_dir(), name);
}

static void on_icons_changed(GtkIconTheme *t, gpointer d) { load_icons(); }
static void on_apps_changed(GAppInfoMonitor *m, gpointer d)
{
    index_wmclass();
    refresh();
}
static void on_anim_setting(GObject *o, GParamSpec *p, gpointer d)
{
    read_animations();
    start_anim();
    write_state();
}

int main(int argc, char **argv)
{
    if (argc > 1) {
        fprintf(stderr, "usage: nexa-orbit   (Super+O: pkill -USR1 -x nexa-orbit)\n");
        return 2;
    }
    signal(SIGPIPE, SIG_IGN);
    gtk_init(&argc, &argv);
    if (!gtk_layer_is_supported()) {
        fprintf(stderr, "nexa-orbit: this session has no layer-shell (needs Sway or Hyprland)\n");
        return 1;
    }
    config_path(O.conf_path, sizeof O.conf_path, "orbit.conf");
    config_path(O.theme_path, sizeof O.theme_path, "lock.conf");
    load_conf();
    load_theme();
    read_animations();
    index_wmclass();

    O.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(O.win), "Orbit");
    GdkVisual *v = gdk_screen_get_rgba_visual(gtk_widget_get_screen(O.win));
    if (v) gtk_widget_set_visual(O.win, v);
    gtk_widget_set_app_paintable(O.win, TRUE);
    gtk_layer_init_for_window(GTK_WINDOW(O.win));
    gtk_layer_set_namespace(GTK_WINDOW(O.win), "nexa-orbit");  /* Hyprland blurs nexa-* layers */
    gtk_layer_set_layer(GTK_WINDOW(O.win), GTK_LAYER_SHELL_LAYER_TOP);
    gtk_layer_set_anchor(GTK_WINDOW(O.win), GTK_LAYER_SHELL_EDGE_LEFT, TRUE);  /* centred top to bottom */
    gtk_layer_set_keyboard_mode(GTK_WINDOW(O.win), GTK_LAYER_SHELL_KEYBOARD_MODE_NONE);

    O.area = gtk_drawing_area_new();
    gtk_widget_add_events(O.area, GDK_POINTER_MOTION_MASK | GDK_ENTER_NOTIFY_MASK | GDK_LEAVE_NOTIFY_MASK | GDK_BUTTON_PRESS_MASK);
    g_signal_connect(O.area, "draw", G_CALLBACK(draw), NULL);
    g_signal_connect(O.area, "motion-notify-event", G_CALLBACK(on_motion), NULL);
    g_signal_connect(O.area, "enter-notify-event", G_CALLBACK(on_enter), NULL);
    g_signal_connect(O.area, "leave-notify-event", G_CALLBACK(on_leave), NULL);
    g_signal_connect(O.area, "button-press-event", G_CALLBACK(on_button), NULL);
    g_signal_connect(O.win, "key-press-event", G_CALLBACK(on_key), NULL);
    g_signal_connect(O.win, "realize", G_CALLBACK(on_realize), NULL);
    g_signal_connect(O.area, "size-allocate", G_CALLBACK(on_size), NULL);
    g_signal_connect(O.win, "destroy", G_CALLBACK(gtk_main_quit), NULL);
    gtk_container_add(GTK_CONTAINER(O.win), O.area);

    O.fold_t = O.fold_target = O.conf.fold ? 1 : 0;
    connect_wm();
    refresh();
    relayout();
    gtk_widget_show_all(O.win);

    GFileMonitor *m1 = watch_file(O.theme_path, 1), *m2 = watch_file(O.conf_path, 2);
    g_signal_connect(gtk_icon_theme_get_default(), "changed", G_CALLBACK(on_icons_changed), NULL);
    g_signal_connect(gtk_settings_get_default(), "notify::gtk-enable-animations", G_CALLBACK(on_anim_setting), NULL);
    g_signal_connect(g_app_info_monitor_get(), "changed", G_CALLBACK(on_apps_changed), NULL);
    g_unix_signal_add(SIGUSR1, on_usr1, NULL);
    g_timeout_add_seconds(2, every_2s, NULL);
    gtk_main();
    if (m1) g_object_unref(m1);
    if (m2) g_object_unref(m2);
    return 0;
}
