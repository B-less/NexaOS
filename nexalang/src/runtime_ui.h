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

#include <signal.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <glib-unix.h>

enum { NX_UI_COLUMN = 1, NX_UI_ROW, NX_UI_LABEL, NX_UI_BUTTON, NX_UI_ENTRY, NX_UI_CHECK, NX_UI_LIST, NX_UI_CODE, NX_UI_EDITOR };
enum { NX_EV_CLICKED = 1, NX_EV_CHANGED, NX_EV_SUBMITTED, NX_EV_TOGGLED, NX_EV_SELECTED, NX_EV_ACTIVATED, NX_EV_CLOSED, NX_EV_TICK,
       NX_EV_EDITED, NX_EV_OUTPUT, NX_EV_FINISHED };
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
    bool tree_full;     /* NEXALANG_UI_TREE=full: with the text of code views and editors */
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
    ".nx-code { background-color: @nexa_field; border: 1px solid @nexa_border; border-radius: 10px; padding: 4px; }\n"
    ".nx-code textview, .nx-code text { background-color: @nexa_field; color: @nexa_text_strong; font-family: \"JetBrains Mono\", monospace; }\n"
    ".nx-code text selection { background-color: @nexa_selected; color: @nexa_selected_text; }\n"
    ".nx-code:focus-within, .nx-code textview:focus { border-color: @nexa_accent; }\n"
    /* the code editor (0.8): its colours come from the style scheme (the
       theme's nexa.xml, or the Horizon fallback); no ligatures */
    ".nx-editor { background-color: @nexa_bg_deep; border: 1px solid @nexa_border; border-radius: 10px; padding: 6px; }\n"
    ".nx-editor:focus-within { border-color: @nexa_accent; }\n"
    ".nx-editor textview, .nx-editor text { font-family: \"JetBrains Mono\", monospace; font-size: 10pt; font-feature-settings: \"-liga, -calt\"; }\n"
    ".nx-editor gutter { font-family: \"JetBrains Mono\", monospace; }\n"
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
    case NX_UI_EDITOR: return "a code editor";
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

static void nx_ui_editor_restyle(void);

/* ---- following the theme live (as apps/video-player does) ---- */
static void nx_ui_reload_theme(void) {
    char *path = g_build_filename(g_get_user_config_dir(), "gtk-3.0", "gtk.css", NULL);
    if (g_file_test(path, G_FILE_TEST_EXISTS))
        gtk_css_provider_load_from_path(nx_ui.live, path, NULL); /* bad file: keep what we have */
    g_free(path);
}
static void nx_ui_on_theme(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent ev, gpointer d) {
    (void)m; (void)f; (void)o; (void)d;
    if (ev == G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT || ev == G_FILE_MONITOR_EVENT_CREATED || ev == G_FILE_MONITOR_EVENT_CHANGED) {
        nx_ui_reload_theme();
        nx_ui_editor_restyle(); /* mistakes underlined in the new @nexa_crit (0.8) */
    }
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
    nx_ui.tree_full = nx_ui.tree && strcmp(tree, "full") == 0;

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

/* A list or code view next to a code editor is shorter, so the editor
   and the window's other parts fit in a 640 px window (0.8). */
static void nx_ui_shorter(nx_ui_item *k) {
    GtkWidget *sc = k->outer;
    if (k->kind == NX_UI_CODE) {
        GList *in = gtk_container_get_children(GTK_CONTAINER(k->outer));
        sc = in ? GTK_WIDGET(in->data) : NULL;
        g_list_free(in);
    }
    if (sc && GTK_IS_SCROLLED_WINDOW(sc)) gtk_scrolled_window_set_min_content_height(GTK_SCROLLED_WINDOW(sc), k->kind == NX_UI_LIST ? 80 : 112);
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
    bool grow = kind == NX_UI_LIST || kind == NX_UI_CODE || kind == NX_UI_EDITOR || (in_row && (kind == NX_UI_ENTRY || kind == NX_UI_COLUMN));
    /* a code editor takes the free height of its column; lists and code
       views next to it keep their own height (0.8) */
    if (!in_row && (kind == NX_UI_EDITOR || kind == NX_UI_LIST || kind == NX_UI_CODE)) {
        GList *kids = gtk_container_get_children(GTK_CONTAINER(box));
        for (GList *l = kids; l; l = l->next) {
            int64_t kid = GPOINTER_TO_INT(g_object_get_data(G_OBJECT(l->data), "nx-id"));
            nx_ui_item *k = kid ? g_hash_table_lookup(nx_ui.items, &kid) : NULL;
            if (!k) continue;
            if (kind == NX_UI_EDITOR && (k->kind == NX_UI_LIST || k->kind == NX_UI_CODE)) {
                gtk_container_child_set(GTK_CONTAINER(box), GTK_WIDGET(l->data), "expand", FALSE, NULL);
                nx_ui_shorter(k);
            }
            if (kind != NX_UI_EDITOR && k->kind == NX_UI_EDITOR) {
                grow = false;
                nx_ui_item tmp = {0, w, outer, kind, style, 0, NULL, NULL};
                nx_ui_shorter(&tmp);
            }
        }
        g_list_free(kids);
    }
    if (!in_row && kind == NX_UI_BUTTON) gtk_widget_set_halign(outer, GTK_ALIGN_START);
    if (in_row && kind != NX_UI_LIST) gtk_widget_set_valign(outer, GTK_ALIGN_CENTER);
    gtk_box_pack_start(GTK_BOX(box), outer, grow, TRUE, 0);
    gtk_widget_show_all(outer);
    /* a list or code view in a row or column makes that row or column take
       the free height too, all the way up to the window (0.7) */
    if (kind == NX_UI_LIST || kind == NX_UI_CODE || kind == NX_UI_EDITOR) {
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
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(sc), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    gtk_scrolled_window_set_min_content_height(GTK_SCROLLED_WINDOW(sc), 160);
    GtkWidget *tv = gtk_text_view_new();
    /* long lines wrap: text never goes off the side (0.8; Hexa: text
       wraps or ellipsises, never clips) */
    gtk_text_view_set_wrap_mode(GTK_TEXT_VIEW(tv), GTK_WRAP_WORD_CHAR);
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
    /* a card around it, with room inside the rounded corners (GTK 3
       doesn't clip a child to them; 0.8) */
    GtkWidget *card = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_style_context_add_class(gtk_widget_get_style_context(card), "nx-code");
    gtk_box_pack_start(GTK_BOX(card), sc, TRUE, TRUE, 0);
    return nx_ui_add(parent, tv, card, NX_UI_CODE, 0, line);
}

#define NX_UI_TEXTUAL (1u << NX_UI_LABEL | 1u << NX_UI_BUTTON | 1u << NX_UI_ENTRY | 1u << NX_UI_CHECK | 1u << NX_UI_CODE | 1u << NX_UI_EDITOR)

static void nx_ui_editor_load(nx_ui_item *it, nx_str text); /* the editor's part, below (0.8) */

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
    case NX_UI_EDITOR: nx_ui_editor_load(it, text); break;
    default: gtk_button_set_label(GTK_BUTTON(it->w), c); break;
    }
    g_free(c);
}

