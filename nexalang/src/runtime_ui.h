/* NexaLang runtime: windows (GTK 3), added only to programs that `use ui`.
 *
 * The program's own thread is the GTK main thread: widgets are created and
 * changed only by the program's calls, and GTK runs only inside ui.wait()
 * (and while a dialog or a broker request is open). Signal handlers never
 * call NexaLang code; they put an event in a queue that ui.wait() hands
 * out one at a time. So there are no callbacks, no re-entrancy and no
 * threads in NexaLang code.
 *
 * Memory across the boundary (NEXALANG.md section 3):
 *   - text passed in is borrowed: it's copied into GTK and never kept;
 *   - text handed out is a new owned NexaLang string (counted, freed by
 *     the program like any other);
 *   - lists passed in are borrowed and only read.
 * GTK's own memory is GTK's; NexaLang's leak counter only counts NexaLang's.
 *
 * Widgets are numbered (1, 2, ...). A number that isn't a widget, or a
 * widget used for something it can't do, stops the program with a plain
 * message, like a list index outside the list. Numbers are never used
 * twice: after ui.remove or ui.clear (0.7), a removed widget's number stops
 * the program if it's used again, instead of meaning some newer widget. */
#include <gtk/gtk.h>

enum { NX_UI_COLUMN = 1, NX_UI_ROW, NX_UI_LABEL, NX_UI_BUTTON, NX_UI_ENTRY, NX_UI_CHECK, NX_UI_LIST, NX_UI_CODE };
enum { NX_EV_CLICKED = 1, NX_EV_CHANGED, NX_EV_SUBMITTED, NX_EV_TOGGLED, NX_EV_SELECTED, NX_EV_ACTIVATED, NX_EV_CLOSED, NX_EV_TICK };
#define NX_UI_MAX_WIDGETS 100000

typedef struct {
    int64_t id;       /* its number (the key in nx_ui.items) */
    GtkWidget *w;     /* the widget itself */
    GtkWidget *outer; /* what's packed into the parent (a list's scroller) */
    int kind, style;  /* style: a label's or a button's (see their makers) */
    gulong sig;       /* the handler that reports the person's changes */
    char *tip, *keys; /* tooltip and shortcut label (g_malloc'd) */
} nx_ui_item;

typedef struct { int kind; int64_t id, num; char *text; } nx_ui_ev;
typedef struct { guint key; GdkModifierType mods; int64_t id; } nx_ui_key;

static struct {
    bool started, busy, has_primary;
    GtkWidget *win, *status, *content;
    GHashTable *items;  /* number -> nx_ui_item*, only widgets that still exist */
    int64_t n;          /* the next number (numbers are never used twice) */
    bool tree;          /* NEXALANG_UI_TREE: Ctrl+Shift+F12 prints the widgets (tests) */
    nx_ui_ev *q; int64_t qhead, qlen, qcap;
    nx_ui_ev cur;
    nx_ui_key *keys; int64_t nkeys, kcap;
    GtkCssProvider *live;
} nx_ui;

/* ---- the look: Hexa (WRITING_NEXA_APPS.md section 4) ---- */
static const char NX_UI_CSS[] =
    /* Fallback: NexaOS "Horizon". The person's theme overrides these. */
    "@define-color nexa_bg #0c1228;         @define-color nexa_bg_deep #080c1e;\n"
    "@define-color nexa_surface #101732;    @define-color nexa_surface_hi #222a46;\n"
    "@define-color nexa_hover #333a54;      @define-color nexa_field #0f152d;\n"
    "@define-color nexa_border #363d5a;     @define-color nexa_text #d9e0f6;\n"
    "@define-color nexa_text_strong #e6ecff;@define-color nexa_muted #8f9bc4;\n"
    "@define-color nexa_disabled #525b79;   @define-color nexa_accent #4da3ff;\n"
    "@define-color nexa_accent_hover #6db4ff;@define-color nexa_on_accent #080c1e;\n"
    "@define-color nexa_selected #264779;   @define-color nexa_selected_text #e6ecff;\n"
    "@define-color nexa_crit #ff5c7a;       @define-color nexa_ok #5ee6a8;\n"
    "@define-color nexa_warn #ffb547;\n"
    "* { font-family: Inter, sans-serif; font-size: 10pt; transition: all 160ms cubic-bezier(0.22, 1, 0.36, 1); }\n"
    "window, dialog { background-color: @nexa_bg; color: @nexa_text; }\n"
    "label { color: @nexa_text; }\n"
    "label:disabled { color: @nexa_disabled; }\n"
    ".nx-heading { font-size: 15pt; font-weight: 700; color: @nexa_text_strong; }\n"
    ".nx-note { font-size: 8.5pt; color: @nexa_muted; }\n"
    ".nx-mono { font-family: \"JetBrains Mono\", monospace; color: @nexa_text_strong; }\n"
    ".nx-display { font-family: \"JetBrains Mono\", monospace; font-size: 22pt; font-weight: 700; color: @nexa_text_strong; }\n"
    /* buttons: surface; one primary in accent; danger in crit */
    "button { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text_strong;"
    "  border: 1px solid @nexa_border; border-radius: 10px; padding: 4px 12px; min-height: 24px;"
    "  box-shadow: none; text-shadow: none; -gtk-icon-shadow: none; }\n"
    "button label { color: inherit; }\n"
    "button:hover { background-color: @nexa_hover; }\n"
    /* keyboard focus must be easy to see */
    "button:focus, checkbutton:focus, row:focus { outline: 2px solid alpha(@nexa_accent, 0.8); outline-offset: 1px; -gtk-outline-radius: 10px; }\n"
    "button:active { background-color: @nexa_selected; }\n"
    "button:disabled { background-color: @nexa_surface; color: @nexa_disabled; border-color: @nexa_surface_hi; }\n"
    "button.nx-primary { background-color: @nexa_accent; color: @nexa_on_accent; border-color: @nexa_accent; font-weight: 700; }\n"
    "button.nx-primary:hover { background-color: @nexa_accent_hover; border-color: @nexa_accent_hover; }\n"
    "button.nx-primary:disabled { background-color: alpha(@nexa_accent, 0.35); color: @nexa_on_accent; }\n"
    "button.nx-danger { color: @nexa_crit; border-color: alpha(@nexa_crit, 0.6); }\n"
    "button.nx-danger:hover { background-color: @nexa_crit; color: @nexa_on_accent; }\n"
    /* text fields */
    "entry { background-image: none; background-color: @nexa_field; color: @nexa_text_strong;"
    "  border: 1px solid @nexa_border; border-radius: 6px; padding: 4px 8px; min-height: 24px; box-shadow: none; caret-color: @nexa_accent; }\n"
    "entry:focus { border-color: @nexa_accent; }\n"
    "entry:disabled { color: @nexa_disabled; background-color: @nexa_surface; }\n"
    "entry placeholder { color: @nexa_muted; }\n"
    "entry selection { background-color: @nexa_selected; color: @nexa_selected_text; }\n"
    /* check boxes */
    "checkbutton { color: @nexa_text; padding: 4px 0; }\n"
    "checkbutton check { -gtk-icon-source: none; background-image: none; background-color: @nexa_field;"
    "  border: 1px solid @nexa_border; border-radius: 6px; min-width: 16px; min-height: 16px; margin-right: 8px; box-shadow: none; }\n"
    "checkbutton check:hover { border-color: @nexa_accent; }\n"
    "checkbutton check:checked { background-color: @nexa_accent; border-color: @nexa_accent;"
    "  -gtk-icon-source: -gtk-icontheme(\"object-select-symbolic\"); color: @nexa_on_accent; }\n"
    /* scrolling lists, with an empty state */
    ".nx-list { background-color: @nexa_surface; border: 1px solid @nexa_border; border-radius: 16px; }\n"
    ".nx-list list { background-color: transparent; padding: 4px; }\n"
    ".nx-list row { border-radius: 10px; padding: 8px 12px; color: @nexa_text; }\n"
    ".nx-list row:hover { background-color: @nexa_hover; }\n"
    ".nx-list row:selected { background-color: @nexa_selected; color: @nexa_selected_text; }\n"
    ".nx-list row:selected label { color: @nexa_selected_text; }\n"
    ".nx-empty { color: @nexa_muted; padding: 24px; }\n"
    "scrollbar { background-color: transparent; border: none; }\n"
    "scrollbar slider { background-color: alpha(@nexa_text, 0.25); border-radius: 6px; min-width: 6px; min-height: 6px; border: none; }\n"
    /* read-only code (0.7) */
    ".nx-code { background-color: @nexa_field; border: 1px solid @nexa_border; border-radius: 10px; }\n"
    ".nx-code textview, .nx-code text { background-color: @nexa_field; color: @nexa_text_strong; font-family: \"JetBrains Mono\", monospace; }\n"
    ".nx-code text selection { background-color: @nexa_selected; color: @nexa_selected_text; }\n"
    ".nx-code:focus-within, .nx-code textview:focus { border-color: @nexa_accent; }\n"
    /* the file chooser (0.7): its places, folders and files on Hexa colours */
    "filechooser, filechooser box, filechooser stack, filechooser paned { background-color: @nexa_bg; color: @nexa_text; }\n"
    "filechooser .sidebar, placessidebar, placessidebar list { background-color: @nexa_surface; color: @nexa_text; border: none; }\n"
    "placessidebar row { color: @nexa_text; border-radius: 10px; }\n"
    "placessidebar row:hover { background-color: @nexa_hover; }\n"
    "placessidebar row:selected { background-color: @nexa_selected; color: @nexa_selected_text; }\n"
    "placessidebar row label, placessidebar row image { color: inherit; }\n"
    "filechooser treeview, filechooser treeview.view { background-color: @nexa_surface; color: @nexa_text; }\n"
    "filechooser treeview:selected, filechooser treeview.view:selected { background-color: @nexa_selected; color: @nexa_selected_text; }\n"
    "filechooser treeview header button { background-color: @nexa_surface_hi; color: @nexa_muted; border-radius: 0; border: none; border-bottom: 1px solid @nexa_border; }\n"
    "filechooser scrolledwindow { background-color: @nexa_surface; }\n"
    "filechooser .path-bar button, filechooser pathbar button { border-radius: 10px; }\n"
    "filechooser actionbar, filechooser .search-bar, filechooser revealer box { background-color: @nexa_bg; color: @nexa_text; border: none; }\n"
    "filechooser label { color: @nexa_text; }\n"
    "filechooser paned separator { background-color: @nexa_border; min-width: 1px; }\n"
    /* GTK's own questions (the file chooser's "replace it?") */
    "messagedialog, messagedialog.background, messagedialog .dialog-vbox, messagedialog box { background-color: @nexa_surface; color: @nexa_text; }\n"
    "messagedialog label { color: @nexa_text; }\n"
    "messagedialog label.title, messagedialog .titlebar label { color: @nexa_text_strong; font-weight: 700; }\n"
    "messagedialog .dialog-action-area { padding: 12px 16px 16px 16px; }\n"
    "button.destructive-action { color: @nexa_crit; border-color: alpha(@nexa_crit, 0.6); }\n"
    "button.destructive-action:hover { background-color: @nexa_crit; color: @nexa_on_accent; }\n"
    "button.suggested-action { background-color: @nexa_accent; color: @nexa_on_accent; border-color: @nexa_accent; font-weight: 700; }\n"
    "popover, popover.background, menu, .menu, .context-menu { background-color: @nexa_surface_hi; color: @nexa_text; border: 1px solid @nexa_border; }\n"
    "menuitem, modelbutton { color: @nexa_text; }\n"
    "menuitem:hover, modelbutton:hover { background-color: @nexa_hover; }\n"
    "tooltip { background-color: @nexa_surface_hi; color: @nexa_text_strong; border-radius: 6px; }\n"
    "tooltip * { color: @nexa_text_strong; }\n"
    /* the status line at the bottom */
    ".nx-status { color: @nexa_muted; padding: 8px 16px; border-top: 1px solid @nexa_border; background-color: @nexa_bg_deep; }\n"
    ".nx-error { color: @nexa_crit; }\n"
    /* its own title bar: Hyprland draws none, so minimize/maximize/close live here */
    "headerbar { background-image: none; background-color: @nexa_surface; color: @nexa_text_strong;"
    "  border: none; border-bottom: 1px solid @nexa_border; box-shadow: none; min-height: 38px; padding: 0 8px; }\n"
    "headerbar .title { font-weight: 600; color: @nexa_text_strong; }\n"
    "headerbar button.titlebutton { background-image: none; background-color: transparent; color: @nexa_text;"
    "  border: none; border-radius: 10px; box-shadow: none; min-width: 26px; min-height: 26px; padding: 0; }\n"
    "headerbar button.titlebutton:hover { background-color: @nexa_hover; }\n"
    "headerbar button.titlebutton.close:hover { background-color: @nexa_crit; color: @nexa_on_accent; }\n"
    /* dialogs (the password question) */
    "dialog .dialog-action-area { padding: 12px 16px 16px 16px; }\n"
    "dialog .dialog-vbox { padding: 0; }\n";

