#include "sky.h"

#include <gtk-layer-shell.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "chart.h"
#include "json.h"

#define STAR_R 24.0        /* a star's hexagon, centre to point, at scale 1 */
#define ICON_PX 30.0
#define TITLE_W 170.0      /* px a title may take */
#define FLIGHT 0.2         /* s: stars fly out from the middle */
#define STARMAP_WAIT 3     /* s: then the graph program is stopped */
#define DUST 140

struct project { unsigned long id; char path[512], name[128], place[256]; };

static struct {
    GtkWidget *win, *area;
    struct sky_hooks hooks;
    struct sky_colours c;
    int animations;
    struct sky_win w[CHART_MAX];
    int n, current;
    unsigned long mru[CHART_MAX];  /* ids, used last first */
    int nmru;
    /* derived, per window */
    int rank[CHART_MAX], group[CHART_MAX], order[CHART_MAX], on[CHART_MAX], proj[CHART_MAX];
    struct chart_pt star[CHART_MAX], centre[CHART_MAX];
    int ngroups, group_first[CHART_MAX];  /* a group's window used last */
    double scale;
    int links[CHART_MAX][2], nlinks;
    /* projects, by window id (kept across updates until the graph answers again) */
    struct project p[CHART_MAX];
    int np, projects_known;
    GSubprocess *job;
    guint job_timeout, again_id;
    int again;
    /* use */
    int sel;
    char query[128];
    double opened_at;
    guint tick, urgent_timer;
} S = {.sel = -1, .animations = 1, .c = {{0.03, 0.05, 0.12}, {0.08, 0.11, 0.23}, {0.9, 0.93, 1}, {0.56, 0.61, 0.77}, {0.3, 0.64, 1}, {1, 0.71, 0.28}, 0.78}};

static void recompute(void);
static void request_projects(void);
static void write_state(void);

static double now_s(void) { return g_get_monotonic_time() / 1e6; }
static void set_rgb(cairo_t *cr, strata_rgb c, double a) { cairo_set_source_rgba(cr, c.r, c.g, c.b, a); }
static strata_rgb mix(strata_rgb a, strata_rgb b, double t)
{
    return (strata_rgb){a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t};
}
static double luma(strata_rgb c) { return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b; }

void sky_init(const struct sky_hooks *h) { S.hooks = *h; }
void sky_set_colours(const struct sky_colours *c)
{
    S.c = *c;
    if (S.area) gtk_widget_queue_draw(S.area);
}
void sky_set_animations(int on) { S.animations = on; }
int sky_is_open(void) { return S.win != NULL; }

static int find_win(unsigned long id)
{
    for (int i = 0; i < S.n; i++)
        if (S.w[i].id == id) return i;
    return -1;
}

static struct project *project_of(unsigned long id)
{
    for (int i = 0; i < S.np; i++)
        if (S.p[i].id == id) return &S.p[i];
    return NULL;
}

/* ---------- windows ---------- */

static void note_use(const struct sky_win *w, int n)
{
    unsigned long keep[CHART_MAX];
    int nk = 0;
    for (int i = 0; i < n; i++)  /* the one in front first */
        if (w[i].focused) keep[nk++] = w[i].id;
    for (int m = 0; m < S.nmru && nk < CHART_MAX; m++) {
        int there = 0, dup = 0;
        for (int i = 0; i < n; i++) there |= w[i].id == S.mru[m];
        for (int k = 0; k < nk; k++) dup |= keep[k] == S.mru[m];
        if (there && !dup) keep[nk++] = S.mru[m];
    }
    for (int i = 0; i < n && nk < CHART_MAX; i++) {  /* new ones, never in front yet */
        int dup = 0;
        for (int k = 0; k < nk; k++) dup |= keep[k] == w[i].id;
        if (!dup) keep[nk++] = w[i].id;
    }
    memcpy(S.mru, keep, sizeof keep);
    S.nmru = nk;
}

void sky_set_windows(const struct sky_win *w, int n, int current)
{
    if (n > CHART_MAX) n = CHART_MAX;
    if (n < 0) n = 0;
    unsigned long sel_id = S.sel >= 0 && S.sel < S.n ? S.w[S.sel].id : 0;
    int changed = n != S.n;
    for (int i = 0; i < n && !changed; i++)
        changed = w[i].id != S.w[i].id || w[i].pid != S.w[i].pid || strcmp(w[i].title, S.w[i].title);
    for (int i = 0; i < S.n; i++)
        if (S.w[i].icon) g_object_unref(S.w[i].icon);
    memcpy(S.w, w, sizeof *w * (size_t)n);
    for (int i = 0; i < n; i++)
        if (S.w[i].icon) g_object_ref(S.w[i].icon);
    S.n = n;
    S.current = current;
    note_use(w, n);
    if (!S.win) return;
    recompute();
    S.sel = sel_id ? find_win(sel_id) : -1;
    if (S.sel < 0 || !S.on[S.sel]) {  /* the window used last that matches */
        S.sel = -1;
        for (int i = 0; i < S.n; i++)
            if (S.on[i] && (S.sel < 0 || S.rank[i] < S.rank[S.sel])) S.sel = i;
    }
    if (changed) request_projects();
    gtk_widget_queue_draw(S.area);
    write_state();
}