NX_RT nx_str nx_b___ui_get_text(int64_t id, int line) {
    nx_ui_item *it = nx_ui_want(id, NX_UI_TEXTUAL, "have text", line);
    switch (it->kind) {
    case NX_UI_LABEL: return nx_ui_str(gtk_label_get_text(GTK_LABEL(it->w)));
    case NX_UI_ENTRY: return nx_ui_str(gtk_entry_get_text(GTK_ENTRY(it->w)));
    case NX_UI_CODE: case NX_UI_EDITOR: {
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
    char *label = gtk_accelerator_get_label(key, mods);
    char *all = it->keys ? g_strdup_printf("%s, %s", it->keys, label) : g_strdup(label); /* a second key for the same button (0.8) */
    g_free(label);
    g_free(it->keys);
    it->keys = all;
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

/* ---- adding to a code view (0.8) ---- */
NX_RT void nx_b___ui_append(int64_t id, nx_str text, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_CODE, "be added to (only code views can: use set_text)", line);
    GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
    char *c = nx_ui_cstr(text);
    GtkTextIter end;
    gtk_text_buffer_get_end_iter(b, &end);
    gtk_text_buffer_insert_with_tags_by_name(b, &end, c, -1, "nx-plain", NULL);
    g_free(c);
    /* show the end (a mark, so it works before the view is laid out) */
    GtkTextMark *m = gtk_text_buffer_get_mark(b, "nx-end");
    gtk_text_buffer_get_end_iter(b, &end);
    if (!m) m = gtk_text_buffer_create_mark(b, "nx-end", &end, FALSE);
    else gtk_text_buffer_move_mark(b, m, &end);
    gtk_text_view_scroll_mark_onscreen(GTK_TEXT_VIEW(it->w), m);
}

/* ---- starting programs (0.8) ----
   A window can start a program with an argument list: never through a
   shell. What it prints (stdout and stderr, in the order it printed them)
   comes back as Output events, then one Finished event with its exit
   code (128 + the signal's number when a signal stopped it). Its input is
   the text given, then end of input.
   - It runs in a process group of its own, so ui.stop_program stops it
     and whatever it started (nexac run stops the program it built too):
     SIGTERM, then SIGKILL two seconds later if it's still running.
   - Output is handed out in whole lines (a line longer than 64 KiB in
     pieces, never cutting a UTF-8 letter in half); zero bytes become
     U+FFFD. After 8 MiB the rest is read and dropped, with a note.
   - At most 8 at once. When the window's program ends, the ones still
     running are stopped.
   Input goes through a socket, written with MSG_NOSIGNAL, so a program
   that stops reading early can't kill this one with SIGPIPE (and nothing
   about signals changes for anything else). */
#define NX_UI_MAX_PROCS 8
#define NX_UI_OUT_CAP ((int64_t)8 << 20)
#define NX_UI_PIECE ((gsize)64 << 10)

typedef struct {
    int64_t id;
    GPid pid;
    int out_fd, in_fd;           /* -1 once closed */
    guint out_watch, in_watch, child_watch, kill_timer, eof_timer;
    char *input; gsize in_len, in_off;
    GString *pend;               /* read, not handed out yet (part of a line) */
    int64_t total;               /* bytes handed out */
    bool capped, exited, stopping;
    int status;
} nx_ui_proc;

static struct {
    GHashTable *all;  /* number -> nx_ui_proc*, only running ones */
    int64_t n;        /* the next number */
    bool hooked;      /* atexit installed */
} nx_procs;

static void nx_ui_proc_finish(nx_ui_proc *p);
static void nx_ui_group_set(GPid pid, bool on);

/* Hand out `len` bytes as one Output event (zero bytes as U+FFFD). */
static void nx_ui_proc_emit(nx_ui_proc *p, const char *b, gsize len) {
    if (!len) return;
    if (p->capped) return;
    if (p->total + (int64_t)len > NX_UI_OUT_CAP) {
        p->capped = true;
        nx_ui_push(NX_EV_OUTPUT, p->id, 0, "\n[The rest of the output was left out: more than 8 MB.]\n");
        return;
    }
    p->total += (int64_t)len;
    GString *t = g_string_sized_new(len + 1);
    for (gsize i = 0; i < len; i++) {
        if (b[i] == 0) g_string_append(t, "\357\277\275");
        else g_string_append_c(t, b[i]);
    }
    nx_ui_push(NX_EV_OUTPUT, p->id, 0, t->str);
    g_string_free(t, TRUE);
}

/* Hand out the whole lines read so far; at the end (`all`), everything. */
static void nx_ui_proc_flush(nx_ui_proc *p, bool all) {
    GString *g = p->pend;
    gsize cut = 0;
    if (all) cut = g->len;
    else {
        for (gsize i = g->len; i > 0; i--)
            if (g->str[i - 1] == '\n') { cut = i; break; }
        if (cut == 0 && g->len >= NX_UI_PIECE) {
            /* a very long line: a piece, ending before an unfinished letter */
            cut = g->len;
            gsize back = 0;
            while (back < 3 && back < cut && ((unsigned char)g->str[cut - 1 - back] & 0xC0) == 0x80) back++;
            if (back < cut) {
                unsigned char lead = (unsigned char)g->str[cut - 1 - back];
                gsize need = lead >= 0xF0 ? 4 : lead >= 0xE0 ? 3 : lead >= 0xC0 ? 2 : 1;
                if (need > back + 1) cut -= back + 1; /* that letter isn't complete yet */
            }
        }
    }
    if (cut) {
        nx_ui_proc_emit(p, g->str, cut);
        g_string_erase(g, 0, (gssize)cut);
    }
}

static void nx_ui_proc_close_out(nx_ui_proc *p) {
    if (p->out_watch) { g_source_remove(p->out_watch); p->out_watch = 0; }
    if (p->out_fd >= 0) { close(p->out_fd); p->out_fd = -1; }
}

static void nx_ui_proc_close_in(nx_ui_proc *p) {
    if (p->in_watch) { g_source_remove(p->in_watch); p->in_watch = 0; }
    if (p->in_fd >= 0) { close(p->in_fd); p->in_fd = -1; }
    g_free(p->input);
    p->input = NULL;
}

/* Read what's there. Returns false at end of output (or an error). */
static bool nx_ui_proc_read(nx_ui_proc *p) {
    char buf[65536];
    for (int rounds = 0; rounds < 16; rounds++) {
        ssize_t n = read(p->out_fd, buf, sizeof buf);
        if (n > 0) {
            g_string_append_len(p->pend, buf, n);
            nx_ui_proc_flush(p, false);
            continue;
        }
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) return true;
        if (n < 0 && errno == EINTR) continue;
        return false; /* 0: end of output; or an error */
    }
    return true; /* more later: let the window draw in between */
}

static gboolean nx_ui_proc_on_out(gint fd, GIOCondition c, gpointer d) {
    (void)fd; (void)c;
    nx_ui_proc *p = d;
    if (nx_ui_proc_read(p)) return G_SOURCE_CONTINUE;
    p->out_watch = 0;
    nx_ui_proc_flush(p, true);
    if (p->out_fd >= 0) { close(p->out_fd); p->out_fd = -1; }
    if (p->exited) nx_ui_proc_finish(p);
    return G_SOURCE_REMOVE;
}

static gboolean nx_ui_proc_on_in(gint fd, GIOCondition c, gpointer d) {
    (void)fd;
    nx_ui_proc *p = d;
    if (!(c & G_IO_ERR)) {
        while (p->in_off < p->in_len) {
            ssize_t n = send(p->in_fd, p->input + p->in_off, p->in_len - p->in_off, MSG_NOSIGNAL | MSG_DONTWAIT);
            if (n > 0) { p->in_off += (gsize)n; continue; }
            if (n < 0 && errno == EINTR) continue;
            if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) return G_SOURCE_CONTINUE;
            break; /* it stopped reading (EPIPE): the rest isn't needed */
        }
    }
    p->in_watch = 0;
    nx_ui_proc_close_in(p);
    return G_SOURCE_REMOVE;
}