/* ---- small helpers ---- */

static void nx_ui_item_free(gpointer p) {
    nx_ui_item *it = p;
    g_free(it->tip);
    g_free(it->keys);
    g_free(it);
}

static nx_ui_item *nx_ui_new_item(GtkWidget *w, GtkWidget *outer, int kind, int style) {
    nx_ui_item *it = g_new0(nx_ui_item, 1);
    *it = (nx_ui_item){nx_ui.n++, w, outer, kind, style, 0, NULL, NULL};
    g_hash_table_insert(nx_ui.items, &it->id, it);
    g_object_set_data(G_OBJECT(outer), "nx-id", GINT_TO_POINTER((int)it->id));
    return it;
}


/* A NUL-terminated copy for GTK. NexaLang text may contain zero bytes;
   GTK can't show them, so each becomes U+FFFD. Free with g_free. */
static char *nx_ui_cstr(nx_str s) {
    int64_t zeros = 0;
    for (int64_t i = 0; i < s.len; i++) zeros += s.p[i] == 0;
    char *c = g_malloc((size_t)(s.len + zeros * 2 + 1));
    char *w = c;
    for (int64_t i = 0; i < s.len; i++) {
        if (s.p[i] == 0) { memcpy(w, "\357\277\275", 3); w += 3; }
        else *w++ = s.p[i];
    }
    *w = 0;
    return c;
}

/* GTK text as new NexaLang text (checked UTF-8, like all NexaLang text). */
static nx_str nx_ui_str(const char *t) {
    if (!t) return nx_str_lit("", 0);
    return nx_str_lossy(t, strlen(t));
}

static void nx_ui_need_start(int line) {
    if (!nx_ui.started) nx_trap(line, "call ui.start first, to open the window");
}

static const char *nx_ui_kind_name(int k) {
    switch (k) {
    case NX_UI_COLUMN: return "a column";
    case NX_UI_ROW: return "a row";
    case NX_UI_LABEL: return "a text";
    case NX_UI_BUTTON: return "a button";
    case NX_UI_ENTRY: return "a text field";
    case NX_UI_CHECK: return "a check box";
    case NX_UI_CODE: return "a code view";
    default: return "a list";
    }
}

static nx_ui_item *nx_ui_get(int64_t id, int line) {
    nx_ui_need_start(line);
    nx_ui_item *it = g_hash_table_lookup(nx_ui.items, &id);
    if (it) return it;
    if (id >= 1 && id < nx_ui.n) nx_trap(line, "widget %" PRId64 " was removed (ui.remove or ui.clear), so it can't be used any more", id);
    nx_trap(line, "%" PRId64 " isn't a widget of this window", id);
}

/* The widget `id`, which must be one of the kinds in `mask` (1 << kind). */
static nx_ui_item *nx_ui_want(int64_t id, unsigned mask, const char *what, int line) {
    nx_ui_item *it = nx_ui_get(id, line);
    if (!(mask & (1u << it->kind)))
        nx_trap(line, "widget %" PRId64 " is %s, which can't %s", id, nx_ui_kind_name(it->kind), what);
    return it;
}

static void nx_ui_push(int kind, int64_t id, int64_t num, const char *text) {
    if (nx_ui.qhead == nx_ui.qlen) nx_ui.qhead = nx_ui.qlen = 0;
    if (nx_ui.qlen == nx_ui.qcap) {
        nx_ui.qcap = nx_ui.qcap ? nx_ui.qcap * 2 : 16;
        nx_ui.q = g_realloc_n(nx_ui.q, (gsize)nx_ui.qcap, sizeof *nx_ui.q);
    }
    nx_ui_ev e = {kind, id, num, text ? g_strdup(text) : NULL};
    nx_ui.q[nx_ui.qlen++] = e;
}

static void nx_ui_tree(GtkWidget *outer, int depth);
static void nx_ui_tree_text(const char *t);

/* ---- signal handlers: they only queue events ---- */

