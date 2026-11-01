/*
 * nexa-pdfdoc: the PDF Viewer's engine. The slow or risky work on a PDF
 * runs here, in its own confined process, so a big document never freezes
 * the window and a hostile one can't take it down:
 *
 *   nexa-pdfdoc info FILE                  details and contents (outline)
 *   nexa-pdfdoc search FILE [--case] [--words] -- TEXT
 *                                          every match, page by page
 *   nexa-pdfdoc thumbs FILE OUTDIR WIDTH PAGE...
 *                                          page pictures, WIDTH pixels wide
 *   nexa-pdfdoc text FILE PAGE             the text of one page
 *
 * A password, when the PDF has one, comes on standard input (one line;
 * never on the command line, where other programs could see it).
 *
 * Output: JSON lines. Progress lines ({"found": ...}, {"thumb": ...})
 * while it works, then {"ok": true, "result": ...} or {"ok": false,
 * "error": "...", "code": "password" | "damaged" | ...}. Pages are counted
 * from 1; rectangles are [x1, y1, x2, y2] in PDF points from the page's top
 * left corner.
 *
 * Before opening the document the process confines itself with Landlock
 * (read: the system and the PDF; write: OUTDIR for thumbs) and caps its
 * memory and CPU time.
 */
#define _GNU_SOURCE
#include <cairo.h>
#include <errno.h>
#include <fcntl.h>
#include <glib.h>
#include <glib/gstdio.h>
#include <poppler.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

/* ---------- Landlock (as in nexa-thumb) ---------- */

#define LL_EXECUTE (1ULL << 0)
#define LL_WRITE_FILE (1ULL << 1)
#define LL_READ_FILE (1ULL << 2)
#define LL_READ_DIR (1ULL << 3)
#define LL_REMOVE_DIR (1ULL << 4)
#define LL_REMOVE_FILE (1ULL << 5)
#define LL_MAKE_CHAR (1ULL << 6)
#define LL_MAKE_DIR (1ULL << 7)
#define LL_MAKE_REG (1ULL << 8)
#define LL_MAKE_SOCK (1ULL << 9)
#define LL_MAKE_FIFO (1ULL << 10)
#define LL_MAKE_BLOCK (1ULL << 11)
#define LL_MAKE_SYM (1ULL << 12)
#define LL_REFER (1ULL << 13)
#define LL_TRUNCATE (1ULL << 14)
#define LL_ALL_V1 ((1ULL << 13) - 1)
#define LL_READ (LL_READ_FILE | LL_READ_DIR)
#define LL_RW (LL_READ | LL_WRITE_FILE | LL_REMOVE_FILE | LL_MAKE_REG | LL_TRUNCATE)

struct ll_ruleset_attr { uint64_t handled_access_fs; };
struct ll_path_beneath { uint64_t allowed_access; int32_t parent_fd; } __attribute__((packed));

static int ll_add(int ruleset, const char *path, uint64_t access, uint64_t handled)
{
    struct stat st;
    if (stat(path, &st) != 0)
        return 0;
    int fd = open(path, O_PATH | O_CLOEXEC);
    if (fd < 0)
        return -1;
    struct ll_path_beneath rule = { access & handled, fd };
    if (!S_ISDIR(st.st_mode))
        rule.allowed_access &= LL_EXECUTE | LL_WRITE_FILE | LL_READ_FILE | LL_TRUNCATE;
    int rc = (int)syscall(445 /* landlock_add_rule */, ruleset, 1 /* PATH_BENEATH */, &rule, 0);
    close(fd);
    return rc;
}

static int confine(const char *read_path, const char *write_path)
{
    long abi = syscall(444 /* landlock_create_ruleset */, NULL, 0, 1U /* VERSION */);
    if (abi < 1)
        return 0;
    uint64_t handled = LL_ALL_V1 | (abi >= 2 ? LL_REFER : 0) | (abi >= 3 ? LL_TRUNCATE : 0);
    struct ll_ruleset_attr attr = { handled };
    int rs = (int)syscall(444, &attr, sizeof attr, 0);
    if (rs < 0)
        return -1;
    const char *system[] = { "/usr", "/lib", "/etc", "/proc/self" };
    for (size_t i = 0; i < G_N_ELEMENTS(system); i++)
        if (ll_add(rs, system[i], LL_READ | LL_EXECUTE, handled) < 0)
            goto fail;
    if (ll_add(rs, "/dev/null", LL_READ_FILE | LL_WRITE_FILE, handled) < 0 || ll_add(rs, "/dev/urandom", LL_READ_FILE, handled) < 0)
        goto fail;
    if (ll_add(rs, read_path, LL_READ, handled) < 0)
        goto fail;
    if (write_path && ll_add(rs, write_path, LL_RW, handled) < 0)
        goto fail;
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 || syscall(446 /* restrict_self */, rs, 0) != 0)
        goto fail;
    close(rs);
    return (int)abi;