/* The program has ended but something it started still holds its output
   open: take what's there and finish anyway. */
static gboolean nx_ui_proc_on_eof_timer(gpointer d) {
    nx_ui_proc *p = d;
    p->eof_timer = 0;
    if (p->out_fd >= 0) nx_ui_proc_read(p);
    nx_ui_proc_flush(p, true);
    nx_ui_proc_close_out(p);
    nx_ui_proc_finish(p);
    return G_SOURCE_REMOVE;
}

static void nx_ui_proc_on_exit(GPid pid, gint status, gpointer d) {
    nx_ui_proc *p = d;
    p->child_watch = 0;
    nx_ui_group_set(pid, false); /* its number may be used again now */
    p->exited = true;
    p->status = status;
    g_spawn_close_pid(pid);
    if (p->out_fd < 0) nx_ui_proc_finish(p);
    else p->eof_timer = g_timeout_add(1000, nx_ui_proc_on_eof_timer, p);
}

/* Both its output and the program are done: say so, and forget it. */
static void nx_ui_proc_finish(nx_ui_proc *p) {
    if (!p->exited || p->out_fd >= 0) return;
    int code = WIFEXITED(p->status) ? WEXITSTATUS(p->status) : WIFSIGNALED(p->status) ? 128 + WTERMSIG(p->status) : 1;
    if (p->kill_timer) g_source_remove(p->kill_timer);
    if (p->eof_timer) g_source_remove(p->eof_timer);
    nx_ui_proc_close_in(p);
    nx_ui_push(NX_EV_FINISHED, p->id, code, NULL);
    g_hash_table_remove(nx_procs.all, &p->id); /* frees it */
}