static void nx_ui_on_clicked(GtkButton *b, gpointer d) {
    (void)b;
    if (!nx_ui.busy) nx_ui_push(NX_EV_CLICKED, GPOINTER_TO_INT(d), 0, NULL);
}
static void nx_ui_on_changed(GtkEditable *e, gpointer d) {
    nx_ui_push(NX_EV_CHANGED, GPOINTER_TO_INT(d), 0, gtk_entry_get_text(GTK_ENTRY(e)));
}
static void nx_ui_on_activate(GtkEntry *e, gpointer d) {
    if (!nx_ui.busy) nx_ui_push(NX_EV_SUBMITTED, GPOINTER_TO_INT(d), 0, gtk_entry_get_text(e));
}
static void nx_ui_on_toggled(GtkToggleButton *b, gpointer d) {
    nx_ui_push(NX_EV_TOGGLED, GPOINTER_TO_INT(d), gtk_toggle_button_get_active(b), NULL);
}
static void nx_ui_on_row_selected(GtkListBox *l, GtkListBoxRow *row, gpointer d) {
    (void)l;
    if (row) nx_ui_push(NX_EV_SELECTED, GPOINTER_TO_INT(d), gtk_list_box_row_get_index(row), NULL);
}
static void nx_ui_on_row_activated(GtkListBox *l, GtkListBoxRow *row, gpointer d) {
    (void)l;
    if (row && !nx_ui.busy) nx_ui_push(NX_EV_ACTIVATED, GPOINTER_TO_INT(d), gtk_list_box_row_get_index(row), NULL);
}
static gboolean nx_ui_on_delete(GtkWidget *w, GdkEvent *e, gpointer d) {
    (void)w; (void)e; (void)d;
    nx_ui_push(NX_EV_CLOSED, 0, 0, NULL);
    return TRUE; /* the program decides; the window stays until it ends */
}
static gboolean nx_ui_on_key(GtkWidget *w, GdkEventKey *e, gpointer d) {
    (void)w; (void)d;
    GdkModifierType mods = e->state & gtk_accelerator_get_default_mod_mask();
    guint key = gdk_keyval_to_lower(e->keyval);
    if (key == GDK_KEY_Escape && mods == 0) { nx_ui_push(NX_EV_CLOSED, 0, 0, NULL); return TRUE; }
    if (key == GDK_KEY_q && mods == GDK_CONTROL_MASK) { nx_ui_push(NX_EV_CLOSED, 0, 0, NULL); return TRUE; }
    if (nx_ui.tree && key == GDK_KEY_F12 && mods == (GDK_CONTROL_MASK | GDK_SHIFT_MASK)) {
        fflush(stdout);
        nx_ui_tree(nx_ui.content, 0);
        if (gtk_widget_get_visible(nx_ui.status)) {
            bool bad = gtk_style_context_has_class(gtk_widget_get_style_context(nx_ui.status), "nx-error");
            fputs(bad ? "status error " : "status ", stdout);
            nx_ui_tree_text(gtk_label_get_text(GTK_LABEL(nx_ui.status)));
            putchar('\n');
        }
        fflush(stdout);
        return TRUE;
    }
    if (nx_ui.busy) return FALSE;
    /* Tab leaves a list (arrow keys move inside it), instead of walking
       through every row */
    if ((key == GDK_KEY_Tab || key == GDK_KEY_ISO_Left_Tab || key == GDK_KEY_KP_Tab) && (mods & ~GDK_SHIFT_MASK) == 0) {
        GtkWidget *f = gtk_window_get_focus(GTK_WINDOW(nx_ui.win));
        if (f && GTK_IS_LIST_BOX_ROW(f)) {
            bool back = key == GDK_KEY_ISO_Left_Tab || (mods & GDK_SHIFT_MASK);
            GtkWidget *lb = gtk_widget_get_parent(f);
            GtkListBoxRow *last = NULL, *first = gtk_list_box_get_row_at_index(GTK_LIST_BOX(lb), 0);
            for (int i = 0; (f = GTK_WIDGET(gtk_list_box_get_row_at_index(GTK_LIST_BOX(lb), i))); i++) last = GTK_LIST_BOX_ROW(f);
            gtk_widget_grab_focus(GTK_WIDGET(back ? first : last)); /* moves focus, not the selection */
            gtk_widget_child_focus(nx_ui.win, back ? GTK_DIR_TAB_BACKWARD : GTK_DIR_TAB_FORWARD);
            return TRUE;
        }
    }
    for (int64_t i = 0; i < nx_ui.nkeys; i++) {
        if (nx_ui.keys[i].key == key && nx_ui.keys[i].mods == mods) {
            nx_ui_item *it = g_hash_table_lookup(nx_ui.items, &nx_ui.keys[i].id);
            if (it && gtk_widget_is_sensitive(it->w) && gtk_widget_is_visible(it->w))
                nx_ui_push(NX_EV_CLICKED, nx_ui.keys[i].id, 0, NULL);
            return TRUE;
        }
    }
    return FALSE;
}
static void nx_ui_title_button(GtkButton *b, gpointer d) {
    (void)b;
    int what = GPOINTER_TO_INT(d);
    if (what == 0) nx_ui_push(NX_EV_CLOSED, 0, 0, NULL);
    else if (what == 1) {
        if (gtk_window_is_maximized(GTK_WINDOW(nx_ui.win))) gtk_window_unmaximize(GTK_WINDOW(nx_ui.win));
        else gtk_window_maximize(GTK_WINDOW(nx_ui.win));
    } else gtk_window_iconify(GTK_WINDOW(nx_ui.win));
}

/* ---- following the theme live (as apps/video-player does) ---- */
static void nx_ui_reload_theme(void) {
    char *path = g_build_filename(g_get_user_config_dir(), "gtk-3.0", "gtk.css", NULL);
    if (g_file_test(path, G_FILE_TEST_EXISTS))
        gtk_css_provider_load_from_path(nx_ui.live, path, NULL); /* bad file: keep what we have */
    g_free(path);
}
static void nx_ui_on_theme(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent ev, gpointer d) {
    (void)m; (void)f; (void)o; (void)d;
    if (ev == G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT || ev == G_FILE_MONITOR_EVENT_CREATED || ev == G_FILE_MONITOR_EVENT_CHANGED)
        nx_ui_reload_theme();
}

/* ---- blocking work (broker requests) while the window keeps drawing ---- */
typedef struct { void (*fn)(void *); void *arg; gint done; } nx_ui_job;
static gpointer nx_ui_job_thread(gpointer p) {
    nx_ui_job *j = p;
    j->fn(j->arg); /* plain C only: no NexaLang memory, no GTK */
    g_atomic_int_set(&j->done, 1);
    g_main_context_wakeup(NULL);
    return NULL;
}
static void nx_ui_run_blocking(void (*fn)(void *), void *arg) {
    nx_ui_job j = {fn, arg, 0};
    bool was_busy = nx_ui.busy;
    nx_ui.busy = true;
    gtk_widget_set_sensitive(nx_ui.content, FALSE);
    GThread *t = g_thread_new("nexa-wait", nx_ui_job_thread, &j);
    while (!g_atomic_int_get(&j.done)) gtk_main_iteration_do(TRUE);
    g_thread_join(t);
    gtk_widget_set_sensitive(nx_ui.content, TRUE);
    nx_ui.busy = was_busy;
}

/* ---- primitives ---- */