fail:
    close(rs);
    return -1;
}

/* ---------- JSON out ---------- */

static void json_str(GString *out, const char *s)
{
    char *valid = g_utf8_make_valid(s ? s : "", -1);
    g_string_append_c(out, '"');
    for (const unsigned char *p = (const unsigned char *)valid; *p; p++) {
        switch (*p) {
        case '"': g_string_append(out, "\\\""); break;
        case '\\': g_string_append(out, "\\\\"); break;
        case '\n': g_string_append(out, "\\n"); break;
        case '\r': g_string_append(out, "\\r"); break;
        case '\t': g_string_append(out, "\\t"); break;
        default:
            if (*p < 0x20)
                g_string_append_printf(out, "\\u%04x", *p);
            else
                g_string_append_c(out, (char)*p);
        }
    }
    g_string_append_c(out, '"');
    g_free(valid);
}

static void emit(GString *line)
{
    fputs(line->str, stdout);
    fputc('\n', stdout);
    fflush(stdout);
    g_string_free(line, TRUE);
}

static int fail(const char *code, const char *message)
{
    GString *o = g_string_new("{\"ok\": false, \"code\": ");
    json_str(o, code);
    g_string_append(o, ", \"error\": ");
    json_str(o, message);
    g_string_append_c(o, '}');
    emit(o);
    return 1;
}

/* ---------- opening ---------- */

static char *password;

static PopplerDocument *open_doc(const char *path, int *rc)
{
    GError *err = NULL;
    char *uri = g_filename_to_uri(path, NULL, NULL);
    PopplerDocument *doc = uri ? poppler_document_new_from_file(uri, password, &err) : NULL;
    g_free(uri);
    if (doc)
        return doc;
    if (err && err->domain == POPPLER_ERROR && err->code == POPPLER_ERROR_ENCRYPTED)
        *rc = fail("password", password && *password ? "That password isn't right." : "This PDF is locked with a password.");
    else if (err && err->domain == G_FILE_ERROR && err->code == G_FILE_ERROR_NOENT)
        *rc = fail("unreadable", "This PDF isn't there any more.");
    else if (err && err->domain == G_FILE_ERROR && (err->code == G_FILE_ERROR_ACCES || err->code == G_FILE_ERROR_PERM))
        *rc = fail("unreadable", "You don't have permission to open this PDF.");
    else if (err && err->domain == G_FILE_ERROR)
        *rc = fail("unreadable", err->message);
    else
        *rc = fail("damaged", "This PDF is damaged, or it isn't a PDF.");
    if (err)
        g_error_free(err);
    return NULL;
}

/* ---------- info ---------- */

static void outline(GString *o, PopplerDocument *doc, PopplerIndexIter *it, int depth, int *count)
{
    g_string_append_c(o, '[');
    gboolean first = TRUE;
    do {
        if (++*count > 5000)
            break;
        PopplerAction *a = poppler_index_iter_get_action(it);
        if (!a)
            continue;
        int page = 0;
        if (a->type == POPPLER_ACTION_GOTO_DEST && a->goto_dest.dest) {
            PopplerDest *d = a->goto_dest.dest;
            if (d->type == POPPLER_DEST_NAMED) {
                PopplerDest *named = poppler_document_find_dest(doc, d->named_dest);
                if (named) {
                    page = named->page_num;
                    poppler_dest_free(named);
                }
            } else {
                page = d->page_num;
            }
        }
        if (!first)
            g_string_append(o, ", ");
        first = FALSE;
        g_string_append(o, "{\"title\": ");
        json_str(o, a->any.title);
        g_string_append_printf(o, ", \"page\": %d, \"children\": ", page);
        PopplerIndexIter *child = depth < 16 ? poppler_index_iter_get_child(it) : NULL;
        if (child) {
            outline(o, doc, child, depth + 1, count);
            poppler_index_iter_free(child);
        } else {
            g_string_append(o, "[]");
        }
        g_string_append_c(o, '}');
        poppler_action_free(a);
    } while (poppler_index_iter_next(it));
    g_string_append_c(o, ']');
}