static void nx_ui_proc_free(gpointer d) {
    nx_ui_proc *p = d;
    g_string_free(p->pend, TRUE);
    g_free(p->input);
    g_free(p);
}

/* In the child, before it starts: a process group of its own (so it can
   be stopped with everything it starts), and the usual SIGPIPE. Only
   async-signal-safe calls. */
static void nx_ui_proc_child(gpointer d) {
    (void)d;
    setpgid(0, 0);
    signal(SIGPIPE, SIG_DFL);
    signal(SIGTERM, SIG_DFL); /* exec resets handlers anyway; be plain about it */
}

/* The process groups still running, for stopping them when this program
   ends: a plain array, so the signal handler below can read it. */
static volatile sig_atomic_t nx_ui_groups[NX_UI_MAX_PROCS];

static void nx_ui_procs_stop_all(void) {
    for (int i = 0; i < NX_UI_MAX_PROCS; i++)
        if (nx_ui_groups[i] > 0) kill(-(pid_t)nx_ui_groups[i], SIGTERM);
}

/* Stopped by a signal (a code editor's Stop, the desktop logging out):
   stop what it started too, then stop the normal way. Only
   async-signal-safe calls. */
static void nx_ui_on_stop_signal(int sig) {
    nx_ui_procs_stop_all();
    signal(sig, SIG_DFL);
    raise(sig);
}

static void nx_ui_group_set(GPid pid, bool on) {
    for (int i = 0; i < NX_UI_MAX_PROCS; i++) {
        if (on && nx_ui_groups[i] == 0) { nx_ui_groups[i] = pid; return; }
        if (!on && nx_ui_groups[i] == pid) { nx_ui_groups[i] = 0; return; }
    }
}

/* Why the last start_program failed (ui.nx turns it into an err). */
static char *nx_ui_proc_why;
NX_RT nx_str nx_b___ui_program_error(int line) {
    (void)line;
    return nx_ui_str(nx_ui_proc_why ? nx_ui_proc_why : "");
}

/* The new program's number, or 0 when it couldn't start (and why in
   nx_ui_proc_why): a plain int, so no new C result type is needed. */