/* Open the window. Returns the id of its main column (always 1). */
NX_RT nx_R_v nx_b___ui_start(nx_str app_id, nx_str title, int line) {
    nx_R_v r = {false, {"", 0, NULL}};
    if (nx_ui.started) nx_trap(line, "ui.start was already called: a program has one window");
    /* GTK must not change the program's locale: numbers would start
       printing with a decimal comma in some languages. */
    gtk_disable_setlocale();
    if (!gtk_init_check(NULL, NULL)) {
        r.e = nx_str_lit("can't open a window: there's no display to show it on (run it from the desktop)", 80);
        return r;
    }
    char *id = nx_ui_cstr(app_id), *t = nx_ui_cstr(title);
    g_set_prgname(id); /* the window's app id, for the desktop and its icon */
    nx_ui.started = true;
    nx_ui.items = g_hash_table_new_full(g_int64_hash, g_int64_equal, NULL, nx_ui_item_free);
    const char *tree = g_getenv("NEXALANG_UI_TREE");
    nx_ui.tree = tree && *tree;

    GtkCssProvider *css = gtk_css_provider_new();
    gtk_css_provider_load_from_data(css, NX_UI_CSS, -1, NULL);
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(css), GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    g_object_unref(css);
    nx_ui.live = gtk_css_provider_new();
    nx_ui_reload_theme();
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(nx_ui.live), GTK_STYLE_PROVIDER_PRIORITY_USER + 1);
    char *path = g_build_filename(g_get_user_config_dir(), "gtk-3.0", "gtk.css", NULL);
    GFile *file = g_file_new_for_path(path);
    GFileMonitor *mon = g_file_monitor_file(file, G_FILE_MONITOR_NONE, NULL, NULL);
    if (mon) g_signal_connect(mon, "changed", G_CALLBACK(nx_ui_on_theme), NULL); /* lives as long as the program */
    g_object_unref(file);
    g_free(path);

    nx_ui.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_icon_name(GTK_WINDOW(nx_ui.win), id);
    gtk_window_set_default_size(GTK_WINDOW(nx_ui.win), 560, 640);
    GtkWidget *bar = gtk_header_bar_new();
    gtk_header_bar_set_show_close_button(GTK_HEADER_BAR(bar), FALSE);
    static const char *const icons[] = {"window-close-symbolic", "window-maximize-symbolic", "window-minimize-symbolic"};
    static const char *const tips[] = {"Close (Ctrl+Q)", "Maximize", "Minimize"};
    static const char *const classes[] = {"close", "maximize", "minimize"};
    for (int i = 0; i < 3; i++) {
        GtkWidget *b = gtk_button_new_from_icon_name(icons[i], GTK_ICON_SIZE_MENU);
        gtk_style_context_add_class(gtk_widget_get_style_context(b), "titlebutton");
        gtk_style_context_add_class(gtk_widget_get_style_context(b), classes[i]);
        gtk_widget_set_tooltip_text(b, tips[i]);
        gtk_widget_set_can_focus(b, FALSE);
        g_signal_connect(b, "clicked", G_CALLBACK(nx_ui_title_button), GINT_TO_POINTER(i));
        gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), b);
    }
    gtk_window_set_titlebar(GTK_WINDOW(nx_ui.win), bar);
    gtk_window_set_title(GTK_WINDOW(nx_ui.win), t);

    GtkWidget *outer = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    GtkWidget *main_col = gtk_box_new(GTK_ORIENTATION_VERTICAL, 12);
    gtk_container_set_border_width(GTK_CONTAINER(main_col), 16);
    gtk_box_pack_start(GTK_BOX(outer), main_col, TRUE, TRUE, 0);
    nx_ui.status = gtk_label_new("");
    gtk_label_set_line_wrap(GTK_LABEL(nx_ui.status), TRUE);
    gtk_label_set_line_wrap_mode(GTK_LABEL(nx_ui.status), PANGO_WRAP_WORD_CHAR);
    gtk_label_set_xalign(GTK_LABEL(nx_ui.status), 0);
    gtk_style_context_add_class(gtk_widget_get_style_context(nx_ui.status), "nx-status");
    gtk_widget_set_no_show_all(nx_ui.status, TRUE);
    gtk_box_pack_end(GTK_BOX(outer), nx_ui.status, FALSE, FALSE, 0);
    gtk_container_add(GTK_CONTAINER(nx_ui.win), outer);
    nx_ui.content = main_col;
    nx_ui.n = 1;
    nx_ui_new_item(main_col, main_col, NX_UI_COLUMN, 0);

    g_signal_connect(nx_ui.win, "delete-event", G_CALLBACK(nx_ui_on_delete), NULL);
    g_signal_connect(nx_ui.win, "key-press-event", G_CALLBACK(nx_ui_on_key), NULL);
    gtk_widget_show_all(nx_ui.win);
    nx_run_blocking = nx_ui_run_blocking;
    g_free(id);
    g_free(t);
    r.ok = true;
    return r;
}

/* Add a new widget to `parent` (a column or a row). */
static int64_t nx_ui_add(int64_t parent, GtkWidget *w, GtkWidget *outer, int kind, int style, int line) {
    nx_ui_item *p = nx_ui_want(parent, 1u << NX_UI_COLUMN | 1u << NX_UI_ROW, "hold other widgets (use a column or a row)", line);
    if (g_hash_table_size(nx_ui.items) >= NX_UI_MAX_WIDGETS) nx_trap(line, "a window can have at most %d widgets; use a list for many items", NX_UI_MAX_WIDGETS);
    bool in_row = p->kind == NX_UI_ROW;
    GtkWidget *box = p->w;
    if (nx_ui.n >= G_MAXINT) nx_trap(line, "this window has made too many widgets");
    int64_t id = nx_ui_new_item(w, outer, kind, style)->id;
    /* lists, code, text fields in a row and columns in a row take the free space */
    bool grow = kind == NX_UI_LIST || kind == NX_UI_CODE || (in_row && (kind == NX_UI_ENTRY || kind == NX_UI_COLUMN));
    if (!in_row && kind == NX_UI_BUTTON) gtk_widget_set_halign(outer, GTK_ALIGN_START);
    if (in_row && kind != NX_UI_LIST) gtk_widget_set_valign(outer, GTK_ALIGN_CENTER);
    gtk_box_pack_start(GTK_BOX(box), outer, grow, TRUE, 0);
    gtk_widget_show_all(outer);
    /* a list or code view in a row or column makes that row or column take
       the free height too, all the way up to the window (0.7) */
    if (kind == NX_UI_LIST || kind == NX_UI_CODE) {
        GtkWidget *child = box;
        while (child != nx_ui.content) {
            GtkWidget *up = gtk_widget_get_parent(child);
            if (!up || !GTK_IS_BOX(up)) break;
            if (gtk_orientable_get_orientation(GTK_ORIENTABLE(up)) == GTK_ORIENTATION_VERTICAL)
                gtk_container_child_set(GTK_CONTAINER(up), child, "expand", TRUE, NULL);
            else
                gtk_widget_set_valign(child, GTK_ALIGN_FILL);
            child = up;
        }
    }
    return id;
}

NX_RT int64_t nx_b___ui_box(int64_t parent, bool across, int line) {
    nx_ui_need_start(line);
    GtkWidget *b = gtk_box_new(across ? GTK_ORIENTATION_HORIZONTAL : GTK_ORIENTATION_VERTICAL, 8);
    return nx_ui_add(parent, b, b, across ? NX_UI_ROW : NX_UI_COLUMN, 0, line);
}

/* style: 0 text, 1 heading, 2 note, 3 mono (numbers that update), 4 display (one big number) */
NX_RT int64_t nx_b___ui_label(int64_t parent, nx_str text, int64_t style, int line) {
    nx_ui_need_start(line);
    char *c = nx_ui_cstr(text);
    GtkWidget *l = gtk_label_new(c);
    g_free(c);
    gtk_label_set_xalign(GTK_LABEL(l), 0);
    gtk_label_set_line_wrap(GTK_LABEL(l), TRUE);
    gtk_label_set_line_wrap_mode(GTK_LABEL(l), PANGO_WRAP_WORD_CHAR);
    static const char *const cls[] = {NULL, "nx-heading", "nx-note", "nx-mono", "nx-display"};
    if (style > 0 && style < 5) gtk_style_context_add_class(gtk_widget_get_style_context(l), cls[style]);
    return nx_ui_add(parent, l, l, NX_UI_LABEL, (int)style, line);
}

/* style: 0 normal, 1 primary (one per window), 2 danger */
NX_RT int64_t nx_b___ui_button(int64_t parent, nx_str text, int64_t style, int line) {
    nx_ui_need_start(line);
    if (style == 1) {
        if (nx_ui.has_primary) nx_trap(line, "a window has one primary action; use ui.button for the others");
        nx_ui.has_primary = true;
    }
    char *c = nx_ui_cstr(text);
    GtkWidget *b = gtk_button_new_with_label(c);
    g_free(c);
    GtkWidget *l = gtk_bin_get_child(GTK_BIN(b));
    if (GTK_IS_LABEL(l)) gtk_label_set_ellipsize(GTK_LABEL(l), PANGO_ELLIPSIZE_END);
    if (style == 1) gtk_style_context_add_class(gtk_widget_get_style_context(b), "nx-primary");
    if (style == 2) gtk_style_context_add_class(gtk_widget_get_style_context(b), "nx-danger");
    int64_t id = nx_ui_add(parent, b, b, NX_UI_BUTTON, (int)style, line);
    nx_ui_get(id, line)->sig = g_signal_connect(b, "clicked", G_CALLBACK(nx_ui_on_clicked), GINT_TO_POINTER((int)id));
    return id;
}

NX_RT int64_t nx_b___ui_entry(int64_t parent, nx_str placeholder, int line) {
    nx_ui_need_start(line);
    GtkWidget *e = gtk_entry_new();
    char *c = nx_ui_cstr(placeholder);
    gtk_entry_set_placeholder_text(GTK_ENTRY(e), c);
    g_free(c);
    gtk_entry_set_width_chars(GTK_ENTRY(e), 8);
    int64_t id = nx_ui_add(parent, e, e, NX_UI_ENTRY, 0, line);
    nx_ui_get(id, line)->sig = g_signal_connect(e, "changed", G_CALLBACK(nx_ui_on_changed), GINT_TO_POINTER((int)id));
    g_signal_connect(e, "activate", G_CALLBACK(nx_ui_on_activate), GINT_TO_POINTER((int)id));
    return id;
}

