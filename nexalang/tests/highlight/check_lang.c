/* Test-only: show how GtkSourceView 4 colours a NexaLang file with
 * editors/gtksourceview/nexalang.lang. Every def:* style gets its own
 * colour in letters.xml; each character is printed as the letter of the
 * colour it gets, under its line:
 *
 *   let x = "a{y}"
 *   kkk.....sseess
 *
 * usage: check_lang LANG_DIR SCHEME_DIR FILE.nx
 * (no display needed: only a buffer is made, never a window) */
#include <gtksourceview/gtksource.h>
#include <stdio.h>
#include <string.h>

static const struct { const char *rgb; char letter; } COLOURS[] = {
    {"#010000", 'c'}, /* def:comment */
    {"#020000", 's'}, /* def:string */
    {"#030000", 'e'}, /* def:special-char */
    {"#040000", 'x'}, /* def:error */
    {"#050000", 'k'}, /* def:keyword */
    {"#060000", 'o'}, /* def:operator */
    {"#070000", 't'}, /* def:type */
    {"#080000", 'b'}, /* def:builtin */
    {"#090000", 'm'}, /* def:preprocessor */
    {"#0a0000", 'n'}, /* def:special-constant */
    {"#0b0000", 'B'}, /* def:boolean */
    {"#0c0000", 'd'}, /* def:decimal */
    {"#0d0000", 'h'}, /* def:base-n-integer */
    {"#0e0000", 'f'}, /* def:floating-point */
    {"#0f0000", 'F'}, /* def:function */
};

static char letter_at(GtkTextIter *it) {
    GSList *tags = gtk_text_iter_get_tags(it);
    char best = '.';
    int best_prio = -1;
    for (GSList *l = tags; l; l = l->next) {
        GtkTextTag *tag = l->data;
        gboolean set = FALSE;
        GdkRGBA *rgba = NULL;
        g_object_get(tag, "foreground-set", &set, "foreground-rgba", &rgba, NULL);
        if (set && rgba) {
            char hex[8];
            snprintf(hex, sizeof hex, "#%02x%02x%02x", (int)(rgba->red * 255 + 0.5), (int)(rgba->green * 255 + 0.5),
                     (int)(rgba->blue * 255 + 0.5));
            int prio = gtk_text_tag_get_priority(tag);
            for (size_t i = 0; i < sizeof COLOURS / sizeof COLOURS[0]; i++) {
                if (strcmp(hex, COLOURS[i].rgb) == 0 && prio > best_prio) {
                    best = COLOURS[i].letter;
                    best_prio = prio;
                }
            }
        }
        if (rgba) gdk_rgba_free(rgba);
    }
    g_slist_free(tags);
    return best;
}

int main(int argc, char **argv) {
    if (argc != 4) {
        fprintf(stderr, "usage: check_lang LANG_DIR SCHEME_DIR FILE.nx\n");
        return 2;
    }
    /* registers GTK's types; a display isn't needed, so failing to open
     * one is fine */
    (void)gtk_init_check(NULL, NULL);
    gchar *text = NULL;
    gsize len = 0;
    if (!g_file_get_contents(argv[3], &text, &len, NULL)) {
        fprintf(stderr, "can't read %s\n", argv[3]);
        return 1;
    }
    /* our folder first, then the system's (it has def.lang and the
     * schema every language file is checked against) */
    GtkSourceLanguageManager *lm = gtk_source_language_manager_new();
    const gchar *const *sys = gtk_source_language_manager_get_search_path(gtk_source_language_manager_get_default());
    GPtrArray *dirs = g_ptr_array_new();
    g_ptr_array_add(dirs, argv[1]);
    for (size_t i = 0; sys && sys[i]; i++) g_ptr_array_add(dirs, (gpointer)sys[i]);
    g_ptr_array_add(dirs, NULL);
    gtk_source_language_manager_set_search_path(lm, (gchar **)dirs->pdata);
    g_ptr_array_free(dirs, TRUE);
    GtkSourceLanguage *lang = gtk_source_language_manager_get_language(lm, "nexalang");
    if (!lang) {
        fprintf(stderr, "no language `nexalang` in %s\n", argv[1]);
        return 1;
    }
    /* the file is found by its name, as the text editor does it
     * (text-editor/nexa_edit/window.py: _guess_language) */
    gchar *ctype = g_content_type_guess("app.nx", NULL, 0, NULL);
    GtkSourceLanguage *guessed = gtk_source_language_manager_guess_language(lm, "app.nx", ctype);
    g_free(ctype);
    if (guessed != lang) {
        fprintf(stderr, "app.nx isn't recognised as NexaLang\n");
        return 1;
    }
    GtkSourceStyleSchemeManager *sm = gtk_source_style_scheme_manager_new();
    const gchar *scheme_dirs[] = {argv[2], NULL};
    gtk_source_style_scheme_manager_set_search_path(sm, (gchar **)scheme_dirs);
    GtkSourceStyleScheme *scheme = gtk_source_style_scheme_manager_get_scheme(sm, "nexalang-letters");
    if (!scheme) {
        fprintf(stderr, "no scheme in %s\n", argv[2]);
        return 1;
    }
    GtkSourceBuffer *buf = gtk_source_buffer_new_with_language(lang);
    gtk_source_buffer_set_style_scheme(buf, scheme);
    gtk_text_buffer_set_text(GTK_TEXT_BUFFER(buf), text, (gint)len);
    GtkTextIter start, end;
    gtk_text_buffer_get_bounds(GTK_TEXT_BUFFER(buf), &start, &end);
    gtk_source_buffer_ensure_highlight(buf, &start, &end);

    GString *line = g_string_new(NULL), *marks = g_string_new(NULL);
    GtkTextIter it = start;
    while (!gtk_text_iter_is_end(&it)) {
        gunichar c = gtk_text_iter_get_char(&it);
        if (c == '\n') {
            printf("%s\n%s\n", line->str, marks->str);
            g_string_truncate(line, 0);
            g_string_truncate(marks, 0);
        } else {
            g_string_append_unichar(line, c);
            g_string_append_c(marks, letter_at(&it));
        }
        gtk_text_iter_forward_char(&it);
    }
    if (line->len) printf("%s\n%s\n", line->str, marks->str);
    g_string_free(line, TRUE);
    g_string_free(marks, TRUE);
    g_object_unref(buf);
    g_object_unref(sm);
    g_object_unref(lm);
    g_free(text);
    return 0;
}