static void field(GString *o, const char *key, char *value)
{
    g_string_append_printf(o, ", \"%s\": ", key);
    if (value && *value)
        json_str(o, value);
    else
        g_string_append(o, "null");
    g_free(value);
}

static int cmd_info(PopplerDocument *doc)
{
    int n = poppler_document_get_n_pages(doc);
    GString *o = g_string_new("{\"ok\": true, \"result\": {");
    g_string_append_printf(o, "\"pages\": %d", n);
    field(o, "title", poppler_document_get_title(doc));
    field(o, "author", poppler_document_get_author(doc));
    field(o, "subject", poppler_document_get_subject(doc));
    field(o, "creator", poppler_document_get_creator(doc));
    field(o, "producer", poppler_document_get_producer(doc));
    time_t made = poppler_document_get_creation_date(doc), changed = poppler_document_get_modification_date(doc);
    g_string_append_printf(o, ", \"created\": %lld, \"modified\": %lld", (long long)(made > 0 ? made : 0), (long long)(changed > 0 ? changed : 0));
    g_string_append(o, ", \"sizes\": [");
    for (int i = 0; i < n && i < 100000; i++) {
        PopplerPage *p = poppler_document_get_page(doc, i);
        double w = 612, h = 792;
        if (p) {
            poppler_page_get_size(p, &w, &h);
            g_object_unref(p);
        }
        g_string_append_printf(o, "%s[%.2f, %.2f]", i ? ", " : "", w, h);
    }
    g_string_append(o, "], \"outline\": ");
    PopplerIndexIter *it = poppler_index_iter_new(doc);
    int count = 0;
    if (it) {
        outline(o, doc, it, 0, &count);
        poppler_index_iter_free(it);
    } else {
        g_string_append(o, "[]");
    }
    g_string_append(o, "}}");
    emit(o);
    return 0;
}

/* ---------- search ---------- */

static int cmd_search(PopplerDocument *doc, const char *text, unsigned options)
{
    int n = poppler_document_get_n_pages(doc), total = 0, pages = 0;
    for (int i = 0; i < n; i++) {
        PopplerPage *p = poppler_document_get_page(doc, i);
        if (!p)
            continue;
        double w, h;
        poppler_page_get_size(p, &w, &h);
        GList *found = poppler_page_find_text_with_options(p, text, (PopplerFindFlags)options);
        if (found) {
            GString *o = g_string_new("{\"found\": {");
            g_string_append_printf(o, "\"page\": %d, \"rects\": [", i + 1);
            int k = 0;
            for (GList *l = found; l; l = l->next, k++) {
                PopplerRectangle *r = l->data;
                /* poppler's are from the bottom left: turn them to the top left. */
                g_string_append_printf(o, "%s[%.2f, %.2f, %.2f, %.2f]", k ? ", " : "", r->x1, h - r->y2, r->x2, h - r->y1);
                poppler_rectangle_free(r);
            }
            g_list_free(found);
            g_string_append(o, "]}}");
            emit(o);
            total += k;
            pages++;
        }
        g_object_unref(p);
    }
    GString *o = g_string_new("{\"ok\": true, \"result\": {");
    g_string_append_printf(o, "\"count\": %d, \"pages\": %d}}", total, pages);
    emit(o);
    return 0;
}

/* ---------- thumbnails ---------- */

static int cmd_thumbs(PopplerDocument *doc, const char *outdir, int width, char **pages, int n_pages)
{
    int n = poppler_document_get_n_pages(doc), made = 0;
    for (int i = 0; i < n_pages; i++) {
        int page = atoi(pages[i]);
        if (page < 1 || page > n)
            continue;
        PopplerPage *p = poppler_document_get_page(doc, page - 1);
        if (!p)
            continue;
        double w, h;
        poppler_page_get_size(p, &w, &h);
        double s = width / (w > 1 ? w : 1);
        int pw = width, ph = (int)(h * s + 0.5);
        if (ph < 1 || ph > 4 * width) {
            g_object_unref(p);
            continue;
        }
        cairo_surface_t *surf = cairo_image_surface_create(CAIRO_FORMAT_RGB24, pw, ph);
        cairo_t *cr = cairo_create(surf);
        cairo_set_source_rgb(cr, 1, 1, 1);
        cairo_paint(cr);
        cairo_scale(cr, s, s);
        poppler_page_render(p, cr);
        cairo_destroy(cr);
        char name[64];
        snprintf(name, sizeof name, "p%d-%d.png", page, width);
        char *file = g_build_filename(outdir, name, NULL);
        if (cairo_surface_write_to_png(surf, file) == CAIRO_STATUS_SUCCESS) {
            GString *o = g_string_new("{\"thumb\": {");
            g_string_append_printf(o, "\"page\": %d, \"png\": ", page);
            json_str(o, file);
            g_string_append(o, "}}");
            emit(o);
            made++;
        }
        g_free(file);
        cairo_surface_destroy(surf);
        g_object_unref(p);
    }
    GString *o = g_string_new("{\"ok\": true, \"result\": {");
    g_string_append_printf(o, "\"made\": %d}}", made);
    emit(o);
    return 0;
}