NX_RT int64_t nx_b___ui_check(int64_t parent, nx_str text, bool on, int line) {
    nx_ui_need_start(line);
    char *c = nx_ui_cstr(text);
    GtkWidget *b = gtk_check_button_new_with_label(c);
    g_free(c);
    GtkWidget *l = gtk_bin_get_child(GTK_BIN(b));
    if (GTK_IS_LABEL(l)) {
        gtk_label_set_line_wrap(GTK_LABEL(l), TRUE);
        gtk_label_set_line_wrap_mode(GTK_LABEL(l), PANGO_WRAP_WORD_CHAR);
    }
    gtk_toggle_button_set_active(GTK_TOGGLE_BUTTON(b), on);
    int64_t id = nx_ui_add(parent, b, b, NX_UI_CHECK, 0, line);
    nx_ui_get(id, line)->sig = g_signal_connect(b, "toggled", G_CALLBACK(nx_ui_on_toggled), GINT_TO_POINTER((int)id));
    return id;
}

/* A scrolling list of text rows. `empty` is shown while it has none. */
NX_RT int64_t nx_b___ui_list(int64_t parent, nx_str empty, int line) {
    nx_ui_need_start(line);
    GtkWidget *sc = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(sc), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    gtk_scrolled_window_set_min_content_height(GTK_SCROLLED_WINDOW(sc), 120);
    gtk_style_context_add_class(gtk_widget_get_style_context(sc), "nx-list");
    GtkWidget *lb = gtk_list_box_new();
    gtk_list_box_set_selection_mode(GTK_LIST_BOX(lb), GTK_SELECTION_SINGLE);
    gtk_list_box_set_activate_on_single_click(GTK_LIST_BOX(lb), FALSE);
    char *c = nx_ui_cstr(empty);
    GtkWidget *ph = gtk_label_new(c);
    g_free(c);
    gtk_label_set_line_wrap(GTK_LABEL(ph), TRUE);
    gtk_label_set_line_wrap_mode(GTK_LABEL(ph), PANGO_WRAP_WORD_CHAR);
    gtk_label_set_justify(GTK_LABEL(ph), GTK_JUSTIFY_CENTER);
    gtk_style_context_add_class(gtk_widget_get_style_context(ph), "nx-empty");
    gtk_widget_show(ph);
    gtk_list_box_set_placeholder(GTK_LIST_BOX(lb), ph);
    g_object_set_data(G_OBJECT(lb), "nx-empty", ph);
    gtk_container_add(GTK_CONTAINER(sc), lb);
    int64_t id = nx_ui_add(parent, lb, sc, NX_UI_LIST, 0, line);
    nx_ui_get(id, line)->sig = g_signal_connect(lb, "row-selected", G_CALLBACK(nx_ui_on_row_selected), GINT_TO_POINTER((int)id));
    g_signal_connect(lb, "row-activated", G_CALLBACK(nx_ui_on_row_activated), GINT_TO_POINTER((int)id));
    return id;
}

static void nx_ui_code_set(GtkWidget *tv, nx_str text) {
    GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(tv));
    char *c = nx_ui_cstr(text);
    gtk_text_buffer_set_text(b, c, -1);
    g_free(c);
    GtkTextIter s, e;
    gtk_text_buffer_get_bounds(b, &s, &e);
    gtk_text_buffer_apply_tag_by_name(b, "nx-plain", &s, &e);
}

/* Read-only text in JetBrains Mono that scrolls: code, logs (0.7). The
   person can select and copy it; Tab moves on to the next widget. */
NX_RT int64_t nx_b___ui_code(int64_t parent, nx_str text, int line) {
    nx_ui_need_start(line);
    GtkWidget *sc = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(sc), GTK_POLICY_AUTOMATIC, GTK_POLICY_AUTOMATIC);
    gtk_scrolled_window_set_min_content_height(GTK_SCROLLED_WINDOW(sc), 160);
    gtk_style_context_add_class(gtk_widget_get_style_context(sc), "nx-code");
    GtkWidget *tv = gtk_text_view_new();
    gtk_text_view_set_editable(GTK_TEXT_VIEW(tv), FALSE);
    gtk_text_view_set_accepts_tab(GTK_TEXT_VIEW(tv), FALSE);
    gtk_text_view_set_monospace(GTK_TEXT_VIEW(tv), TRUE);
    gtk_text_view_set_left_margin(GTK_TEXT_VIEW(tv), 12);
    gtk_text_view_set_right_margin(GTK_TEXT_VIEW(tv), 12);
    gtk_text_view_set_top_margin(GTK_TEXT_VIEW(tv), 8);
    gtk_text_view_set_bottom_margin(GTK_TEXT_VIEW(tv), 8);
    /* no ligatures: what's shown is what's typed (-> stays ->, == stays ==) */
    gtk_text_buffer_create_tag(gtk_text_view_get_buffer(GTK_TEXT_VIEW(tv)), "nx-plain", "font-features", "-calt, -liga", NULL);
    gtk_container_add(GTK_CONTAINER(sc), tv);
    nx_ui_code_set(tv, text);
    return nx_ui_add(parent, tv, sc, NX_UI_CODE, 0, line);
}

#define NX_UI_TEXTUAL (1u << NX_UI_LABEL | 1u << NX_UI_BUTTON | 1u << NX_UI_ENTRY | 1u << NX_UI_CHECK | 1u << NX_UI_CODE)

/* Changes made by the program never come back as events. */
NX_RT void nx_b___ui_set_text(int64_t id, nx_str text, int line) {
    nx_ui_item *it = nx_ui_want(id, NX_UI_TEXTUAL, "show text", line);
    char *c = nx_ui_cstr(text);
    switch (it->kind) {
    case NX_UI_LABEL: gtk_label_set_text(GTK_LABEL(it->w), c); break;
    case NX_UI_ENTRY:
        g_signal_handler_block(it->w, it->sig);
        gtk_entry_set_text(GTK_ENTRY(it->w), c);
        g_signal_handler_unblock(it->w, it->sig);
        break;
    case NX_UI_CODE: nx_ui_code_set(it->w, text); break;
    default: gtk_button_set_label(GTK_BUTTON(it->w), c); break;
    }
    g_free(c);
}

NX_RT nx_str nx_b___ui_get_text(int64_t id, int line) {
    nx_ui_item *it = nx_ui_want(id, NX_UI_TEXTUAL, "have text", line);
    switch (it->kind) {
    case NX_UI_LABEL: return nx_ui_str(gtk_label_get_text(GTK_LABEL(it->w)));
    case NX_UI_ENTRY: return nx_ui_str(gtk_entry_get_text(GTK_ENTRY(it->w)));
    case NX_UI_CODE: {
        GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
        GtkTextIter s, e;
        gtk_text_buffer_get_bounds(b, &s, &e);
        char *t = gtk_text_buffer_get_text(b, &s, &e, TRUE);
        nx_str r = nx_ui_str(t);
        g_free(t);
        return r;
    }
    default: return nx_ui_str(gtk_button_get_label(GTK_BUTTON(it->w)));
    }
}

NX_RT void nx_b___ui_set_checked(int64_t id, bool on, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_CHECK, "be checked", line);
    g_signal_handler_block(it->w, it->sig);
    gtk_toggle_button_set_active(GTK_TOGGLE_BUTTON(it->w), on);
    g_signal_handler_unblock(it->w, it->sig);
}

NX_RT bool nx_b___ui_is_checked(int64_t id, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_CHECK, "be checked", line);
    return gtk_toggle_button_get_active(GTK_TOGGLE_BUTTON(it->w));
}

/* Replace a list's rows. The selection is cleared. */
NX_RT void nx_b___ui_set_items(int64_t id, nx_arr *items, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_LIST, "hold items", line);
    g_signal_handler_block(it->w, it->sig);
    /* a keyboard user working in the list stays in it, near the same row */
    GtkWidget *f = gtk_window_get_focus(GTK_WINDOW(nx_ui.win));
    int focus_at = f && GTK_IS_LIST_BOX_ROW(f) && gtk_widget_get_parent(f) == it->w ? gtk_list_box_row_get_index(GTK_LIST_BOX_ROW(f)) : -1;
    GList *old = gtk_container_get_children(GTK_CONTAINER(it->w));
    for (GList *l = old; l; l = l->next) gtk_widget_destroy(GTK_WIDGET(l->data));
    g_list_free(old);
    for (int64_t i = 0; i < items->len; i++) {
        char *c = nx_ui_cstr(*(nx_str *)(items->data + i * items->esz));
        GtkWidget *lab = gtk_label_new(c);
        g_free(c);
        gtk_label_set_xalign(GTK_LABEL(lab), 0);
        gtk_label_set_ellipsize(GTK_LABEL(lab), PANGO_ELLIPSIZE_END);
        gtk_label_set_single_line_mode(GTK_LABEL(lab), TRUE);
        gtk_widget_show(lab);
        gtk_list_box_insert(GTK_LIST_BOX(it->w), lab, -1);
    }
    if (focus_at >= 0) {
        int64_t at = focus_at < items->len ? focus_at : items->len - 1;
        if (at >= 0) gtk_widget_grab_focus(GTK_WIDGET(gtk_list_box_get_row_at_index(GTK_LIST_BOX(it->w), (int)at)));
        else gtk_widget_child_focus(nx_ui.win, GTK_DIR_TAB_FORWARD);
    }
    g_signal_handler_unblock(it->w, it->sig);
}