static void recompute(void)
{
    int W = S.area ? gtk_widget_get_allocated_width(S.area) : 1280, H = S.area ? gtk_widget_get_allocated_height(S.area) : 800;
    if (W < 100 || H < 100) W = 1280, H = 800;
    for (int i = 0; i < S.n; i++) {
        S.rank[i] = CHART_MAX;
        for (int m = 0; m < S.nmru; m++)
            if (S.mru[m] == S.w[i].id) {
                S.rank[i] = m;
                break;
            }
    }
    /* by last use: groups in the order their apps were used, windows within */
    int idx[CHART_MAX];
    for (int i = 0; i < S.n; i++) idx[i] = i;
    for (int a = 1; a < S.n; a++)
        for (int b = a; b > 0 && S.rank[idx[b]] < S.rank[idx[b - 1]]; b--) {
            int t = idx[b];
            idx[b] = idx[b - 1];
            idx[b - 1] = t;
        }
    int key[CHART_MAX], count[CHART_MAX];
    S.ngroups = 0;
    for (int k = 0; k < S.n; k++) {
        int i = idx[k], g = -1;
        for (int j = 0; j < S.ngroups; j++)
            if (key[j] == S.w[i].group) g = j;
        if (g < 0) {
            g = S.ngroups++;
            key[g] = S.w[i].group;
            count[g] = 0;
            S.group_first[g] = i;
        }
        S.group[i] = g;
        S.order[i] = count[g]++;
    }
    S.scale = chart_layout(S.group, S.order, S.n, S.ngroups, W, H, S.star, S.centre);
    for (int i = 0; i < S.n; i++) S.on[i] = chart_match(S.w[i].title, S.w[i].app, S.query);
    /* projects: the shortest tree through each one's windows */
    S.nlinks = 0;
    int done[CHART_MAX] = {0};
    for (int i = 0; i < S.n; i++) {
        struct project *p = project_of(S.w[i].id);
        S.proj[i] = p ? (int)(p - S.p) : -1;
    }
    for (int i = 0; i < S.n; i++) {
        if (done[i] || S.proj[i] < 0) continue;
        int mem[CHART_MAX], m = 0;
        for (int j = i; j < S.n; j++)
            if (S.proj[j] >= 0 && !strcmp(S.p[S.proj[j]].path, S.p[S.proj[i]].path)) {
                mem[m++] = j;
                done[j] = 1;
            }
        S.nlinks += chart_tree(S.star, mem, m, S.links + S.nlinks);
    }
}

/* ---------- projects (nexa-starmap) ---------- */

static void clean_field(GString *s, const char *t)
{
    for (; t && *t; t++) g_string_append_c(s, *t == '\t' || *t == '\n' || *t == '\r' ? ' ' : *t);
}

static gboolean starmap_too_slow(gpointer d)
{
    S.job_timeout = 0;
    if (S.job) g_subprocess_force_exit(S.job);
    return G_SOURCE_REMOVE;
}

static gboolean run_again(gpointer d)
{
    S.again_id = 0;
    request_projects();
    return G_SOURCE_REMOVE;
}

static void starmap_done(GObject *src, GAsyncResult *res, gpointer d)
{
    GSubprocess *job = G_SUBPROCESS(src);
    char *out = NULL;
    GError *e = NULL;
    if (g_subprocess_communicate_utf8_finish(job, res, &out, NULL, &e) && out && g_subprocess_get_if_exited(job) &&
        g_subprocess_get_exit_status(job) == 0) {
        struct jdoc doc = {0};
        if (json_parse(&doc, out) == 0) {
            int arr = json_get(&doc, 0, "windows");
            S.np = 0;
            for (int c = arr >= 0 ? doc.n[arr].child : -1; c >= 0 && S.np < CHART_MAX; c = doc.n[c].next) {
                const char *path = json_str(&doc, c, "project", NULL);
                if (!path || !*path) continue;
                struct project *p = &S.p[S.np++];
                p->id = strtoul(json_str(&doc, c, "id", "0"), NULL, 10);
                snprintf(p->path, sizeof p->path, "%s", path);
                snprintf(p->name, sizeof p->name, "%s", json_str(&doc, c, "name", ""));
                snprintf(p->place, sizeof p->place, "%s", json_str(&doc, c, "place", ""));
            }
            S.projects_known = 1;
        }
        json_free(&doc);
    } else if (e && !g_error_matches(e, G_IO_ERROR, G_IO_ERROR_CANCELLED)) {
        g_printerr("nexa-orbit: Constellation's projects: %s\n", e->message);
    }
    g_clear_error(&e);
    g_free(out);
    if (S.job == job) {
        g_clear_object(&S.job);
        if (S.job_timeout) g_source_remove(S.job_timeout);
        S.job_timeout = 0;
    }
    if (S.win) {
        recompute();
        gtk_widget_queue_draw(S.area);
        write_state();
        if (S.again && !S.again_id) S.again_id = g_timeout_add(300, run_again, NULL);
    }
    S.again = 0;
}

