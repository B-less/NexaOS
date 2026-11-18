/* NexaLang runtime: the code editor (GtkSourceView 4), added only to
 * programs that `use editor` (0.8), after runtime_ui.h.
 *
 * An editor is a widget like the others (numbered, checked on every use,
 * removed with ui.remove). Its text goes in and out through ui.set_text
 * and ui.get_text, copied both ways like every widget's text; the person's
 * changes come back as one `Changed` event per ui.wait() (runtime_ui.h).
 *
 * Colours: Hexa says apps use the theme's @nexa_* colours, never their
 * own. GtkSourceView takes its colours from a "style scheme" file, which
 * can't name CSS colours, so the scheme is written from the @nexa_* colours
 * the window has right now (the theme's, or the fallback block), loaded,
 * and its file deleted again; it's written again when the theme changes.
 * Nothing is kept on the disk. */
#include <gtksourceview/gtksource.h>
#include <glib/gstdio.h>

static GtkSourceStyleSchemeManager *nx_ed_schemes; /* holds the scheme in use */
static char *nx_ed_scheme_xml;                     /* to write it only when colours change */

static void nx_ed_color(GtkStyleContext *ctx, const char *name, char out[8]) {
    GdkRGBA c = {0.5, 0.5, 0.5, 1};
    gtk_style_context_lookup_color(ctx, name, &c); /* the fallback block defines every name */
    snprintf(out, 8, "#%02x%02x%02x", (int)(c.red * 255 + 0.5) & 255, (int)(c.green * 255 + 0.5) & 255, (int)(c.blue * 255 + 0.5) & 255);
}