/* The selected row, or -1. */
NX_RT int64_t nx_b___ui_selected(int64_t id, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_LIST, "have a selection", line);
    GtkListBoxRow *row = gtk_list_box_get_selected_row(GTK_LIST_BOX(it->w));
    return row ? gtk_list_box_row_get_index(row) : -1;
}

/* Select row `index`, or nothing when it's -1. */
NX_RT void nx_b___ui_select(int64_t id, int64_t index, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_LIST, "have a selection", line);
    GList *rows = gtk_container_get_children(GTK_CONTAINER(it->w));
    int64_t n = (int64_t)g_list_length(rows);
    g_list_free(rows);
    if (index < -1 || index >= n) nx_trap(line, "row %" PRId64 " is outside the list (it has %" PRId64 ")", index, n);
    g_signal_handler_block(it->w, it->sig);
    if (index < 0) gtk_list_box_unselect_all(GTK_LIST_BOX(it->w));
    else gtk_list_box_select_row(GTK_LIST_BOX(it->w), gtk_list_box_get_row_at_index(GTK_LIST_BOX(it->w), (int)index));
    g_signal_handler_unblock(it->w, it->sig);
}

NX_RT void nx_b___ui_set_enabled(int64_t id, bool on, int line) {
    gtk_widget_set_sensitive(nx_ui_get(id, line)->outer, on);
}

NX_RT void nx_b___ui_set_visible(int64_t id, bool on, int line) {
    if (id == 1) nx_trap(line, "the window's main column can't be hidden");
    gtk_widget_set_visible(nx_ui_get(id, line)->outer, on);
}

NX_RT void nx_b___ui_focus(int64_t id, int line) {
    nx_ui_item *it = nx_ui_get(id, line);
    if (it->kind == NX_UI_LIST) {
        /* into the list: its selected row, or the first (0.7) */
        GtkListBoxRow *row = gtk_list_box_get_selected_row(GTK_LIST_BOX(it->w));
        if (!row) row = gtk_list_box_get_row_at_index(GTK_LIST_BOX(it->w), 0);
        if (row) {
            gtk_widget_grab_focus(GTK_WIDGET(row));
            return;
        }
    }
    gtk_widget_grab_focus(it->w);
}

static void nx_ui_update_tip(nx_ui_item *it) {
    char *t;
    if (it->tip && it->keys) t = g_strdup_printf("%s (%s)", it->tip, it->keys);
    else t = g_strdup(it->tip ? it->tip : it->keys);
    gtk_widget_set_tooltip_text(it->outer, t);
    g_free(t);
}

NX_RT void nx_b___ui_tooltip(int64_t id, nx_str text, int line) {
    nx_ui_item *it = nx_ui_get(id, line);
    g_free(it->tip);
    it->tip = text.len ? nx_ui_cstr(text) : NULL;
    nx_ui_update_tip(it);
}

/* A key that clicks a button: "Ctrl+N", "Ctrl+Shift+S", "F5", "Delete". */
NX_RT void nx_b___ui_shortcut(int64_t id, nx_str keys, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_BUTTON, "have a shortcut (only buttons can)", line);
    char *c = nx_ui_cstr(keys);
    GString *acc = g_string_new(NULL);
    char **parts = g_strsplit(c, "+", -1);
    int n = (int)g_strv_length(parts);
    bool bad = n == 0;
    for (int i = 0; i < n && !bad; i++) {
        char *p = g_strstrip(parts[i]);
        if (i < n - 1) {
            if (!g_ascii_strcasecmp(p, "ctrl")) g_string_append(acc, "<Control>");
            else if (!g_ascii_strcasecmp(p, "shift")) g_string_append(acc, "<Shift>");
            else if (!g_ascii_strcasecmp(p, "alt")) g_string_append(acc, "<Alt>");
            else if (!g_ascii_strcasecmp(p, "super")) g_string_append(acc, "<Super>");
            else bad = true;
        } else if (strlen(p) == 1) {
            g_string_append_c(acc, g_ascii_tolower(p[0]));
        } else {
            g_string_append(acc, p);
        }
    }
    g_strfreev(parts);
    guint key = 0;
    GdkModifierType mods = 0;
    if (!bad) gtk_accelerator_parse(acc->str, &key, &mods);
    g_string_free(acc, TRUE);
    if (key == 0) {
        nx_trap(line, "`%s` isn't a key ui knows; write it like \"Ctrl+N\", \"Ctrl+Shift+S\", \"F5\" or \"Delete\"", c);
    }
    key = gdk_keyval_to_lower(key);
    if ((key == GDK_KEY_Escape && mods == 0) || (key == GDK_KEY_q && mods == GDK_CONTROL_MASK))
        nx_trap(line, "%s already closes the window", c);
    for (int64_t i = 0; i < nx_ui.nkeys; i++)
        if (nx_ui.keys[i].key == key && nx_ui.keys[i].mods == mods)
            nx_trap(line, "%s is already the shortcut of widget %" PRId64, c, nx_ui.keys[i].id);
    g_free(c);
    if (nx_ui.nkeys == nx_ui.kcap) {
        nx_ui.kcap = nx_ui.kcap ? nx_ui.kcap * 2 : 8;
        nx_ui.keys = g_realloc_n(nx_ui.keys, (gsize)nx_ui.kcap, sizeof *nx_ui.keys);
    }
    nx_ui.keys[nx_ui.nkeys++] = (nx_ui_key){key, mods, id};
    g_free(it->keys);
    it->keys = gtk_accelerator_get_label(key, mods);
    nx_ui_update_tip(it);
}

/* ---- removing widgets (0.7) ---- */

/* Forget `outer`'s widget and everything inside it: their numbers stop
   working, their shortcuts and waiting events go, and a primary button
   can be made again. GTK destroys the widgets afterwards. */
static void nx_ui_forget(GtkWidget *outer) {
    int64_t id = GPOINTER_TO_INT(g_object_get_data(G_OBJECT(outer), "nx-id"));
    nx_ui_item *it = id ? g_hash_table_lookup(nx_ui.items, &id) : NULL;
    if (!it) return;
    if (it->kind == NX_UI_COLUMN || it->kind == NX_UI_ROW) {
        GList *kids = gtk_container_get_children(GTK_CONTAINER(it->w));
        for (GList *l = kids; l; l = l->next) nx_ui_forget(GTK_WIDGET(l->data));
        g_list_free(kids);
    }
    int64_t k = 0;
    for (int64_t i = 0; i < nx_ui.nkeys; i++)
        if (nx_ui.keys[i].id != id) nx_ui.keys[k++] = nx_ui.keys[i];
    nx_ui.nkeys = k;
    int64_t q = nx_ui.qhead;
    for (int64_t i = nx_ui.qhead; i < nx_ui.qlen; i++) {
        if (nx_ui.q[i].id == id && nx_ui.q[i].kind != NX_EV_CLOSED && nx_ui.q[i].kind != NX_EV_TICK) g_free(nx_ui.q[i].text);
        else nx_ui.q[q++] = nx_ui.q[i];
    }
    nx_ui.qlen = q;
    if (it->kind == NX_UI_BUTTON && it->style == 1) nx_ui.has_primary = false;
    g_hash_table_remove(nx_ui.items, &id); /* frees it */
}

/* clear: remove what's inside a column or row; otherwise remove the widget. */
NX_RT void nx_b___ui_remove(int64_t id, bool inside_only, int line) {
    nx_ui_item *it = inside_only ? nx_ui_want(id, 1u << NX_UI_COLUMN | 1u << NX_UI_ROW, "be cleared (only columns and rows hold widgets)", line)
                                 : nx_ui_get(id, line);
    if (!inside_only && id == 1) nx_trap(line, "the window's main column can't be removed (ui.clear(page) empties it)");
    if (inside_only) {
        GList *kids = gtk_container_get_children(GTK_CONTAINER(it->w));
        for (GList *l = kids; l; l = l->next) {
            nx_ui_forget(GTK_WIDGET(l->data));
            gtk_widget_destroy(GTK_WIDGET(l->data));
        }
        g_list_free(kids);
    } else {
        GtkWidget *outer = it->outer;
        nx_ui_forget(outer);
        gtk_widget_destroy(outer);
    }
}