NX_RT int64_t nx_b___ui_start_program(nx_str folder, nx_arr *args, nx_str input, int line) {
    nx_ui_need_start(line);
#define NX_UI_PROC_ERR(...) do { g_free(nx_ui_proc_why); nx_ui_proc_why = g_strdup_printf(__VA_ARGS__); return 0; } while (0)
    if (!nx_procs.all) nx_procs.all = g_hash_table_new_full(g_int64_hash, g_int64_equal, NULL, nx_ui_proc_free);
    if (!nx_procs.hooked) {
        atexit(nx_ui_procs_stop_all);
        static const int sigs[] = {SIGTERM, SIGINT, SIGHUP};
        for (int i = 0; i < 3; i++) {
            struct sigaction sa, old;
            memset(&sa, 0, sizeof sa);
            sa.sa_handler = nx_ui_on_stop_signal;
            sigemptyset(&sa.sa_mask);
            /* only where nothing else handles it (an ignored SIGHUP stays ignored) */
            if (sigaction(sigs[i], NULL, &old) == 0 && old.sa_handler == SIG_DFL) sigaction(sigs[i], &sa, NULL);
        }
        nx_procs.hooked = true;
    }
    if (args->len == 0) NX_UI_PROC_ERR("start_program needs the program's name and its arguments: [\"nexac\", \"check\", ...]");
    if (args->len > 4096) NX_UI_PROC_ERR("too many arguments (%" PRId64 "; at most 4096)", args->len);
    if (g_hash_table_size(nx_procs.all) >= NX_UI_MAX_PROCS)
        NX_UI_PROC_ERR("%d programs are already running; wait for one to finish", NX_UI_MAX_PROCS);
    for (int64_t i = 0; i < args->len; i++) {
        nx_str a = *(nx_str *)(args->data + i * args->esz);
        if (memchr(a.p, 0, (size_t)a.len)) NX_UI_PROC_ERR("argument %" PRId64 " has a zero byte in it, which a program can't be given", i + 1);
    }
    nx_str name = *(nx_str *)args->data;
    if (name.len == 0) NX_UI_PROC_ERR("the program's name is empty");
    if (memchr(folder.p, 0, (size_t)folder.len)) NX_UI_PROC_ERR("the folder's name has a zero byte in it");
    char **argv = g_new0(char *, (gsize)args->len + 1);
    for (int64_t i = 0; i < args->len; i++) argv[i] = nx_ui_cstr(*(nx_str *)(args->data + i * args->esz));
    char *dir = folder.len ? nx_ui_cstr(folder) : NULL;
    if (dir && !g_file_test(dir, G_FILE_TEST_IS_DIR)) {
        g_free(nx_ui_proc_why);
        nx_ui_proc_why = g_strdup_printf("can't start `%s`: the folder %s can't be used", argv[0], dir);
        g_strfreev(argv);
        g_free(dir);
        return 0;
    }
    /* nothing of ours waiting in stdout's buffer when the child starts: a
       child that fails before it runs its program must never print it
       a second time (seen under Valgrind) */
    fflush(stdout);
    fflush(stderr);
    int out[2] = {-1, -1}, in[2] = {-1, -1};
    GError *e = NULL;
    GPid pid = 0;
    bool ok = g_unix_open_pipe(out, FD_CLOEXEC, &e) && socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, in) == 0;
    if (ok) {
        ok = g_spawn_async_with_fds(dir, argv, NULL, G_SPAWN_SEARCH_PATH | G_SPAWN_DO_NOT_REAP_CHILD, nx_ui_proc_child, NULL, &pid,
                                    in[1], out[1], out[1], &e);
    } else if (!e) {
        g_set_error_literal(&e, G_SPAWN_ERROR, G_SPAWN_ERROR_FAILED, g_strerror(errno));
    }
    if (out[1] >= 0) close(out[1]);
    if (in[1] >= 0) close(in[1]);
    if (!ok) {
        if (out[0] >= 0) close(out[0]);
        if (in[0] >= 0) close(in[0]);
        char *msg;
        if (g_error_matches(e, G_SPAWN_ERROR, G_SPAWN_ERROR_NOENT))
            msg = g_strdup_printf("can't start `%s`: no program with that name is installed", argv[0]);
        else if (g_error_matches(e, G_SPAWN_ERROR, G_SPAWN_ERROR_ACCES) || g_error_matches(e, G_SPAWN_ERROR, G_SPAWN_ERROR_PERM))
            msg = g_strdup_printf("can't start `%s`: it isn't allowed to run here", argv[0]);
        else if (g_error_matches(e, G_SPAWN_ERROR, G_SPAWN_ERROR_CHDIR))
            msg = g_strdup_printf("can't start `%s`: the folder %s can't be used", argv[0], dir ? dir : "");
        else
            msg = g_strdup_printf("can't start `%s`: %s", argv[0], e ? e->message : "unknown problem");
        g_clear_error(&e);
        g_strfreev(argv);
        g_free(dir);
        g_free(nx_ui_proc_why);
        nx_ui_proc_why = msg;
        return 0;
    }
    g_strfreev(argv);
    g_free(dir);
    nx_ui_proc *p = g_new0(nx_ui_proc, 1);
    p->id = ++nx_procs.n;
    p->pid = pid;
    p->out_fd = out[0];
    p->in_fd = in[0];
    p->pend = g_string_new(NULL);
    p->in_len = (gsize)input.len;
    p->input = g_memdup2(input.p, input.len ? (gsize)input.len : 1);
    g_unix_set_fd_nonblocking(p->out_fd, TRUE, NULL);
    g_unix_set_fd_nonblocking(p->in_fd, TRUE, NULL);
    g_hash_table_insert(nx_procs.all, &p->id, p);
    nx_ui_group_set(pid, true);
    p->out_watch = g_unix_fd_add(p->out_fd, G_IO_IN | G_IO_HUP | G_IO_ERR, nx_ui_proc_on_out, p);
    p->in_watch = g_unix_fd_add(p->in_fd, G_IO_OUT | G_IO_ERR | G_IO_HUP, nx_ui_proc_on_in, p);
    p->child_watch = g_child_watch_add(pid, nx_ui_proc_on_exit, p);
    return p->id;
#undef NX_UI_PROC_ERR
}

static gboolean nx_ui_proc_on_kill_timer(gpointer d) {
    nx_ui_proc *p = d;
    p->kill_timer = 0;
    if (!p->exited) kill(-p->pid, SIGKILL);
    return G_SOURCE_REMOVE;
}

/* Stop a program this window started (and what it started). Stopping one
   that has already finished does nothing: it may have ended on its own
   just before. */
NX_RT void nx_b___ui_stop_program(int64_t id, int line) {
    nx_ui_need_start(line);
    if (id < 1 || id > nx_procs.n) nx_trap(line, "%" PRId64 " isn't a program this window started", id);
    nx_ui_proc *p = g_hash_table_lookup(nx_procs.all, &id);
    if (!p || p->exited || p->stopping) return;
    p->stopping = true;
    kill(-p->pid, SIGTERM);
    p->kill_timer = g_timeout_add(2000, nx_ui_proc_on_kill_timer, p);
}

#ifdef NX_UI_SOURCE
/* ---- the code editor (0.8): GtkSourceView 4, only in programs that
   `use editor` (so other programs don't need GtkSourceView) ----
   NexaLang's colours (editors/gtksourceview/nexalang.lang) and the Horizon
   fallback scheme are built in (NX_UI_LANG, NX_UI_SCHEME, written by
   nexac) and written to the app's cache folder, $XDG_CACHE_HOME/<id>/nexalang,
   once per start when they differ, so the editor always matches the
   compiler that built it. The theme's own scheme (nexa.xml, written by
   nexa-theme) is used when it's there, and followed live. */
#include <gtksourceview/gtksource.h>

static struct {
    bool ready;
    GtkSourceLanguage *lang;      /* NULL: plain text (the files couldn't be written) */
    GPtrArray *views;             /* every editor */
    GFileMonitor *mon;
} nx_ed;

static void nx_ui_put_file(const char *dir, const char *name, const char *text) {
    char *path = g_build_filename(dir, name, NULL);
    char *old = NULL;
    gsize len = 0;
    if (!(g_file_get_contents(path, &old, &len, NULL) && len == strlen(text) && memcmp(old, text, len) == 0))
        g_file_set_contents(path, text, -1, NULL); /* all-or-nothing; failing leaves plain text */
    g_free(old);
    g_free(path);
}