static int cmd_text(PopplerDocument *doc, int page)
{
    if (page < 1 || page > poppler_document_get_n_pages(doc))
        return fail("page", "There's no such page.");
    PopplerPage *p = poppler_document_get_page(doc, page - 1);
    if (!p)
        return fail("damaged", "That page can't be read.");
    char *text = poppler_page_get_text(p);
    GString *o = g_string_new("{\"ok\": true, \"result\": {\"text\": ");
    json_str(o, text);
    g_string_append(o, "}}");
    emit(o);
    g_free(text);
    g_object_unref(p);
    return 0;
}

static int usage(void)
{
    fprintf(stderr, "usage: nexa-pdfdoc info FILE | search FILE [--case] [--words] -- TEXT | thumbs FILE OUTDIR WIDTH PAGE... | text FILE PAGE\n"
                    "       (a password, if any, on standard input with --password-stdin before the command)\n");
    return 2;
}

int main(int argc, char **argv)
{
    int a = 1;
    if (a < argc && strcmp(argv[a], "--password-stdin") == 0) {
        char buf[1024];
        if (fgets(buf, sizeof buf, stdin)) {
            buf[strcspn(buf, "\r\n")] = 0;
            password = g_strdup(buf);
        }
        a++;
    }
    if (argc - a < 2)
        return usage();
    const char *cmd = argv[a], *path = argv[a + 1];
    if (!g_path_is_absolute(path))
        return fail("path", "The PDF must be given with its full path.");
    const char *outdir = NULL;
    if (strcmp(cmd, "thumbs") == 0) {
        if (argc - a < 5)
            return usage();
        outdir = argv[a + 2];
        if (g_mkdir_with_parents(outdir, 0700) != 0)
            return fail("unwritable", "The thumbnail folder can't be made.");
    }
    struct rlimit mem = { 3ULL << 30, 3ULL << 30 }, cpu = { 300, 300 };
    setrlimit(RLIMIT_AS, &mem);
    setrlimit(RLIMIT_CPU, &cpu);
    if (confine(path, outdir) < 0)
        return fail("confine", "The PDF engine couldn't confine itself.");
    int rc = 0;
    PopplerDocument *doc = open_doc(path, &rc);
    if (!doc)
        return rc;
    if (strcmp(cmd, "info") == 0) {
        rc = cmd_info(doc);
    } else if (strcmp(cmd, "search") == 0) {
        unsigned options = POPPLER_FIND_DEFAULT;
        const char *text = NULL;
        for (int i = a + 2; i < argc; i++) {
            if (strcmp(argv[i], "--case") == 0)
                options |= POPPLER_FIND_CASE_SENSITIVE;
            else if (strcmp(argv[i], "--words") == 0)
                options |= POPPLER_FIND_WHOLE_WORDS_ONLY;
            else if (strcmp(argv[i], "--") == 0 && i + 1 < argc) {
                text = argv[i + 1];
                break;
            }
        }
        rc = text && *text ? cmd_search(doc, text, options) : fail("text", "Nothing to look for.");
    } else if (strcmp(cmd, "thumbs") == 0) {
        rc = cmd_thumbs(doc, outdir, CLAMP(atoi(argv[a + 3]), 32, 1024), argv + a + 4, argc - a - 4);
    } else if (strcmp(cmd, "text") == 0 && argc - a >= 3) {
        rc = cmd_text(doc, atoi(argv[a + 2]));
    } else {
        rc = usage();
    }
    g_object_unref(doc);
    g_free(password);
    return rc;
}