/* ---- the clipboard (0.7) ---- */
NX_RT void nx_b___ui_to_clipboard(nx_str text, int line) {
    nx_ui_need_start(line);
    char *c = nx_ui_cstr(text);
    gtk_clipboard_set_text(gtk_clipboard_get(GDK_SELECTION_CLIPBOARD), c, -1);
    g_free(c);
}

/* ---- choosing a file (0.7) ----
   The file chooser runs in the program, so an installed app can only use
   what nexa-run lets it (its own folders and its files.* folders); it
   starts in Documents. `name`: a suggested file name (saving) or a
   pattern like "*.nx" (opening; "" for every file). A name that isn't
   UTF-8 can't be NexaLang text, so the chooser says so and stays open. */
/* GTK's chooser sizes itself from settings that may be missing (and
   then comes out bigger than a small screen); keep it to the window it
   belongs to, or 720 x 480. */
static void nx_ui_chooser_size(GtkWidget *d, gpointer p) {
    (void)p;
    int w = 720, h = 480, pw = 0, ph = 0;
    gtk_window_get_size(GTK_WINDOW(nx_ui.win), &pw, &ph);
    GdkRectangle area = {0, 0, 0, 0};
    GdkMonitor *m = gdk_display_get_monitor_at_window(gtk_widget_get_display(d), gtk_widget_get_window(nx_ui.win));
    if (m) gdk_monitor_get_workarea(m, &area);
    if (area.width > 0 && w > area.width - 32) w = area.width - 32;
    if (area.height > 0 && h > area.height - 32) h = area.height - 32;
    w = w < 360 ? 360 : w;
    h = h < 320 ? 320 : h;
    gtk_window_resize(GTK_WINDOW(d), w, h);
    /* over the middle of its window (where the desktop lets programs place
       windows; on Wayland the desktop decides) */
    int px = 0, py = 0;
    gtk_window_get_position(GTK_WINDOW(nx_ui.win), &px, &py);
    int x = px + (pw - w) / 2, y = py + (ph - h) / 2;
    gtk_window_move(GTK_WINDOW(d), x < area.x ? area.x : x, y < area.y ? area.y : y);
}

NX_RT nx_M_s nx_b___ui_choose(nx_str title, nx_str name, bool save, int line) {
    nx_ui_need_start(line);
    nx_M_s r = {false, {"", 0, NULL}};
    char *t = nx_ui_cstr(title), *n = nx_ui_cstr(name);
    GtkWidget *d = gtk_file_chooser_dialog_new(t, GTK_WINDOW(nx_ui.win), save ? GTK_FILE_CHOOSER_ACTION_SAVE : GTK_FILE_CHOOSER_ACTION_OPEN,
                                               "Cancel", GTK_RESPONSE_CANCEL, save ? "Save" : "Open", GTK_RESPONSE_ACCEPT, NULL);
    GtkWidget *bar = gtk_header_bar_new();
    gtk_header_bar_set_title(GTK_HEADER_BAR(bar), t);
    gtk_window_set_titlebar(GTK_WINDOW(d), bar);
    gtk_widget_show(bar);
    gtk_window_set_modal(GTK_WINDOW(d), TRUE);
    g_signal_connect_after(d, "map", G_CALLBACK(nx_ui_chooser_size), NULL);
    GtkWidget *ok = gtk_dialog_get_widget_for_response(GTK_DIALOG(d), GTK_RESPONSE_ACCEPT);
    gtk_style_context_add_class(gtk_widget_get_style_context(ok), "nx-primary");
    gtk_dialog_set_default_response(GTK_DIALOG(d), GTK_RESPONSE_ACCEPT);
    GtkFileChooser *fc = GTK_FILE_CHOOSER(d);
    gtk_file_chooser_set_local_only(fc, TRUE);
    gtk_file_chooser_set_do_overwrite_confirmation(fc, TRUE);
    const char *docs = g_get_user_special_dir(G_USER_DIRECTORY_DOCUMENTS);
    char *fallback = g_build_filename(g_get_home_dir(), "Documents", NULL);
    if (docs && g_file_test(docs, G_FILE_TEST_IS_DIR)) gtk_file_chooser_set_current_folder(fc, docs);
    else if (g_file_test(fallback, G_FILE_TEST_IS_DIR)) gtk_file_chooser_set_current_folder(fc, fallback);
    else gtk_file_chooser_set_current_folder(fc, g_get_home_dir());
    g_free(fallback);
    if (save && *n) gtk_file_chooser_set_current_name(fc, n);
    if (!save && *n) {
        GtkFileFilter *f = gtk_file_filter_new();
        gtk_file_filter_set_name(f, n);
        gtk_file_filter_add_pattern(f, n);
        gtk_file_chooser_add_filter(fc, f);
    }
    GtkWidget *bad = gtk_label_new("");
    gtk_label_set_xalign(GTK_LABEL(bad), 0);
    gtk_label_set_line_wrap(GTK_LABEL(bad), TRUE);
    gtk_style_context_add_class(gtk_widget_get_style_context(bad), "nx-error");
    gtk_widget_set_no_show_all(bad, TRUE);
    gtk_file_chooser_set_extra_widget(fc, bad);
    g_free(t);
    g_free(n);
    bool was_busy = nx_ui.busy;
    nx_ui.busy = true;
    while (gtk_dialog_run(GTK_DIALOG(d)) == GTK_RESPONSE_ACCEPT) {
        char *path = gtk_file_chooser_get_filename(fc);
        if (path && g_utf8_validate(path, -1, NULL)) {
            r.has = true;
            r.v = nx_ui_str(path);
            g_free(path);
            break;
        }
        g_free(path);
        gtk_label_set_text(GTK_LABEL(bad), "That name has characters NexaOS can't read here. Pick another file or name.");
        gtk_widget_show(bad);
    }
    nx_ui.busy = was_busy;
    gtk_widget_destroy(d);
    return r;
}

/* ---- for tests: the widgets as text (NEXALANG_UI_TREE, Ctrl+Shift+F12) ---- */
static void nx_ui_tree_text(const char *t) {
    putchar('"');
    for (; *t; t++) {
        if (*t == '"' || *t == '\\') printf("\\%c", *t);
        else if (*t == '\n') fputs("\\n", stdout);
        else putchar(*t);
    }
    putchar('"');
}
static void nx_ui_tree(GtkWidget *outer, int depth) {
    int64_t id = GPOINTER_TO_INT(g_object_get_data(G_OBJECT(outer), "nx-id"));
    nx_ui_item *it = id ? g_hash_table_lookup(nx_ui.items, &id) : NULL;
    if (!it || !gtk_widget_get_visible(outer)) return;
    printf("%*s", depth * 2, "");
    static const char *const labels[] = {"text", "heading", "note", "mono", "display"};
    static const char *const buttons[] = {"button", "primary button", "danger button"};
    switch (it->kind) {
    case NX_UI_COLUMN: case NX_UI_ROW: fputs(it->kind == NX_UI_ROW ? "row" : "column", stdout); break;
    case NX_UI_LABEL: printf("%s ", labels[it->style]); nx_ui_tree_text(gtk_label_get_text(GTK_LABEL(it->w))); break;
    case NX_UI_BUTTON: printf("%s ", buttons[it->style]); nx_ui_tree_text(gtk_button_get_label(GTK_BUTTON(it->w))); break;
    case NX_UI_ENTRY:
        fputs("field ", stdout);
        nx_ui_tree_text(gtk_entry_get_placeholder_text(GTK_ENTRY(it->w)));
        if (gtk_entry_get_text_length(GTK_ENTRY(it->w))) { fputs(" = ", stdout); nx_ui_tree_text(gtk_entry_get_text(GTK_ENTRY(it->w))); }
        break;
    case NX_UI_CHECK:
        printf("check %s ", gtk_toggle_button_get_active(GTK_TOGGLE_BUTTON(it->w)) ? "on" : "off");
        nx_ui_tree_text(gtk_button_get_label(GTK_BUTTON(it->w)));
        break;
    case NX_UI_LIST: {
        GList *rows = gtk_container_get_children(GTK_CONTAINER(it->w));
        fputs("list ", stdout);
        nx_ui_tree_text(gtk_label_get_text(GTK_LABEL(g_object_get_data(G_OBJECT(it->w), "nx-empty"))));
        printf(" (%u rows)", g_list_length(rows));
        GtkListBoxRow *sel = gtk_list_box_get_selected_row(GTK_LIST_BOX(it->w));
        for (GList *l = rows; l; l = l->next) {
            GtkWidget *lab = gtk_bin_get_child(GTK_BIN(l->data));
            printf("\n%*s%s ", depth * 2 + 2, "", l->data == (gpointer)sel ? "*" : "-");
            nx_ui_tree_text(GTK_IS_LABEL(lab) ? gtk_label_get_text(GTK_LABEL(lab)) : "");
        }
        g_list_free(rows);
        break;
    }
    default: printf("code (%d lines)", gtk_text_buffer_get_line_count(gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w)))); break;
    }
    if (!gtk_widget_get_sensitive(outer)) fputs(" (off)", stdout);
    putchar('\n');
    if (it->kind == NX_UI_COLUMN || it->kind == NX_UI_ROW) {
        GList *kids = gtk_container_get_children(GTK_CONTAINER(it->w));
        for (GList *l = kids; l; l = l->next) nx_ui_tree(GTK_WIDGET(l->data), depth + 1);
        g_list_free(kids);
    }
}