/* The theme's scheme when there is one, else the fallback. */
static GtkSourceStyleScheme *nx_ui_scheme(void) {
    GtkSourceStyleSchemeManager *sm = gtk_source_style_scheme_manager_get_default();
    GtkSourceStyleScheme *s = gtk_source_style_scheme_manager_get_scheme(sm, "nexa");
    return s ? s : gtk_source_style_scheme_manager_get_scheme(sm, "nexalang-horizon");
}

/* Mistakes are underlined in the theme's @nexa_crit. */
static void nx_ui_editor_restyle(void) {
    if (!nx_ed.ready) return;
    GdkRGBA crit = {1.0, 0.36, 0.48, 1.0};
    GtkStyleContext *sc = gtk_widget_get_style_context(nx_ui.win);
    gtk_style_context_lookup_color(sc, "nexa_crit", &crit);
    GtkSourceStyleScheme *s = nx_ui_scheme();
    for (guint i = 0; i < nx_ed.views->len; i++) {
        GtkSourceBuffer *b = GTK_SOURCE_BUFFER(gtk_text_view_get_buffer(g_ptr_array_index(nx_ed.views, i)));
        if (s) gtk_source_buffer_set_style_scheme(b, s);
        GtkTextTag *t = gtk_text_tag_table_lookup(gtk_text_buffer_get_tag_table(GTK_TEXT_BUFFER(b)), "nx-mistake");
        if (t) g_object_set(t, "underline-rgba", &crit, NULL);
    }
}

static gboolean nx_ui_editor_rescan(gpointer d) {
    (void)d;
    gtk_source_style_scheme_manager_force_rescan(gtk_source_style_scheme_manager_get_default());
    nx_ui_editor_restyle();
    return G_SOURCE_REMOVE;
}

static void nx_ui_on_scheme(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent ev, gpointer d) {
    (void)m; (void)f; (void)o; (void)d;
    if (ev == G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT || ev == G_FILE_MONITOR_EVENT_CREATED || ev == G_FILE_MONITOR_EVENT_DELETED)
        g_timeout_add(200, nx_ui_editor_rescan, NULL);
}

static void nx_ui_editor_setup(void) {
    if (nx_ed.ready) return;
    nx_ed.ready = true;
    nx_ed.views = g_ptr_array_new();
    char *dir = g_build_filename(g_get_user_cache_dir(), g_get_prgname(), "nexalang", NULL);
    bool have = g_mkdir_with_parents(dir, 0700) == 0;
    if (have) {
        nx_ui_put_file(dir, "nexalang.lang", NX_UI_LANG);
        nx_ui_put_file(dir, "nexa-horizon.xml", NX_UI_SCHEME);
        /* ours first: the same version as the compiler, even when an older
           nexalang.lang is installed on the system */
        GtkSourceLanguageManager *lm = gtk_source_language_manager_get_default();
        const gchar *const *old = gtk_source_language_manager_get_search_path(lm);
        GPtrArray *paths = g_ptr_array_new();
        g_ptr_array_add(paths, dir);
        for (int i = 0; old && old[i]; i++) g_ptr_array_add(paths, (gpointer)old[i]);
        g_ptr_array_add(paths, NULL);
        gtk_source_language_manager_set_search_path(lm, (gchar **)paths->pdata);
        g_ptr_array_free(paths, TRUE);
        gtk_source_style_scheme_manager_prepend_search_path(gtk_source_style_scheme_manager_get_default(), dir);
        nx_ed.lang = gtk_source_language_manager_get_language(lm, "nexalang");
    }
    g_free(dir);
    /* follow the theme's editor colours live */
    char *path = g_build_filename(g_get_user_data_dir(), "gtksourceview-4", "styles", "nexa.xml", NULL);
    GFile *file = g_file_new_for_path(path);
    nx_ed.mon = g_file_monitor_file(file, G_FILE_MONITOR_NONE, NULL, NULL);
    if (nx_ed.mon) g_signal_connect(nx_ed.mon, "changed", G_CALLBACK(nx_ui_on_scheme), NULL);
    g_object_unref(file);
    g_free(path);
}

/* The person's edits: one Edited event waiting at a time, at most. */
static void nx_ui_on_edited(GtkTextBuffer *b, gpointer d) {
    (void)b;
    int64_t id = GPOINTER_TO_INT(d);
    for (int64_t i = nx_ui.qhead; i < nx_ui.qlen; i++)
        if (nx_ui.q[i].kind == NX_EV_EDITED && nx_ui.q[i].id == id) return;
    nx_ui_push(NX_EV_EDITED, id, 0, NULL);
}