/* Write, load and delete the scheme; give it to every editor. */
static void nx_ed_refresh(GtkWidget *any) {
    GtkStyleContext *ctx = gtk_widget_get_style_context(any);
    enum { TEXT, STRONG, MUTED, FIELD, BG, DEEP, SURFACE, HI, ACCENT, ACCENT_HI, SEL, SEL_TEXT, CRIT, OK, WARN, N };
    static const char *const names[N] = {"nexa_text", "nexa_text_strong", "nexa_muted", "nexa_field", "nexa_bg", "nexa_bg_deep", "nexa_surface",
                                         "nexa_surface_hi", "nexa_accent", "nexa_accent_hover", "nexa_selected", "nexa_selected_text",
                                         "nexa_crit", "nexa_ok", "nexa_warn"};
    char c[N][8];
    for (int i = 0; i < N; i++) nx_ed_color(ctx, names[i], c[i]);
    char *xml = g_strdup_printf(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
        "<style-scheme id=\"nexa-ui\" name=\"NexaOS (from the theme)\" version=\"1.0\">\n"
        "  <style name=\"text\" foreground=\"%s\" background=\"%s\"/>\n"
        "  <style name=\"selection\" foreground=\"%s\" background=\"%s\"/>\n"
        "  <style name=\"cursor\" foreground=\"%s\"/>\n"
        "  <style name=\"current-line\" background=\"%s\"/>\n"
        "  <style name=\"line-numbers\" foreground=\"%s\" background=\"%s\"/>\n"
        "  <style name=\"current-line-number\" foreground=\"%s\" background=\"%s\" bold=\"true\"/>\n"
        "  <style name=\"bracket-match\" foreground=\"%s\" background=\"%s\" bold=\"true\"/>\n"
        "  <style name=\"bracket-mismatch\" foreground=\"%s\" underline=\"error\"/>\n"
        "  <style name=\"def:comment\" foreground=\"%s\" italic=\"true\"/>\n"
        "  <style name=\"def:keyword\" foreground=\"%s\" bold=\"true\"/>\n"
        "  <style name=\"def:operator\" foreground=\"%s\" bold=\"true\"/>\n"
        "  <style name=\"def:type\" foreground=\"%s\"/>\n"
        "  <style name=\"def:preprocessor\" foreground=\"%s\"/>\n"
        "  <style name=\"def:function\" foreground=\"%s\"/>\n"
        "  <style name=\"def:builtin\" foreground=\"%s\"/>\n"
        "  <style name=\"def:string\" foreground=\"%s\"/>\n"
        "  <style name=\"def:special-char\" foreground=\"%s\"/>\n"
        "  <style name=\"def:number\" foreground=\"%s\"/>\n"
        "  <style name=\"def:boolean\" foreground=\"%s\"/>\n"
        "  <style name=\"def:special-constant\" foreground=\"%s\"/>\n"
        "  <style name=\"def:constant\" foreground=\"%s\"/>\n"
        "  <style name=\"def:identifier\" foreground=\"%s\"/>\n"
        "  <style name=\"def:error\" foreground=\"%s\" underline=\"error\"/>\n"
        "</style-scheme>\n",
        c[STRONG], c[FIELD], c[SEL_TEXT], c[SEL], c[ACCENT], c[SURFACE], c[MUTED], c[DEEP], c[ACCENT], c[SURFACE],
        c[ACCENT], c[HI], c[CRIT], c[MUTED], c[ACCENT], c[ACCENT], c[ACCENT_HI], c[ACCENT_HI], c[TEXT], c[ACCENT_HI],
        c[OK], c[WARN], c[WARN], c[WARN], c[WARN], c[WARN], c[TEXT], c[CRIT]);
    GdkRGBA crit = {1, 0.36, 0.48, 1};
    gtk_style_context_lookup_color(ctx, "nexa_crit", &crit);
    GdkRGBA tint = crit;
    tint.alpha = 0.18;
    if (nx_ed_scheme_xml && strcmp(xml, nx_ed_scheme_xml) == 0) {
        g_free(xml);
        return;
    }
    GtkSourceStyleScheme *scheme = NULL;
    GtkSourceStyleSchemeManager *m = NULL;
    char *dir = g_dir_make_tmp("nexa-editor-XXXXXX", NULL);
    if (dir) {
        char *file = g_build_filename(dir, "nexa-ui.xml", NULL);
        if (g_file_set_contents(file, xml, -1, NULL)) {
            m = gtk_source_style_scheme_manager_new();
            const gchar *const path[] = {dir, NULL};
            gtk_source_style_scheme_manager_set_search_path(m, (gchar **)path);
            scheme = gtk_source_style_scheme_manager_get_scheme(m, "nexa-ui"); /* read now */
        }
        g_remove(file);
        g_rmdir(dir);
        g_free(file);
        g_free(dir);
    }
    if (!scheme) { /* couldn't write it (no /tmp?): keep what the editors have */
        if (m) g_object_unref(m);
        g_free(xml);
        return;
    }
    GHashTableIter i;
    gpointer k, v;
    g_hash_table_iter_init(&i, nx_ui.items);
    while (g_hash_table_iter_next(&i, &k, &v)) {
        nx_ui_item *it = v;
        if (it->kind != NX_UI_EDITOR) continue;
        GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
        gtk_source_buffer_set_style_scheme(GTK_SOURCE_BUFFER(b), scheme);
        GtkTextTag *t = gtk_text_tag_table_lookup(gtk_text_buffer_get_tag_table(b), "nx-mistake");
        if (t) g_object_set(t, "underline-rgba", &crit, "background-rgba", &tint, NULL);
    }
    if (nx_ed_schemes) g_object_unref(nx_ed_schemes);
    nx_ed_schemes = m;
    g_free(nx_ed_scheme_xml);
    nx_ed_scheme_xml = xml;
}

/* The theme changed (or the widget's style did): new colours if they changed. */
static void nx_ed_on_style(GtkWidget *w, gpointer d) {
    (void)d;
    nx_ed_refresh(w);
}

/* No ligatures (as in ui.code): what's shown is what's typed, so `->`
   stays two letters. Everything typed or set gets the tag. */
static void nx_ed_on_insert(GtkTextBuffer *b, GtkTextIter *at, gchar *text, gint len, gpointer d) {
    (void)d;
    GtkTextIter from = *at;
    gtk_text_iter_backward_chars(&from, (gint)g_utf8_strlen(text, len));
    gtk_text_buffer_apply_tag_by_name(b, "nx-plain", &from, at);
}

static void nx_ed_set(GtkWidget *view, const char *text) {
    GtkSourceBuffer *b = GTK_SOURCE_BUFFER(gtk_text_view_get_buffer(GTK_TEXT_VIEW(view)));
    gtk_source_buffer_begin_not_undoable_action(b); /* a new start: undo can't go back past it */
    gtk_text_buffer_set_text(GTK_TEXT_BUFFER(b), text, -1);
    gtk_source_buffer_end_not_undoable_action(b);
    GtkTextIter s;
    gtk_text_buffer_get_start_iter(GTK_TEXT_BUFFER(b), &s);
    gtk_text_buffer_place_cursor(GTK_TEXT_BUFFER(b), &s);
}

/* For tests (NEXALANG_UI_TREE): the language, the cursor, the marks and,
   when it's short, the text. */