/* The status line at the bottom: plain (style 0) or an error (1). Empty
   text hides it. */
NX_RT void nx_b___ui_status(nx_str text, int64_t style, int line) {
    nx_ui_need_start(line);
    char *c = nx_ui_cstr(text);
    gtk_label_set_text(GTK_LABEL(nx_ui.status), c);
    g_free(c);
    GtkStyleContext *sc = gtk_widget_get_style_context(nx_ui.status);
    if (style == 1) gtk_style_context_add_class(sc, "nx-error");
    else gtk_style_context_remove_class(sc, "nx-error");
    gtk_widget_set_visible(nx_ui.status, text.len > 0);
}

static gboolean nx_ui_on_tick(gpointer d) {
    *(guint *)d = 0;
    if (nx_ui.qhead == nx_ui.qlen) nx_ui_push(NX_EV_TICK, 0, 0, NULL);
    return G_SOURCE_REMOVE;
}

/* Wait for the next event; returns its kind. ms: -1 forever, else 1 or more. */
NX_RT int64_t nx_b___ui_wait(int64_t ms, int line) {
    nx_ui_need_start(line);
    if (ms == 0 || ms < -1) nx_trap(line, "ui.wait_ms needs a time of at least 1 ms, not %" PRId64, ms);
    g_free(nx_ui.cur.text);
    nx_ui.cur.text = NULL;
    guint timer = 0;
    if (ms > 0 && nx_ui.qhead == nx_ui.qlen)
        timer = g_timeout_add(ms > G_MAXINT ? G_MAXINT : (guint)ms, nx_ui_on_tick, &timer);
    while (nx_ui.qhead == nx_ui.qlen) gtk_main_iteration_do(TRUE);
    if (timer) g_source_remove(timer);
    nx_ui.cur = nx_ui.q[nx_ui.qhead++];
    return nx_ui.cur.kind;
}
NX_RT int64_t nx_b___ui_event_id(int line) { (void)line; return nx_ui.cur.id; }
NX_RT int64_t nx_b___ui_event_num(int line) { (void)line; return nx_ui.cur.num; }
NX_RT nx_str nx_b___ui_event_text(int line) { (void)line; return nx_ui_str(nx_ui.cur.text); }

/* A dialog with a heading, a message and two buttons; the content area is
   returned so the caller can add more. */
static GtkWidget *nx_ui_dialog(const char *heading, const char *message, const char *yes, bool danger, GtkWidget **box) {
    GtkWidget *d = gtk_dialog_new();
    gtk_window_set_transient_for(GTK_WINDOW(d), GTK_WINDOW(nx_ui.win));
    gtk_window_set_modal(GTK_WINDOW(d), TRUE);
    gtk_window_set_default_size(GTK_WINDOW(d), 400, -1);
    GtkWidget *bar = gtk_header_bar_new();
    gtk_window_set_titlebar(GTK_WINDOW(d), bar);
    gtk_header_bar_set_title(GTK_HEADER_BAR(bar), heading); /* also the window's title */
    gtk_window_set_title(GTK_WINDOW(d), heading);
    GtkWidget *area = gtk_dialog_get_content_area(GTK_DIALOG(d));
    GtkWidget *col = gtk_box_new(GTK_ORIENTATION_VERTICAL, 12);
    gtk_container_set_border_width(GTK_CONTAINER(col), 16);
    gtk_box_pack_start(GTK_BOX(area), col, TRUE, TRUE, 0);
    if (message && *message) {
        GtkWidget *m = gtk_label_new(message);
        gtk_label_set_xalign(GTK_LABEL(m), 0);
        gtk_label_set_line_wrap(GTK_LABEL(m), TRUE);
        gtk_label_set_line_wrap_mode(GTK_LABEL(m), PANGO_WRAP_WORD_CHAR);
        gtk_label_set_max_width_chars(GTK_LABEL(m), 48);
        gtk_box_pack_start(GTK_BOX(col), m, FALSE, FALSE, 0);
    }
    gtk_dialog_add_button(GTK_DIALOG(d), "Cancel", GTK_RESPONSE_CANCEL);
    GtkWidget *ok = gtk_dialog_add_button(GTK_DIALOG(d), yes, GTK_RESPONSE_OK);
    gtk_style_context_add_class(gtk_widget_get_style_context(ok), danger ? "nx-danger" : "nx-primary");
    gtk_dialog_set_default_response(GTK_DIALOG(d), GTK_RESPONSE_OK);
    *box = col;
    return d;
}

/* Ask for the person's password for `what`. `why` is the broker's reason
   ("wrong password" shows a message). none when cancelled. */
NX_RT nx_M_s nx_b___ui_ask_password(nx_str what, nx_str why, int line) {
    nx_ui_need_start(line);
    nx_M_s r = {false, {"", 0, NULL}};
    char *w = nx_ui_cstr(what);
    char *msg = *w ? g_strdup_printf("NexaOS needs your password to do this: %s.", w)
                   : g_strdup("NexaOS needs your password to do this.");
    GtkWidget *col;
    GtkWidget *d = nx_ui_dialog("Enter your password", msg, "Continue", false, &col);
    g_free(msg);
    g_free(w);
    GtkWidget *e = gtk_entry_new();
    gtk_entry_set_visibility(GTK_ENTRY(e), FALSE);
    gtk_entry_set_input_purpose(GTK_ENTRY(e), GTK_INPUT_PURPOSE_PASSWORD);
    gtk_entry_set_activates_default(GTK_ENTRY(e), TRUE);
    gtk_entry_set_placeholder_text(GTK_ENTRY(e), "Password");
    gtk_box_pack_start(GTK_BOX(col), e, FALSE, FALSE, 0);
    if (why.len == 14 && memcmp(why.p, "wrong password", 14) == 0) {
        GtkWidget *bad = gtk_label_new("That password wasn't right. Try again.");
        gtk_label_set_xalign(GTK_LABEL(bad), 0);
        gtk_style_context_add_class(gtk_widget_get_style_context(bad), "nx-error");
        gtk_box_pack_start(GTK_BOX(col), bad, FALSE, FALSE, 0);
    }
    gtk_widget_show_all(d);
    gtk_widget_grab_focus(e);
    bool was_busy = nx_ui.busy;
    nx_ui.busy = true; /* shortcuts of the window underneath do nothing */
    int resp = gtk_dialog_run(GTK_DIALOG(d));
    nx_ui.busy = was_busy;
    if (resp == GTK_RESPONSE_OK) {
        const char *t = gtk_entry_get_text(GTK_ENTRY(e));
        r.has = true;
        r.v = nx_ui_str(t);
    }
    gtk_entry_set_text(GTK_ENTRY(e), ""); /* GTK wipes a hidden entry's old text */
    gtk_widget_destroy(d);
    return r;
}

/* Ask a yes/no question; true when the person picked `yes`. */
NX_RT bool nx_b___ui_confirm(nx_str heading, nx_str message, nx_str yes, bool danger, int line) {
    nx_ui_need_start(line);
    char *h = nx_ui_cstr(heading), *m = nx_ui_cstr(message), *y = nx_ui_cstr(yes);
    GtkWidget *col;
    GtkWidget *d = nx_ui_dialog(h, m, y, danger, &col);
    g_free(h); g_free(m); g_free(y);
    gtk_widget_show_all(d);
    bool was_busy = nx_ui.busy;
    nx_ui.busy = true;
    int resp = gtk_dialog_run(GTK_DIALOG(d));
    nx_ui.busy = was_busy;
    gtk_widget_destroy(d);
    return resp == GTK_RESPONSE_OK;
}