static void request_projects(void)
{
    if (S.job) {
        S.again = 1;
        return;
    }
    GString *in = g_string_new(NULL);
    for (int i = 0; i < S.n; i++) {
        g_string_append_printf(in, "%lu\t%d\t", S.w[i].id, S.w[i].pid);
        clean_field(in, S.w[i].app_id);
        g_string_append_c(in, '\t');
        clean_field(in, S.w[i].title);
        g_string_append_c(in, '\n');
    }
    const char *prog = g_getenv("NEXA_STARMAP");
    const char *argv[] = {prog && *prog ? prog : "nexa-starmap", NULL};
    GError *e = NULL;
    S.job = g_subprocess_newv(argv, G_SUBPROCESS_FLAGS_STDIN_PIPE | G_SUBPROCESS_FLAGS_STDOUT_PIPE | G_SUBPROCESS_FLAGS_STDERR_SILENCE, &e);
    if (!S.job) {
        g_printerr("nexa-orbit: Constellation can't start %s: %s\n", argv[0], e ? e->message : "?");
        g_clear_error(&e);
    } else {
        g_subprocess_communicate_utf8_async(S.job, in->str, NULL, starmap_done, NULL);
        S.job_timeout = g_timeout_add_seconds(STARMAP_WAIT, starmap_too_slow, NULL);
    }
    g_string_free(in, TRUE);
}

/* ---------- test state ---------- */

static void json_esc(GString *s, const char *t)
{
    g_string_append_c(s, '"');
    for (; t && *t; t++) {
        unsigned char ch = (unsigned char)*t;
        if (ch == '"' || ch == '\\') g_string_append_printf(s, "\\%c", ch);
        else if (ch < 0x20) g_string_append_printf(s, "\\u%04x", ch);
        else g_string_append_c(s, (char)ch);
    }
    g_string_append_c(s, '"');
}