static void nx_ed_tree(GtkWidget *view, int depth) {
    GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(view));
    const char *lang = g_object_get_data(G_OBJECT(b), "nx-lang");
    GtkTextIter c, s, e;
    gtk_text_buffer_get_iter_at_mark(b, &c, gtk_text_buffer_get_insert(b));
    printf("editor ");
    nx_ui_tree_text(lang ? lang : "");
    if (lang && *lang && !gtk_source_buffer_get_language(GTK_SOURCE_BUFFER(b))) fputs(" (not found: plain text)", stdout);
    printf(" (%d lines, cursor %d:%d)", gtk_text_buffer_get_line_count(b), gtk_text_iter_get_line(&c) + 1, gtk_text_iter_get_line_offset(&c) + 1);
    GtkTextTag *t = gtk_text_tag_table_lookup(gtk_text_buffer_get_tag_table(b), "nx-mistake");
    gtk_text_buffer_get_start_iter(b, &s);
    while (t) {
        if (!gtk_text_iter_has_tag(&s, t) && !gtk_text_iter_forward_to_tag_toggle(&s, t)) break;
        e = s;
        gtk_text_iter_forward_to_tag_toggle(&e, t);
        printf(" mark %d:%d-%d:%d", gtk_text_iter_get_line(&s) + 1, gtk_text_iter_get_line_offset(&s) + 1,
               gtk_text_iter_get_line(&e) + 1, gtk_text_iter_get_line_offset(&e) + 1);
        s = e;
    }
    if (gtk_text_buffer_get_char_count(b) <= 2000) {
        gtk_text_buffer_get_bounds(b, &s, &e);
        char *all = gtk_text_buffer_get_text(b, &s, &e, TRUE);
        char **lines = g_strsplit(all, "\n", -1);
        for (char **l = lines; *l; l++) printf("\n%*s| %s", depth * 2 + 2, "", *l);
        g_strfreev(lines);
        g_free(all);
    } else {
        printf(" (%d letters)", gtk_text_buffer_get_char_count(b));
    }
}

/* An iterator at (line, column), from 1; the nearest place in the text. */
static void nx_ed_place(GtkTextBuffer *b, int64_t line, int64_t col, GtkTextIter *at, int64_t *line_len) {
    int lines = gtk_text_buffer_get_line_count(b);
    if (line < 1) line = 1;
    if (line > lines) line = lines;
    gtk_text_buffer_get_iter_at_line(b, at, (gint)(line - 1));
    GtkTextIter end = *at;
    if (!gtk_text_iter_ends_line(&end)) gtk_text_iter_forward_to_line_end(&end);
    int64_t len = gtk_text_iter_get_line_offset(&end);
    if (col < 1) col = 1;
    if (col > len + 1) col = len + 1;
    gtk_text_iter_set_line_offset(at, (gint)(col - 1));
    if (line_len) *line_len = len;
}

NX_RT int64_t nx_b___editor_new(int64_t parent, nx_str language, int line) {
    nx_ui_need_start(line);
    nx_ui_editor_set = nx_ed_set;
    nx_ui_editor_tree = nx_ed_tree;
    GtkWidget *sc = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(sc), GTK_POLICY_AUTOMATIC, GTK_POLICY_AUTOMATIC);
    gtk_scrolled_window_set_min_content_height(GTK_SCROLLED_WINDOW(sc), 160);
    gtk_style_context_add_class(gtk_widget_get_style_context(sc), "nx-code");
    gtk_style_context_add_class(gtk_widget_get_style_context(sc), "nx-editor");
    GtkSourceBuffer *buf = gtk_source_buffer_new(NULL);
    char *lang = nx_ui_cstr(language);
    if (*lang) {
        GtkSourceLanguage *l = gtk_source_language_manager_get_language(gtk_source_language_manager_get_default(), lang);
        if (l) gtk_source_buffer_set_language(buf, l);
    }
    g_object_set_data_full(G_OBJECT(buf), "nx-lang", lang, g_free);
    gtk_source_buffer_set_highlight_matching_brackets(buf, TRUE);
    gtk_text_buffer_create_tag(GTK_TEXT_BUFFER(buf), "nx-mistake", "underline", PANGO_UNDERLINE_ERROR, NULL);
    gtk_text_buffer_create_tag(GTK_TEXT_BUFFER(buf), "nx-plain", "font-features", "-calt, -liga", NULL);
    g_signal_connect_after(buf, "insert-text", G_CALLBACK(nx_ed_on_insert), NULL);
    GtkWidget *v = gtk_source_view_new_with_buffer(buf);
    g_object_unref(buf);
    GtkSourceView *sv = GTK_SOURCE_VIEW(v);
    gtk_source_view_set_show_line_numbers(sv, TRUE);
    gtk_source_view_set_highlight_current_line(sv, TRUE);
    gtk_source_view_set_auto_indent(sv, TRUE);
    gtk_source_view_set_indent_on_tab(sv, TRUE);
    gtk_source_view_set_tab_width(sv, 4);
    gtk_source_view_set_indent_width(sv, 4);
    gtk_source_view_set_insert_spaces_instead_of_tabs(sv, TRUE);
    gtk_source_view_set_smart_backspace(sv, TRUE);
    gtk_text_view_set_monospace(GTK_TEXT_VIEW(v), TRUE);
    gtk_text_view_set_left_margin(GTK_TEXT_VIEW(v), 8);
    gtk_text_view_set_top_margin(GTK_TEXT_VIEW(v), 8);
    gtk_text_view_set_bottom_margin(GTK_TEXT_VIEW(v), 8);
    gtk_container_add(GTK_CONTAINER(sc), v);
    int64_t id = nx_ui_add(parent, v, sc, NX_UI_EDITOR, 0, line);
    nx_ui_get(id, line)->sig = g_signal_connect(buf, "changed", G_CALLBACK(nx_ui_on_buffer_changed), GINT_TO_POINTER((int)id));
    g_signal_connect(v, "style-updated", G_CALLBACK(nx_ed_on_style), NULL);
    g_free(nx_ed_scheme_xml); /* a new editor gets the scheme too */
    nx_ed_scheme_xml = NULL;
    nx_ed_refresh(v);
    return id;
}