NX_RT int64_t nx_b___editor_add(int64_t parent, int line) {
    nx_ui_need_start(line);
    nx_ui_editor_setup();
    GtkSourceBuffer *buf = gtk_source_buffer_new(NULL);
    if (nx_ed.lang) gtk_source_buffer_set_language(buf, nx_ed.lang);
    gtk_source_buffer_set_highlight_syntax(buf, TRUE);
    gtk_source_buffer_set_highlight_matching_brackets(buf, TRUE);
    gtk_source_buffer_set_max_undo_levels(buf, 1000);
    GtkSourceStyleScheme *s = nx_ui_scheme();
    if (s) gtk_source_buffer_set_style_scheme(buf, s);
    gtk_text_buffer_create_tag(GTK_TEXT_BUFFER(buf), "nx-mistake", "underline", PANGO_UNDERLINE_ERROR, NULL);
    GtkWidget *v = gtk_source_view_new_with_buffer(buf);
    g_object_unref(buf); /* the view holds it */
    GtkSourceView *sv = GTK_SOURCE_VIEW(v);
    gtk_source_view_set_show_line_numbers(sv, TRUE);
    gtk_source_view_set_highlight_current_line(sv, TRUE);
    gtk_source_view_set_auto_indent(sv, TRUE);
    gtk_source_view_set_indent_on_tab(sv, TRUE);
    gtk_source_view_set_tab_width(sv, 4);
    gtk_source_view_set_indent_width(sv, 4);
    gtk_source_view_set_insert_spaces_instead_of_tabs(sv, TRUE); /* nexac fmt's 4 spaces */
    gtk_source_view_set_smart_backspace(sv, TRUE);
    gtk_source_view_set_smart_home_end(sv, GTK_SOURCE_SMART_HOME_END_BEFORE);
    gtk_text_view_set_monospace(GTK_TEXT_VIEW(v), TRUE);
    gtk_text_view_set_wrap_mode(GTK_TEXT_VIEW(v), GTK_WRAP_NONE);
    gtk_text_view_set_left_margin(GTK_TEXT_VIEW(v), 8);
    gtk_text_view_set_top_margin(GTK_TEXT_VIEW(v), 8);
    gtk_text_view_set_bottom_margin(GTK_TEXT_VIEW(v), 8);
    GtkWidget *sc = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(sc), GTK_POLICY_AUTOMATIC, GTK_POLICY_AUTOMATIC);
    gtk_scrolled_window_set_min_content_height(GTK_SCROLLED_WINDOW(sc), 160);
    gtk_container_add(GTK_CONTAINER(sc), v);
    /* a card around it, with room inside the rounded corners (GTK 3
       doesn't clip a child to them) */
    GtkWidget *card = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    gtk_style_context_add_class(gtk_widget_get_style_context(card), "nx-editor");
    gtk_box_pack_start(GTK_BOX(card), sc, TRUE, TRUE, 0);
    int64_t id = nx_ui_add(parent, v, card, NX_UI_EDITOR, 0, line);
    nx_ui_get(id, line)->sig = g_signal_connect(buf, "changed", G_CALLBACK(nx_ui_on_edited), GINT_TO_POINTER((int)id));
    g_ptr_array_add(nx_ed.views, v);
    g_signal_connect_swapped(v, "destroy", G_CALLBACK(g_ptr_array_remove), nx_ed.views);
    nx_ui_editor_restyle();
    return id;
}

static GtkTextBuffer *nx_ui_editor_buf(nx_ui_item *it) {
    return gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
}

/* ui.set_text on an editor: new text (a file just opened), which can't be
   undone back to the old one; the cursor at the start. */
static void nx_ui_editor_load(nx_ui_item *it, nx_str text) {
    GtkTextBuffer *b = nx_ui_editor_buf(it);
    char *c = nx_ui_cstr(text);
    g_signal_handler_block(b, it->sig);
    gtk_source_buffer_begin_not_undoable_action(GTK_SOURCE_BUFFER(b));
    gtk_text_buffer_set_text(b, c, -1);
    gtk_source_buffer_end_not_undoable_action(GTK_SOURCE_BUFFER(b));
    g_signal_handler_unblock(b, it->sig);
    g_free(c);
    GtkTextIter s;
    gtk_text_buffer_get_start_iter(b, &s);
    gtk_text_buffer_place_cursor(b, &s);
    gtk_text_view_scroll_to_iter(GTK_TEXT_VIEW(it->w), &s, 0, FALSE, 0, 0);
}

/* An iterator at `line`, `column` (from 1; columns in characters, as
   nexac check --json gives them). Positions past the end are moved to
   the nearest real one: the text may have changed since the check. */
static GtkTextIter nx_ui_editor_at(GtkTextBuffer *b, int64_t line, int64_t column) {
    GtkTextIter it;
    int lines = gtk_text_buffer_get_line_count(b);
    if (line < 1) line = 1;
    if (line > lines) {
        gtk_text_buffer_get_end_iter(b, &it);
        return it;
    }
    gtk_text_buffer_get_iter_at_line(b, &it, (gint)(line - 1));
    GtkTextIter end = it;
    if (!gtk_text_iter_ends_line(&end)) gtk_text_iter_forward_to_line_end(&end);
    int max = gtk_text_iter_get_line_offset(&end); /* without the line break */
    if (column < 1) column = 1;
    if (column - 1 > max) column = max + 1;
    gtk_text_iter_set_line_offset(&it, (gint)(column - 1));
    return it;
}

NX_RT void nx_b___editor_replace(int64_t id, nx_str text, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_EDITOR, "be edited (only code editors can)", line);
    GtkTextBuffer *b = nx_ui_editor_buf(it);
    GtkTextIter cur;
    gtk_text_buffer_get_iter_at_mark(b, &cur, gtk_text_buffer_get_insert(b));
    int64_t at_line = gtk_text_iter_get_line(&cur) + 1, at_col = gtk_text_iter_get_line_offset(&cur) + 1;
    char *c = nx_ui_cstr(text);
    GtkTextIter s, e;
    gtk_text_buffer_get_bounds(b, &s, &e);
    char *old = gtk_text_buffer_get_text(b, &s, &e, TRUE);
    if (strcmp(old, c) != 0) {
        g_signal_handler_block(b, it->sig);
        gtk_text_buffer_begin_user_action(b); /* one step for undo */
        gtk_text_buffer_delete(b, &s, &e);
        gtk_text_buffer_get_start_iter(b, &s);
        gtk_text_buffer_insert(b, &s, c, -1);
        gtk_text_buffer_end_user_action(b);
        g_signal_handler_unblock(b, it->sig);
        GtkTextIter to = nx_ui_editor_at(b, at_line, at_col);
        gtk_text_buffer_place_cursor(b, &to);
        gtk_text_view_scroll_to_mark(GTK_TEXT_VIEW(it->w), gtk_text_buffer_get_insert(b), 0.1, FALSE, 0, 0);
    }
    g_free(old);
    g_free(c);
}