static void write_state(void)
{
    const char *path = g_getenv("NEXA_CONSTELLATION_STATE");
    if (!path) return;
    GString *s = g_string_new(NULL);
    g_string_append_printf(s, "{\"open\": %d, \"current\": %d, \"scale\": %.3f, \"projects_known\": %d, \"animations\": %d, \"query\": ",
                           S.win != NULL, S.current, S.scale, S.projects_known, S.animations);
    json_esc(s, S.query);
    g_string_append(s, ", \"selected\": ");
    if (S.win && S.sel >= 0) g_string_append_printf(s, "\"%lu\"", S.w[S.sel].id);
    else g_string_append(s, "null");
    if (S.win)
        g_string_append_printf(s, ", \"size\": [%d, %d]", gtk_widget_get_allocated_width(S.area), gtk_widget_get_allocated_height(S.area));
    g_string_append(s, ", \"stars\": [");
    for (int i = 0; S.win && i < S.n; i++) {
        g_string_append_printf(s, "%s{\"id\": \"%lu\", \"app\": ", i ? ", " : "", S.w[i].id);
        json_esc(s, S.w[i].app);
        g_string_append(s, ", \"title\": ");
        json_esc(s, S.w[i].title);
        g_string_append_printf(s, ", \"x\": %.1f, \"y\": %.1f, \"group\": %d, \"rank\": %d, \"on\": %d, \"layer\": %d, \"urgent\": %d, "
                                  "\"minimized\": %d, \"icon\": %d, \"project\": ",
                               S.star[i].x, S.star[i].y, S.group[i], S.rank[i], S.on[i], S.w[i].ws, S.w[i].urgent, S.w[i].minimized,
                               S.w[i].icon != NULL);
        json_esc(s, S.proj[i] >= 0 ? S.p[S.proj[i]].name : "");
        g_string_append_c(s, '}');
    }
    g_string_append(s, "], \"links\": [");
    for (int k = 0; S.win && k < S.nlinks; k++)
        g_string_append_printf(s, "%s[\"%lu\", \"%lu\"]", k ? ", " : "", S.w[S.links[k][0]].id, S.w[S.links[k][1]].id);
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

/* Text centred on (cx, cy), at most `width` px (0: any), cut with "…". Returns its width. */
static int text_at(cairo_t *cr, const char *text, double size, int bold, double cx, double cy, double width)
{
    PangoLayout *l = pango_cairo_create_layout(cr);
    PangoFontDescription *fd = pango_font_description_from_string(bold ? "Inter Bold" : "Inter");
    pango_font_description_set_absolute_size(fd, size * PANGO_SCALE);
    pango_layout_set_font_description(l, fd);
    pango_layout_set_text(l, text, -1);
    if (width > 0) {
        pango_layout_set_width(l, (int)(width * PANGO_SCALE));
        pango_layout_set_ellipsize(l, PANGO_ELLIPSIZE_END);
    }
    int w, h;
    pango_layout_get_pixel_size(l, &w, &h);
    cairo_move_to(cr, cx - w / 2.0, cy - h / 2.0);
    pango_cairo_show_layout(cr, l);
    pango_font_description_free(fd);
    g_object_unref(l);
    return w;
}

static double flight(void)
{
    if (!S.animations) return 1;
    return strata_ease((now_s() - S.opened_at) / FLIGHT);
}

/* Where star i is drawn now (flying out from the middle while opening). */
static struct chart_pt shown(int i, double W, double H, double f)
{
    return (struct chart_pt){W / 2 + (S.star[i].x - W / 2) * f, H / 2 + (S.star[i].y - H / 2) * f};
}

static void draw_star(cairo_t *cr, int i, struct chart_pt p, double r, double a)
{
    const struct sky_win *w = &S.w[i];
    double rec = S.rank[i] < CHART_MAX ? fmax(0, 1 - S.rank[i] / 6.0) : 0;  /* 1 = used last */
    double half = w->minimized ? 0.5 : 1;
    /* glow */
    cairo_pattern_t *g = cairo_pattern_create_radial(p.x, p.y, r * 0.4, p.x, p.y, r * 2.4);
    cairo_pattern_add_color_stop_rgba(g, 0, S.c.accent.r, S.c.accent.g, S.c.accent.b, (0.1 + 0.26 * rec) * a * half);
    cairo_pattern_add_color_stop_rgba(g, 1, S.c.accent.r, S.c.accent.g, S.c.accent.b, 0);
    cairo_set_source(cr, g);
    cairo_arc(cr, p.x, p.y, r * 2.4, 0, 2 * M_PI);
    cairo_fill(cr);
    cairo_pattern_destroy(g);
    /* the hexagon */
    hex_path(cr, p.x, p.y, r);
    set_rgb(cr, w->focused ? mix(S.c.surface, S.c.accent, 0.28) : S.c.surface, 0.96 * a);
    cairo_fill_preserve(cr);
    set_rgb(cr, S.c.accent, (0.5 + 0.45 * rec) * a * half);
    cairo_set_line_width(cr, w->focused ? 2.2 : 1.5);
    cairo_stroke(cr);
    /* the icon */
    if (w->icon) {
        double pw = gdk_pixbuf_get_width(w->icon), want = ICON_PX * fmax(S.scale, 0.7);
        cairo_save(cr);
        cairo_translate(cr, p.x - want / 2, p.y - want / 2);
        cairo_scale(cr, want / pw, want / pw);
        gdk_cairo_set_source_pixbuf(cr, w->icon, 0, 0);
        cairo_paint_with_alpha(cr, a * (w->minimized ? 0.55 : 1));
        cairo_restore(cr);
    }
    /* on another layer: its number, in the layer's tint */
    if (w->ws > 0 && w->ws != S.current) {
        double bx = p.x + r * 0.86, by = p.y + r * 0.62, br = 9;
        strata_rgb t = strata_tint(S.c.accent, w->ws);
        hex_path(cr, bx, by, br);
        set_rgb(cr, t, 0.95 * a);
        cairo_fill_preserve(cr);
        set_rgb(cr, S.c.bg, 0.9 * a);
        cairo_set_line_width(cr, 1.5);
        cairo_stroke(cr);
        char num[8];
        snprintf(num, sizeof num, "%d", w->ws);
        set_rgb(cr, luma(t) > 0.55 ? (strata_rgb){0.05, 0.06, 0.1} : (strata_rgb){1, 1, 1}, a);
        text_at(cr, num, 10, 1, bx, by, 0);
    }
    /* needs you: a dashed ring, slowly turning */
    if (w->urgent) {
        double dash[] = {5, 4};
        cairo_set_dash(cr, dash, 2, S.animations ? fmod(now_s() * 12, 9) : 0);
        hex_path(cr, p.x, p.y, r + 7);
        set_rgb(cr, S.c.warn, 0.95 * a);
        cairo_set_line_width(cr, 2);
        cairo_stroke(cr);
        cairo_set_dash(cr, NULL, 0, 0);
    }
    if (i == S.sel) {
        hex_path(cr, p.x, p.y, r + (w->urgent ? 12 : 6));
        set_rgb(cr, S.c.text, 0.9 * a);
        cairo_set_line_width(cr, 2.4);
        cairo_stroke(cr);
    }
    /* the title */
    set_rgb(cr, S.c.text, (i == S.sel ? 1 : 0.82) * a);
    text_at(cr, *w->title ? w->title : w->app, 11.5, i == S.sel, p.x, p.y + r + 15, TITLE_W * fmax(S.scale, 0.75));
}

static void capsule(cairo_t *cr, double cx, double cy, double w, double h)
{
    double x = cx - w / 2, y = cy - h / 2, rr = h / 2;
    cairo_new_sub_path(cr);
    cairo_arc(cr, x + w - rr, y + rr, rr, -M_PI / 2, M_PI / 2);
    cairo_arc(cr, x + rr, y + rr, rr, M_PI / 2, 3 * M_PI / 2);
    cairo_close_path(cr);
}

static gboolean draw(GtkWidget *wd, cairo_t *cr, gpointer d)
{
    double W = gtk_widget_get_allocated_width(wd), H = gtk_widget_get_allocated_height(wd), f = flight();
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    set_rgb(cr, S.c.bg, 0.97 * (S.animations ? fmin(1, 0.4 + f) : 1));  /* nearly opaque: windows behind would muddle the sky */
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
    /* dust: the same faint stars every time */
    guint32 seed = 0x9e3779b9u;
    for (int k = 0; k < DUST; k++) {
        seed = seed * 1664525u + 1013904223u;
        double x = (seed >> 8) / 16777216.0 * W;
        seed = seed * 1664525u + 1013904223u;
        double y = (seed >> 8) / 16777216.0 * H;
        seed = seed * 1664525u + 1013904223u;
        double t = (seed >> 8) / 16777216.0;
        set_rgb(cr, S.c.text, (0.05 + 0.16 * t) * f);
        cairo_arc(cr, x, y, 0.6 + 0.9 * t, 0, 2 * M_PI);
        cairo_fill(cr);
    }
    double r = STAR_R * fmax(S.scale, 0.7);
    if (!S.n) {
        set_rgb(cr, S.c.text, 0.95);
        text_at(cr, "No windows open", 20, 1, W / 2, H / 2 - 14, 0);
        set_rgb(cr, S.c.muted, 0.95);
        text_at(cr, "Open an app from Orbit. Escape leaves.", 13, 0, W / 2, H / 2 + 16, 0);
        return FALSE;
    }
    struct chart_pt at[CHART_MAX];
    for (int i = 0; i < S.n; i++) at[i] = shown(i, W, H, f);
    double alpha[CHART_MAX];
    for (int i = 0; i < S.n; i++) alpha[i] = f * (S.on[i] ? 1 : 0.16);
    /* each app's figure: its windows joined around their ring */
    cairo_set_line_width(cr, 1);
    for (int g = 0; g < S.ngroups; g++) {
        int mem[CHART_MAX], m = 0;
        for (int k = 0; k < CHART_MAX && m < S.n; k++)
            for (int i = 0; i < S.n; i++)
                if (S.group[i] == g && S.order[i] == k) mem[m++] = i;
        for (int k = 0; m > 1 && k < (m > 2 ? m : 1); k++) {
            int a = mem[k], b = mem[(k + 1) % m];
            set_rgb(cr, S.c.muted, 0.34 * fmin(alpha[a], alpha[b]));
            cairo_move_to(cr, at[a].x, at[a].y);
            cairo_line_to(cr, at[b].x, at[b].y);
            cairo_stroke(cr);
        }
        /* the app's name above its figure */
        int top = S.group_first[g];
        for (int k = 0; k < m; k++)
            if (at[mem[k]].y < at[top].y) top = mem[k];
        double ga = 0;
        for (int k = 0; k < m; k++) ga = fmax(ga, alpha[mem[k]]);
        set_rgb(cr, S.c.muted, 0.95 * ga);
        text_at(cr, S.w[S.group_first[g]].app, 11, 1, at[top].x, at[top].y - r - 16, 200);
    }
    /* projects: dashed lines, the name beside the first */
    double dash[] = {6, 5};
    cairo_set_dash(cr, dash, 2, 0);
    cairo_set_line_width(cr, 1.6);
    for (int k = 0; k < S.nlinks; k++) {
        int a = S.links[k][0], b = S.links[k][1];
        double la = 0.7 * fmin(alpha[a], alpha[b]) * (S.on[a] || S.on[b] ? 1 : 0.5);
        double len = hypot(at[b].x - at[a].x, at[b].y - at[a].y);
        if (len < 2 * r + 4) continue;
        double ux = (at[b].x - at[a].x) / len, uy = (at[b].y - at[a].y) / len;  /* from hexagon edge to edge */
        set_rgb(cr, S.c.accent, la);
        cairo_move_to(cr, at[a].x + ux * (r + 4), at[a].y + uy * (r + 4));
        cairo_line_to(cr, at[b].x - ux * (r + 4), at[b].y - uy * (r + 4));
        cairo_stroke(cr);
    }
    cairo_set_dash(cr, NULL, 0, 0);
    int named[CHART_MAX] = {0};
    for (int k = 0; k < S.nlinks; k++) {
        int a = S.links[k][0], b = S.links[k][1], pj = S.proj[a];
        if (pj < 0 || named[pj]) continue;
        named[pj] = 1;
        double mx = (at[a].x + at[b].x) / 2, my = (at[a].y + at[b].y) / 2;
        cairo_save(cr);
        PangoLayout *l = pango_cairo_create_layout(cr);
        PangoFontDescription *fd = pango_font_description_from_string("Inter Bold");
        pango_font_description_set_absolute_size(fd, 10.5 * PANGO_SCALE);
        pango_layout_set_font_description(l, fd);
        pango_layout_set_text(l, S.p[pj].name, -1);
        int tw, th;
        pango_layout_get_pixel_size(l, &tw, &th);
        capsule(cr, mx, my, tw + 16, th + 6);
        set_rgb(cr, S.c.bg, 0.92 * fmin(alpha[a], alpha[b]));
        cairo_fill_preserve(cr);
        set_rgb(cr, S.c.accent, 0.7 * fmin(alpha[a], alpha[b]));
        cairo_set_line_width(cr, 1);
        cairo_stroke(cr);
        set_rgb(cr, S.c.accent, fmin(alpha[a], alpha[b]));
        cairo_move_to(cr, mx - tw / 2.0, my - th / 2.0);
        pango_cairo_show_layout(cr, l);
        pango_font_description_free(fd);
        g_object_unref(l);
        cairo_restore(cr);
    }
    /* the stars, the one picked last (on top) */
    for (int i = 0; i < S.n; i++)
        if (i != S.sel) draw_star(cr, i, at[i], r, alpha[i]);
    if (S.sel >= 0) draw_star(cr, S.sel, at[S.sel], r, alpha[S.sel]);
    /* the search */
    if (*S.query) {
        int on = 0;
        for (int i = 0; i < S.n; i++) on += S.on[i];
        char *t = g_strdup_printf("Find: %s    %d of %d", S.query, on, S.n);
        cairo_save(cr);
        capsule(cr, W / 2, 34, fmin(W - 40, 120 + 9.0 * g_utf8_strlen(t, -1)), 34);
        set_rgb(cr, S.c.surface, 0.95);
        cairo_fill_preserve(cr);
        set_rgb(cr, S.c.accent, 0.8);
        cairo_set_line_width(cr, 1.2);
        cairo_stroke(cr);
        cairo_restore(cr);
        set_rgb(cr, on ? S.c.text : S.c.warn, 1);
        text_at(cr, t, 13, 0, W / 2, 34, W - 60);
        g_free(t);
    }
    /* the picked window, in words */
    if (S.sel >= 0) {
        const struct sky_win *w = &S.w[S.sel];
        GString *sub = g_string_new(w->app);
        if (w->ws > 0) g_string_append_printf(sub, "  ·  Layer %d", w->ws);
        if (w->minimized) g_string_append(sub, "  ·  minimised");
        if (S.proj[S.sel] >= 0) g_string_append_printf(sub, "  ·  %s", S.p[S.proj[S.sel]].place);
        double cw = fmin(W - 40, 560);
        cairo_save(cr);
        capsule(cr, W / 2, H - 76, cw, 50);
        set_rgb(cr, S.c.surface, 0.96);
        cairo_fill_preserve(cr);
        set_rgb(cr, S.c.accent, 0.5);
        cairo_set_line_width(cr, 1);
        cairo_stroke(cr);
        cairo_restore(cr);
        set_rgb(cr, S.c.text, 1);
        text_at(cr, *w->title ? w->title : w->app, 14, 1, W / 2, H - 85, cw - 40);
        set_rgb(cr, S.c.muted, 1);
        text_at(cr, sub->str, 11.5, 0, W / 2, H - 65, cw - 40);
        g_string_free(sub, TRUE);
    }
    set_rgb(cr, S.c.muted, 0.85);
    text_at(cr, "Arrows move  ·  Enter goes there  ·  Delete closes  ·  type to find  ·  Escape leaves", 11, 0, W / 2, H - 26, W - 40);
    return FALSE;
}

/* ---------- use ---------- */

static void sky_close(void)
{
    if (!S.win) return;
    if (S.tick) gtk_widget_remove_tick_callback(S.area, S.tick);
    if (S.urgent_timer) g_source_remove(S.urgent_timer);
    S.tick = S.urgent_timer = 0;
    gtk_widget_destroy(S.win);
    S.win = S.area = NULL;
    S.query[0] = 0;
    S.sel = -1;
    write_state();
}

static void go_to(int i)
{
    if (i < 0 || i >= S.n) return;
    unsigned long id = S.w[i].id;
    sky_close();  /* first: the keyboard goes back before the window comes forward */
    if (S.hooks.focus) S.hooks.focus(id);
}

static void close_window(int i)
{
    if (i < 0 || i >= S.n || !S.hooks.close) return;
    S.hooks.close(S.w[i].id);
}

static void pick_best_match(void)
{
    if (S.sel >= 0 && S.on[S.sel]) return;
    S.sel = -1;
    for (int i = 0; i < S.n; i++)
        if (S.on[i] && (S.sel < 0 || S.rank[i] < S.rank[S.sel])) S.sel = i;
}

/* Tab: the next window by last use (among the ones that match). */
static void by_use(int dir)
{
    int idx[CHART_MAX], m = 0;
    for (int r = 0; r <= CHART_MAX; r++)
        for (int i = 0; i < S.n; i++)
            if (S.rank[i] == r && S.on[i]) idx[m++] = i;
    if (!m) return;
    int at = 0;
    for (int k = 0; k < m; k++)
        if (idx[k] == S.sel) at = k;
    S.sel = idx[((at + dir) % m + m) % m];
}

static gboolean on_key(GtkWidget *wd, GdkEventKey *e, gpointer d)
{
    guint k = e->keyval;
    int ctrl = (e->state & GDK_CONTROL_MASK) != 0;
    if (k == GDK_KEY_Escape) {
        if (*S.query) {
            S.query[0] = 0;
            recompute();
            pick_best_match();
        } else {
            sky_close();
            return TRUE;
        }
    } else if (k == GDK_KEY_Return || k == GDK_KEY_KP_Enter || (k == GDK_KEY_space && !*S.query)) {
        go_to(S.sel);
        return TRUE;
    } else if (k == GDK_KEY_Delete || (ctrl && (k == GDK_KEY_w || k == GDK_KEY_W))) {
        close_window(S.sel);
    } else if (k == GDK_KEY_Left || k == GDK_KEY_Right || k == GDK_KEY_Up || k == GDK_KEY_Down) {
        int dx = k == GDK_KEY_Left ? -1 : k == GDK_KEY_Right ? 1 : 0, dy = k == GDK_KEY_Up ? -1 : k == GDK_KEY_Down ? 1 : 0;
        if (S.sel < 0) pick_best_match();
        else {
            int to = chart_step(S.star, S.on, S.n, S.sel, dx, dy);
            if (to >= 0) S.sel = to;
        }
    } else if (k == GDK_KEY_Tab || k == GDK_KEY_ISO_Left_Tab) {
        by_use(k == GDK_KEY_ISO_Left_Tab || (e->state & GDK_SHIFT_MASK) ? -1 : 1);
    } else if (k == GDK_KEY_Home) {
        S.sel = -1;
        for (int i = 0; i < S.n; i++)
            if (S.w[i].focused) S.sel = i;
        if (S.sel < 0 || !S.on[S.sel]) {
            S.sel = -1;
            pick_best_match();
        }
    } else if (k == GDK_KEY_BackSpace) {
        if (!*S.query) return TRUE;
        char *end = S.query + strlen(S.query), *prev = g_utf8_find_prev_char(S.query, end);
        *(prev ? prev : S.query) = 0;
        recompute();
        pick_best_match();
    } else if (!ctrl && !(e->state & (GDK_MOD1_MASK | GDK_SUPER_MASK | GDK_MOD4_MASK))) {
        gunichar u = gdk_keyval_to_unicode(k);
        if (!u || g_unichar_iscntrl(u)) return TRUE;
        char buf[8];
        int len = g_unichar_to_utf8(u, buf);
        if (strlen(S.query) + (size_t)len + 1 > sizeof S.query) return TRUE;
        strncat(S.query, buf, (size_t)len);
        recompute();
        S.sel = -1;
        pick_best_match();
    } else {
        return TRUE;
    }
    gtk_widget_queue_draw(S.area);
    write_state();
    return TRUE;
}

static int star_at(double x, double y)
{
    double r = STAR_R * fmax(S.scale, 0.7) + 6;
    for (int i = 0; i < S.n; i++)
        if (hypot(x - S.star[i].x, y - S.star[i].y) <= r) return i;
    return -1;
}

static void point_at(double x, double y)
{
    int i = star_at(x, y);
    if (i >= 0 && i != S.sel) {
        S.sel = i;
        gtk_widget_queue_draw(S.area);
        write_state();
    }
}

static gboolean on_motion(GtkWidget *wd, GdkEventMotion *e, gpointer d)
{
    point_at(e->x, e->y);
    return TRUE;
}

/* A pointer that arrives in one jump sends only this. */
static gboolean on_enter(GtkWidget *wd, GdkEventCrossing *e, gpointer d)
{
    point_at(e->x, e->y);
    return TRUE;
}

static gboolean on_button(GtkWidget *wd, GdkEventButton *e, gpointer d)
{
    if (e->type != GDK_BUTTON_PRESS) return TRUE;
    int i = star_at(e->x, e->y);
    if (e->button == 1) {
        if (i >= 0) go_to(i);
        else sky_close();
    } else if (e->button == 2 && i >= 0) {
        close_window(i);
    }
    return TRUE;
}

static gboolean on_tick(GtkWidget *wd, GdkFrameClock *fc, gpointer d)
{
    gtk_widget_queue_draw(wd);
    if (now_s() - S.opened_at > FLIGHT + 0.05) {
        S.tick = 0;
        write_state();
        return G_SOURCE_REMOVE;
    }
    return G_SOURCE_CONTINUE;
}

static gboolean on_urgent_turn(gpointer d)
{
    int any = 0;
    for (int i = 0; i < S.n; i++) any |= S.w[i].urgent;
    if (S.area && any) gtk_widget_queue_draw(S.area);
    return G_SOURCE_CONTINUE;
}

static void on_size(GtkWidget *wd, GtkAllocation *a, gpointer d)
{
    recompute();
    write_state();
}

static void sky_open(void)
{
    S.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(S.win), "Constellation");
    GdkVisual *v = gdk_screen_get_rgba_visual(gtk_widget_get_screen(S.win));
    if (v) gtk_widget_set_visual(S.win, v);
    gtk_widget_set_app_paintable(S.win, TRUE);
    gtk_layer_init_for_window(GTK_WINDOW(S.win));
    gtk_layer_set_namespace(GTK_WINDOW(S.win), "nexa-constellation");
    gtk_layer_set_layer(GTK_WINDOW(S.win), GTK_LAYER_SHELL_LAYER_OVERLAY);
    for (int e = 0; e < GTK_LAYER_SHELL_EDGE_ENTRY_NUMBER; e++) gtk_layer_set_anchor(GTK_WINDOW(S.win), e, TRUE);
    gtk_layer_set_exclusive_zone(GTK_WINDOW(S.win), -1);  /* over the bar too */
    gtk_layer_set_keyboard_mode(GTK_WINDOW(S.win), GTK_LAYER_SHELL_KEYBOARD_MODE_EXCLUSIVE);
    S.area = gtk_drawing_area_new();
    gtk_widget_add_events(S.area, GDK_POINTER_MOTION_MASK | GDK_ENTER_NOTIFY_MASK | GDK_BUTTON_PRESS_MASK);
    g_signal_connect(S.area, "draw", G_CALLBACK(draw), NULL);
    g_signal_connect(S.area, "motion-notify-event", G_CALLBACK(on_motion), NULL);
    g_signal_connect(S.area, "enter-notify-event", G_CALLBACK(on_enter), NULL);
    g_signal_connect(S.area, "button-press-event", G_CALLBACK(on_button), NULL);
    g_signal_connect(S.area, "size-allocate", G_CALLBACK(on_size), NULL);
    g_signal_connect(S.win, "key-press-event", G_CALLBACK(on_key), NULL);
    gtk_container_add(GTK_CONTAINER(S.win), S.area);
    S.query[0] = 0;
    S.opened_at = now_s();
    recompute();
    /* the window that was in front before is in front of the list; start on the one before it,
     * so Super+Tab then Enter swaps between the last two */
    S.sel = -1;
    for (int i = 0; i < S.n; i++)
        if (S.rank[i] == 1) S.sel = i;
    if (S.sel < 0) pick_best_match();
    gtk_widget_show_all(S.win);
    if (S.animations) S.tick = gtk_widget_add_tick_callback(S.area, on_tick, NULL, NULL);
    if (S.animations) S.urgent_timer = g_timeout_add(66, on_urgent_turn, NULL);
    request_projects();
    write_state();
}

void sky_toggle(void)
{
    if (S.win) sky_close();
    else sky_open();
}