/* line 0: remove every mark */
NX_RT void nx_b___editor_mark(int64_t id, int64_t ln, int64_t col, int64_t len, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_EDITOR, "mark mistakes (only editors can)", line);
    GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
    GtkTextIter s, e;
    if (ln == 0) {
        gtk_text_buffer_get_bounds(b, &s, &e);
        gtk_text_buffer_remove_tag_by_name(b, "nx-mistake", &s, &e);
        return;
    }
    int64_t line_len = 0;
    nx_ed_place(b, ln, col, &s, &line_len);
    int64_t from = gtk_text_iter_get_line_offset(&s);
    int64_t to = from + (len < 1 ? 1 : len > line_len ? line_len : len);
    if (to > line_len) to = line_len;
    if (from >= to) { /* at the end of the line: mark its last letter */
        if (line_len == 0) return;
        from = line_len - 1;
        to = line_len;
    }
    gtk_text_iter_set_line_offset(&s, (gint)from);
    e = s;
    gtk_text_iter_set_line_offset(&e, (gint)to);
    gtk_text_buffer_apply_tag_by_name(b, "nx-mistake", &s, &e);
}

NX_RT void nx_b___editor_go_to(int64_t id, int64_t ln, int64_t col, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_EDITOR, "have a cursor (only editors can)", line);
    GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
    GtkTextIter at;
    nx_ed_place(b, ln, col, &at, NULL);
    gtk_text_buffer_place_cursor(b, &at);
    gtk_text_view_scroll_to_mark(GTK_TEXT_VIEW(it->w), gtk_text_buffer_get_insert(b), 0.1, FALSE, 0, 0);
    gtk_widget_grab_focus(it->w);
}

NX_RT int64_t nx_b___editor_cursor(int64_t id, bool column, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_EDITOR, "have a cursor (only editors can)", line);
    GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
    GtkTextIter c;
    gtk_text_buffer_get_iter_at_mark(b, &c, gtk_text_buffer_get_insert(b));
    return (column ? gtk_text_iter_get_line_offset(&c) : gtk_text_iter_get_line(&c)) + 1;
}

/* The whole text replaced as one step the person can undo (formatting). */
NX_RT void nx_b___editor_replace(int64_t id, nx_str text, int line) {
    nx_ui_item *it = nx_ui_want(id, 1u << NX_UI_EDITOR, "have its text replaced (only editors can)", line);
    GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(it->w));
    char *c = nx_ui_cstr(text);
    GtkTextIter s, e;
    g_signal_handler_block(b, it->sig);
    gtk_text_buffer_begin_user_action(b);
    gtk_text_buffer_get_bounds(b, &s, &e);
    gtk_text_buffer_delete(b, &s, &e);
    gtk_text_buffer_insert(b, &s, c, -1);
    gtk_text_buffer_end_user_action(b);
    g_signal_handler_unblock(b, it->sig);
    g_free(c);
}