NX_RT void nx_b___editor_mark(int64_t id, int64_t ln, int64_t col, int64_t length, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_EDITOR, "mark mistakes (only code editors can)", line);
    GtkTextBuffer *b = nx_ui_editor_buf(it);
    if (ln < 1 || ln > gtk_text_buffer_get_line_count(b)) return; /* not in this text (any more) */
    GtkTextIter s = nx_ui_editor_at(b, ln, col), e = s;
    if (length < 1) length = 1;
    if (!gtk_text_iter_ends_line(&e)) {
        GtkTextIter eol = e;
        gtk_text_iter_forward_to_line_end(&eol);
        int room = gtk_text_iter_get_line_offset(&eol) - gtk_text_iter_get_line_offset(&e);
        gtk_text_iter_forward_chars(&e, (gint)(length < room ? length : room));
    } else if (gtk_text_iter_get_line_offset(&s) > 0) {
        gtk_text_iter_backward_char(&s); /* at the end of a line: the last letter */
    } else {
        return; /* an empty line: nothing to underline (the list still says where) */
    }
    gtk_text_buffer_apply_tag_by_name(b, "nx-mistake", &s, &e);
}

NX_RT void nx_b___editor_clear_marks(int64_t id, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_EDITOR, "mark mistakes (only code editors can)", line);
    GtkTextBuffer *b = nx_ui_editor_buf(it);
    GtkTextIter s, e;
    gtk_text_buffer_get_bounds(b, &s, &e);
    gtk_text_buffer_remove_tag_by_name(b, "nx-mistake", &s, &e);
}

NX_RT void nx_b___editor_go_to(int64_t id, int64_t ln, int64_t col, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_EDITOR, "go to a line (only code editors can)", line);
    GtkTextBuffer *b = nx_ui_editor_buf(it);
    GtkTextIter to = nx_ui_editor_at(b, ln, col);
    gtk_text_buffer_place_cursor(b, &to);
    gtk_widget_grab_focus(it->w);
    gtk_text_view_scroll_to_mark(GTK_TEXT_VIEW(it->w), gtk_text_buffer_get_insert(b), 0.1, TRUE, 0.0, 0.3);
}

/* For the test tree: how many places are underlined. */
static int nx_ui_editor_marks(GtkTextBuffer *b) {
    GtkTextTag *t = gtk_text_tag_table_lookup(gtk_text_buffer_get_tag_table(b), "nx-mistake");
    GtkTextIter i;
    gtk_text_buffer_get_start_iter(b, &i);
    int n = gtk_text_iter_starts_tag(&i, t) ? 1 : 0;
    while (gtk_text_iter_forward_to_tag_toggle(&i, t))
        if (gtk_text_iter_starts_tag(&i, t)) n++;
    return n;
}
static const char *nx_ui_editor_scheme_id(GtkTextBuffer *b) {
    GtkSourceStyleScheme *s = gtk_source_buffer_get_style_scheme(GTK_SOURCE_BUFFER(b));
    return s ? gtk_source_style_scheme_get_id(s) : "none";
}
#else
__attribute__((unused)) static const char *nx_ui_editor_scheme_id(GtkTextBuffer *b) { (void)b; return "none"; }
__attribute__((unused)) static void nx_ui_editor_load(nx_ui_item *it, nx_str text) { (void)it; (void)text; }
__attribute__((unused)) static void nx_ui_editor_restyle(void) { }
__attribute__((unused)) static int nx_ui_editor_marks(GtkTextBuffer *b) { (void)b; return 0; }
#endif

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
    case NX_UI_EDITOR: {
        GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
        GtkTextIter c;
        gtk_text_buffer_get_iter_at_mark(b, &c, gtk_text_buffer_get_insert(b));
        printf("editor (%d lines, %d marked, at %d:%d, colours %s)", gtk_text_buffer_get_line_count(b), nx_ui_editor_marks(b),
               gtk_text_iter_get_line(&c) + 1, gtk_text_iter_get_line_offset(&c) + 1, nx_ui_editor_scheme_id(b));
        break;
    }
    default: printf("code (%d lines)", gtk_text_buffer_get_line_count(gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w)))); break;
    }
    if (nx_ui.tree_full && (it->kind == NX_UI_CODE || it->kind == NX_UI_EDITOR)) {
        /* NEXALANG_UI_TREE=full (0.8): their text, line by line */
        GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
        GtkTextIter s, e;
        gtk_text_buffer_get_bounds(b, &s, &e);
        char *all = gtk_text_buffer_get_text(b, &s, &e, TRUE);
        char **lines = g_strsplit(all, "\n", -1);
        for (int i = 0; lines[i]; i++) {
            if (!lines[i + 1] && !*lines[i]) break; /* after the last line break */
            printf("\n%*s| ", depth * 2 + 2, "");
            nx_ui_tree_text(lines[i]);
        }
        g_strfreev(lines);
        g_free(all);
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
