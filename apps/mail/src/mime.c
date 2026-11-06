#include "mime.h"

#include <ctype.h>
#include <stdio.h>
#include <string.h>

#define MAX_DEPTH 10
#define MAX_PARTS 200
#define MAX_HEADERS 1000

typedef struct {
    char *name, *value;
} Hdr;

static void hdr_free(Hdr *h)
{
    g_free(h->name);
    g_free(h->value);
    g_free(h);
}

/* Headers up to the first empty line; returns where the body starts. */
static const char *parse_headers(const char *p, const char *end, GPtrArray *out)
{
    Hdr *cur = NULL;
    while (p < end) {
        const char *nl = memchr(p, '\n', (size_t)(end - p));
        const char *le = nl ? nl : end;
        const char *line_end = le > p && le[-1] == '\r' ? le - 1 : le;
        if (line_end == p) return nl ? nl + 1 : end;  /* the empty line */
        if ((*p == ' ' || *p == '\t') && cur) {
            const char *s = p;
            while (s < line_end && (*s == ' ' || *s == '\t')) s++;
            char *joined = g_strdup_printf("%s %.*s", cur->value, (int)(line_end - s), s);
            g_free(cur->value);
            cur->value = joined;
        } else {
            const char *colon = memchr(p, ':', (size_t)(line_end - p));
            if (colon && out->len < MAX_HEADERS) {
                cur = g_new0(Hdr, 1);
                cur->name = g_ascii_strdown(p, colon - p);
                g_strstrip(cur->name);
                const char *v = colon + 1;
                while (v < line_end && (*v == ' ' || *v == '\t')) v++;
                cur->value = g_strndup(v, (gsize)(line_end - v));
                g_ptr_array_add(out, cur);
            } else {
                cur = NULL;  /* not a header line: skipped */
            }
        }
        if (!nl) return end;
        p = nl + 1;
    }
    return end;
}

static const char *hdr(GPtrArray *h, const char *name)
{
    for (guint i = 0; i < h->len; i++)
        if (!strcmp(((Hdr *)h->pdata[i])->name, name)) return ((Hdr *)h->pdata[i])->value;
    return NULL;
}

/* ---- charsets and encoded words ---- */

static char *to_utf8(const char *s, gsize n, const char *charset)
{
    if (!s) s = "", n = 0;  /* (empty bytes come back as NULL) */
    if (!charset || !*charset || !g_ascii_strcasecmp(charset, "utf-8") || !g_ascii_strcasecmp(charset, "us-ascii") ||
        !g_ascii_strcasecmp(charset, "utf8")) {
        char *raw = g_strndup(s, n), *ok = g_utf8_make_valid(raw, -1);
        g_free(raw);
        return ok;
    }
    char *cs = g_strdup(charset), *star = strchr(cs, '*');
    if (star) *star = 0;  /* "utf-8*en": the language part */
    char *out = g_convert_with_fallback(s, (gssize)n, "UTF-8", cs, "?", NULL, NULL, NULL);
    if (!out && !g_ascii_strncasecmp(cs, "iso-8859", 8)) out = g_convert_with_fallback(s, (gssize)n, "UTF-8", "WINDOWS-1252", "?", NULL, NULL, NULL);
    g_free(cs);
    if (!out) {  /* an unknown charset: what is UTF-8 in it */
        char *raw = g_strndup(s, n);
        out = g_utf8_make_valid(raw, -1);
        g_free(raw);
    }
    return out;
}

static int hexv(int c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

GBytes *mime_decode_qp(const char *s, gsize n)
{
    GByteArray *o = g_byte_array_sized_new((guint)n);
    for (gsize i = 0; i < n; i++) {
        if (s[i] == '=' && i + 1 < n && (s[i + 1] == '\n' || (s[i + 1] == '\r' && i + 2 < n && s[i + 2] == '\n'))) {
            i += s[i + 1] == '\r' ? 2 : 1;  /* a soft line break */
        } else if (s[i] == '=' && i + 2 < n && hexv(s[i + 1]) >= 0 && hexv(s[i + 2]) >= 0) {
            guint8 b = (guint8)(hexv(s[i + 1]) * 16 + hexv(s[i + 2]));
            g_byte_array_append(o, &b, 1);
            i += 2;
        } else {
            g_byte_array_append(o, (const guint8 *)s + i, 1);
        }
    }
    return g_byte_array_free_to_bytes(o);
}

static GBytes *decode_b64(const char *s, gsize n)
{
    guchar *out = g_malloc(n * 3 / 4 + 4);
    gint state = 0;
    guint save = 0;
    gsize len = g_base64_decode_step(s, n, out, &state, &save);  /* (skips anything not base64) */
    return g_bytes_new_take(out, len);
}

char *mime_decode_words(const char *s)
{
    if (!s) return g_strdup("");
    GString *o = g_string_new(NULL);
    const char *p = s;
    gboolean last_word = FALSE;
    while (*p) {
        const char *w = strstr(p, "=?");
        if (!w) {
            g_string_append(o, p);
            break;
        }
        /* =?charset?X?text?= */
        const char *q1 = strchr(w + 2, '?');
        const char *q2 = q1 ? strchr(q1 + 1, '?') : NULL;
        const char *e = q2 ? strstr(q2 + 1, "?=") : NULL;
        if (!q1 || !q2 || !e || q2 != q1 + 2 || e - w > 1000) {
            g_string_append_len(o, p, w - p + 2);
            p = w + 2;
            last_word = FALSE;
            continue;
        }
        /* whitespace between two encoded words is dropped */
        gboolean only_space = TRUE;
        for (const char *c = p; c < w; c++) only_space &= (*c == ' ' || *c == '\t' || *c == '\r' || *c == '\n');
        if (!(last_word && only_space)) g_string_append_len(o, p, w - p);
        char *cs = g_strndup(w + 2, (gsize)(q1 - w - 2));
        char kind = (char)g_ascii_toupper(q1[1]);
        const char *t = q2 + 1;
        gsize tn = (gsize)(e - t);
        GBytes *b = NULL;
        if (kind == 'B') {
            b = decode_b64(t, tn);
        } else if (kind == 'Q') {
            char *u = g_strndup(t, tn);
            for (char *c = u; *c; c++)
                if (*c == '_') *c = ' ';
            b = mime_decode_qp(u, tn);
            g_free(u);
        }
        if (b) {
            gsize bn;
            const char *bd = g_bytes_get_data(b, &bn);
            char *u = to_utf8(bd, bn, cs);
            g_string_append(o, u);
            g_free(u);
            g_bytes_unref(b);
            last_word = TRUE;
        } else {
            g_string_append_len(o, w, e + 2 - w);
            last_word = FALSE;
        }
        g_free(cs);
        p = e + 2;
    }
    char *raw = g_string_free(o, FALSE);
    char *out = g_utf8_validate(raw, -1, NULL) ? g_strdup(raw) : to_utf8(raw, strlen(raw), "windows-1252");
    g_free(raw);
    /* no control characters in a header for people */
    for (char *c = out; *c; c++)
        if ((unsigned char)*c < 32) *c = ' ';
    return g_strstrip(out);
}

/* "text/plain; charset=\"x\"; name*=utf-8''a%20b" */
static char *param(const char *value, const char *want)
{
    if (!value) return NULL;
    size_t wl = strlen(want);
    const char *p = strchr(value, ';');
    GString *joined = NULL;  /* name*0*, name*1* ... */
    char *found = NULL;
    while (p) {
        p++;
        while (*p == ' ' || *p == '\t') p++;
        const char *eq = strchr(p, '=');
        if (!eq) break;
        char *key = g_ascii_strdown(p, eq - p);
        g_strstrip(key);
        const char *v = eq + 1;
        while (*v == ' ') v++;
        GString *val = g_string_new(NULL);
        if (*v == '"') {
            for (v++; *v && *v != '"'; v++) {
                if (*v == '\\' && v[1]) v++;
                g_string_append_c(val, *v);
            }
            if (*v == '"') v++;
        } else {
            while (*v && *v != ';') g_string_append_c(val, *v++);
            g_strchomp(val->str);
            val->len = strlen(val->str);
        }
        gboolean ext = FALSE;
        if (!strncmp(key, want, wl)) {
            const char *rest = key + wl;
            if (!*rest) {
                if (!found) found = g_strdup(val->str);
            } else if (rest[0] == '*') {
                ext = rest[strlen(rest) - 1] == '*';
                const char *text = val->str;
                if (ext && (!strcmp(rest, "*") || !strcmp(rest, "*0*"))) {  /* charset'lang'value */
                    const char *a = strchr(text, '\''), *b = a ? strchr(a + 1, '\'') : NULL;
                    if (b) text = b + 1;
                }
                if (!joined) joined = g_string_new(NULL);
                if (ext) {
                    for (const char *c = text; *c; c++) {
                        if (*c == '%' && hexv(c[1]) >= 0 && hexv(c[2]) >= 0) {
                            g_string_append_c(joined, (char)(hexv(c[1]) * 16 + hexv(c[2])));
                            c += 2;
                        } else {
                            g_string_append_c(joined, *c);
                        }
                    }
                } else {
                    g_string_append(joined, text);
                }
            }
        }
        g_string_free(val, TRUE);
        g_free(key);
        p = strchr(v, ';');
    }
    if (joined) {
        g_free(found);
        char *u = g_utf8_make_valid(joined->str, -1);
        g_string_free(joined, TRUE);
        return u;
    }
    if (found && strstr(found, "=?")) {
        char *d = mime_decode_words(found);
        g_free(found);
        return d;
    }
    return found;
}

static char *mime_type(const char *ct)
{
    if (!ct) return g_strdup("text/plain");
    const char *semi = strchr(ct, ';');
    char *t = g_ascii_strdown(ct, semi ? semi - ct : (gssize)strlen(ct));
    g_strstrip(t);
    if (!*t || !strchr(t, '/')) {
        g_free(t);
        return g_strdup("text/plain");
    }
    return t;
}

char *mime_safe_filename(const char *name)
{
    if (!name || !*name) return g_strdup("attachment");
    const char *base = name;
    for (const char *c = name; *c; c++)
        if (*c == '/' || *c == '\\') base = c + 1;
    GString *o = g_string_new(NULL);
    for (const char *c = base; *c; c++) g_string_append_c(o, (unsigned char)*c < 32 || *c == 127 || *c == ':' ? '_' : *c);
    char *s = g_strstrip(g_string_free(o, FALSE));
    while (*s == '.') memmove(s, s + 1, strlen(s));  /* no hidden files, no ".." */
    if (!*s) {
        g_free(s);
        return g_strdup("attachment");
    }
    if (strlen(s) > 200) s[200] = 0;
    char *ok = g_utf8_make_valid(s, -1);
    g_free(s);
    return ok;
}

/* ---- HTML to text ---- */

static const struct { const char *name; const char *text; } ENTITIES[] = {
    {"amp", "&"}, {"lt", "<"}, {"gt", ">"}, {"quot", "\""}, {"apos", "'"}, {"nbsp", " "}, {"copy", "©"}, {"reg", "®"},
    {"hellip", "…"}, {"mdash", "—"}, {"ndash", "–"}, {"rsquo", "’"}, {"lsquo", "‘"}, {"rdquo", "”"}, {"ldquo", "“"},
    {"euro", "€"}, {"pound", "£"}, {"bull", "•"}, {"middot", "·"}, {"zwnj", ""}, {"zwj", ""}, {"shy", ""}, {NULL, NULL},
};

static void entity(GString *o, const char **pp)
{
    const char *p = *pp + 1, *semi = strchr(p, ';');
    if (!semi || semi - p > 10) {
        g_string_append_c(o, '&');
        return;
    }
    if (*p == '#') {
        gunichar u = p[1] == 'x' || p[1] == 'X' ? (gunichar)strtoul(p + 2, NULL, 16) : (gunichar)strtoul(p + 1, NULL, 10);
        if (u && g_unichar_validate(u) && u != 0xfeff) g_string_append_unichar(o, u < 32 ? ' ' : u);
        *pp = semi;
        return;
    }
    for (int i = 0; ENTITIES[i].name; i++)
        if ((size_t)(semi - p) == strlen(ENTITIES[i].name) && !strncmp(p, ENTITIES[i].name, (size_t)(semi - p))) {
            g_string_append(o, ENTITIES[i].text);
            *pp = semi;
            return;
        }
    g_string_append_c(o, '&');
}

static char *attr(const char *tag, const char *name)
{
    size_t n = strlen(name);
    for (const char *p = tag; (p = strstr(p, name)); p++) {
        if (p > tag && !isspace((unsigned char)p[-1])) continue;
        const char *v = p + n;
        while (*v == ' ') v++;
        if (*v != '=') continue;
        v++;
        while (*v == ' ') v++;
        char q = *v == '"' || *v == '\'' ? *v++ : 0;
        const char *e = v;
        while (*e && (q ? *e != q : !isspace((unsigned char)*e) && *e != '>')) e++;
        GString *o = g_string_new(NULL);
        char *raw = g_strndup(v, (gsize)(e - v));
        for (const char *c = raw; *c; c++) {
            if (*c == '&') entity(o, &c);
            else g_string_append_c(o, *c);
        }
        g_free(raw);
        return g_string_free(o, FALSE);
    }
    return NULL;
}

char *mime_html_to_text(const char *html)
{
    GString *o = g_string_new(NULL);
    const char *p = html ? html : "";
    char *href = NULL;
    size_t link_start = 0;
    int skip = 0;  /* inside <script>, <style>, <head> */
    while (*p) {
        if (!strncmp(p, "<!--", 4)) {
            const char *e = strstr(p + 4, "-->");
            p = e ? e + 3 : p + strlen(p);
            continue;
        }
        if (*p == '<') {
            const char *e = strchr(p, '>');
            if (!e) break;
            char *tag = g_ascii_strdown(p + 1, e - p - 1);
            gboolean close = tag[0] == '/';
            const char *name = close ? tag + 1 : tag;
            size_t nl = strcspn(name, " \t\r\n/");
            char *tn = g_strndup(name, nl);
            if (!strcmp(tn, "script") || !strcmp(tn, "style") || !strcmp(tn, "head") || !strcmp(tn, "title")) {
                skip += close ? -1 : 1;
                if (skip < 0) skip = 0;
            } else if (!skip) {
                static const char *const BLOCK[] = {"p", "div", "tr", "table", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "ul", "ol",
                                                    "section", "article", "header", "footer", "center", "pre", NULL};
                for (int i = 0; BLOCK[i]; i++)
                    if (!strcmp(tn, BLOCK[i])) g_string_append_c(o, '\n');
                if (!strcmp(tn, "br")) g_string_append_c(o, '\n');
                else if (!strcmp(tn, "li") && !close) g_string_append(o, "\n• ");
                else if (!strcmp(tn, "td") && close) g_string_append_c(o, ' ');
                else if (!strcmp(tn, "hr")) g_string_append(o, "\n——————\n");
                else if (!strcmp(tn, "img") && !close) {
                    char *alt = attr(p + 1, "alt");  /* the picture itself is never loaded */
                    if (alt && *g_strstrip(alt)) g_string_append_printf(o, "[%s]", alt);
                    g_free(alt);
                } else if (!strcmp(tn, "a")) {
                    if (!close) {
                        g_free(href);
                        href = attr(p + 1, "href");
                        link_start = o->len;
                    } else if (href) {
                        char *text = g_strndup(o->str + link_start, o->len - link_start);
                        g_strstrip(text);
                        if ((g_str_has_prefix(href, "http://") || g_str_has_prefix(href, "https://")) && strcmp(text, href))
                            g_string_append_printf(o, " <%s>", href);
                        g_free(text);
                        g_clear_pointer(&href, g_free);
                    }
                }
            }
            g_free(tn);
            g_free(tag);
            p = e + 1;
            continue;
        }
        if (skip) {
            p++;
            continue;
        }
        if (*p == '&') entity(o, &p);
        else if (*p == '\r' || *p == '\n' || *p == '\t') g_string_append_c(o, ' ');
        else g_string_append_c(o, *p);
        p++;
    }
    g_free(href);
    /* tidy: no runs of spaces, at most one empty line in a row */
    GString *t = g_string_new(NULL);
    int nls = 0;
    gboolean space = FALSE;
    for (const char *c = o->str; *c; c++) {
        if (*c == '\n') {
            while (t->len && t->str[t->len - 1] == ' ') g_string_truncate(t, t->len - 1);
            if (nls < 2 && t->len) g_string_append_c(t, '\n');
            nls++;
            space = FALSE;
        } else if (*c == ' ') {
            space = TRUE;
        } else {
            if (space && nls == 0 && t->len) g_string_append_c(t, ' ');
            space = FALSE;
            nls = 0;
            g_string_append_c(t, *c);
        }
    }
    g_string_free(o, TRUE);
    char *out = g_strstrip(g_string_free(t, FALSE));
    char *ok = g_utf8_make_valid(out, -1);
    g_free(out);
    return ok;
}

/* ---- addresses and dates ---- */

void address_free(Address *a)
{
    if (!a) return;
    g_free(a->name);
    g_free(a->addr);
    g_free(a);
}

gboolean address_valid(const char *addr)
{
    if (!addr || strlen(addr) > 254) return FALSE;
    const char *at = strchr(addr, '@');
    if (!at || at == addr || strchr(at + 1, '@') || !at[1]) return FALSE;
    for (const char *c = addr; *c; c++)
        if ((unsigned char)*c <= 32 || strchr("<>(),;:\\\"[]", *c)) return FALSE;
    const char *dom = at + 1;
    return strchr(dom, '.') != NULL || !strcmp(dom, "localhost");
}

static Address *one_address(const char *s)
{
    char *t = g_strstrip(g_strdup(s));
    Address *a = g_new0(Address, 1);
    char *lt = strrchr(t, '<'), *gt = lt ? strchr(lt, '>') : NULL;
    if (lt && gt) {
        a->addr = g_strstrip(g_strndup(lt + 1, (gsize)(gt - lt - 1)));
        *lt = 0;
        char *n = g_strstrip(t);
        if (*n == '"' && strlen(n) >= 2 && n[strlen(n) - 1] == '"') {
            n[strlen(n) - 1] = 0;
            n++;
        }
        GString *un = g_string_new(NULL);
        for (const char *c = n; *c; c++) {
            if (*c == '\\' && c[1]) c++;
            g_string_append_c(un, *c);
        }
        a->name = mime_decode_words(un->str);
        g_string_free(un, TRUE);
    } else {
        char *paren = strchr(t, '(');
        if (paren) {  /* "a@x (Ama)" */
            char *close = strchr(paren, ')');
            char *inner = close ? g_strndup(paren + 1, (gsize)(close - paren - 1)) : g_strdup("");
            a->name = mime_decode_words(inner);
            g_free(inner);
            *paren = 0;
        } else {
            a->name = g_strdup("");
        }
        a->addr = g_strstrip(g_strdup(t));
    }
    g_free(t);
    char *low = g_ascii_strdown(a->addr, -1);
    g_free(a->addr);
    a->addr = low;
    if (!address_valid(a->addr)) {
        address_free(a);
        return NULL;
    }
    return a;
}

GPtrArray *mime_addresses(const char *s)
{
    GPtrArray *out = g_ptr_array_new_with_free_func((GDestroyNotify)address_free);
    if (!s) return out;
    GString *cur = g_string_new(NULL);
    gboolean quoted = FALSE;
    int angle = 0;
    for (const char *c = s;; c++) {
        if (!*c || (*c == ',' && !quoted && !angle) || (*c == ';' && !quoted && !angle)) {
            /* "Group: a@x, b@y;" — the group's name goes, its addresses stay */
            const char *part = cur->str, *colon = NULL;
            gboolean q = FALSE;
            for (const char *x = cur->str; *x && *x != '<' && *x != '@'; x++) {
                if (*x == '"') q = !q;
                else if (*x == ':' && !q) colon = x;
            }
            if (colon) part = colon + 1;
            if (*part && out->len < 500) {
                Address *a = one_address(part);
                if (a) g_ptr_array_add(out, a);
            }
            g_string_truncate(cur, 0);
            if (!*c) break;
            continue;
        }
        if (*c == '"' && (c == s || c[-1] != '\\')) quoted = !quoted;
        else if (*c == '<' && !quoted) angle++;
        else if (*c == '>' && !quoted && angle) angle--;
        g_string_append_c(cur, *c);
    }
    g_string_free(cur, TRUE);
    return out;
}

char *address_format(const Address *a)
{
    if (!a) return g_strdup("");
    return a->name && *a->name ? g_strdup_printf("%s <%s>", a->name, a->addr) : g_strdup(a->addr);
}

static gint64 days_from_civil(int y, int m, int d)
{
    y -= m <= 2;
    gint64 era = (y >= 0 ? y : y - 399) / 400, yoe = y - era * 400;
    gint64 doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    return era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
}

static const char *const MONTHS[12] = {"jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"};
static const char *const MON_NAMES[12] = {"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"};
static const char *const DAY_NAMES[7] = {"Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"};  /* 1 Jan 1970 was a Thursday */

gint64 mime_date(const char *s)
{
    if (!s) return 0;
    const char *p = strchr(s, ',');
    p = p ? p + 1 : s;
    int d = 0, y = 0, h = 0, mi = 0, se = 0;
    char mon[8] = "", zone[16] = "";
    int n = sscanf(p, " %d %7s %d %d:%d:%d %15s", &d, mon, &y, &h, &mi, &se, zone);
    if (n < 5) return 0;
    if (n == 5) {
        sscanf(p, " %d %7s %d %d:%d %15s", &d, mon, &y, &h, &mi, zone);
        se = 0;
    }
    if (y < 100) y += y < 50 ? 2000 : 1900;
    int m = -1;
    for (int i = 0; i < 12; i++)
        if (!g_ascii_strncasecmp(mon, MONTHS[i], 3)) m = i + 1;
    if (m < 0 || d < 1 || d > 31 || y < 1970 || y > 2200 || h > 23 || mi > 59 || se > 60) return 0;
    gint64 off = 0;
    if ((zone[0] == '+' || zone[0] == '-') && strlen(zone) >= 5 && isdigit((unsigned char)zone[1])) {
        int z = atoi(zone + 1);
        off = (z / 100) * 3600 + (z % 100) * 60;
        if (zone[0] == '-') off = -off;
    } else if (!g_ascii_strcasecmp(zone, "EST")) off = -5 * 3600;
    else if (!g_ascii_strcasecmp(zone, "EDT")) off = -4 * 3600;
    else if (!g_ascii_strcasecmp(zone, "PST")) off = -8 * 3600;
    else if (!g_ascii_strcasecmp(zone, "PDT")) off = -7 * 3600;
    return days_from_civil(y, m, d) * 86400 + h * 3600 + mi * 60 + se - off;
}

/* ---- a whole message ---- */

static void attachment_free(Attachment *a)
{
    g_free(a->filename);
    g_free(a->type);
    if (a->data) g_bytes_unref(a->data);
    g_free(a);
}

void message_free(Message *m)
{
    if (!m) return;
    g_free(m->subject);
    g_free(m->message_id);
    g_free(m->in_reply_to);
    g_free(m->references);
    g_free(m->text);
    g_ptr_array_free(m->from, TRUE);
    g_ptr_array_free(m->to, TRUE);
    g_ptr_array_free(m->cc, TRUE);
    g_ptr_array_free(m->reply_to, TRUE);
    g_ptr_array_free(m->attachments, TRUE);
    g_free(m);
}

typedef struct {
    Message *m;
    char *plain, *html;
    int parts;
    gsize decoded;
} Parse;

static GBytes *decode_body(const char *cte, const char *b, gsize n)
{
    if (cte && !g_ascii_strncasecmp(cte, "base64", 6)) return decode_b64(b, n);
    if (cte && !g_ascii_strncasecmp(cte, "quoted-printable", 16)) return mime_decode_qp(b, n);
    return g_bytes_new(b, n);
}

static void entity_parse(Parse *ps, const char *start, const char *end, int depth);

static void multipart(Parse *ps, const char *body, const char *end, const char *boundary, int depth)
{
    char *delim = g_strdup_printf("--%s", boundary);
    size_t dl = strlen(delim);
    const char *p = body, *part = NULL;
    while (p < end) {
        /* the delimiter starts a line */
        const char *hit = NULL;
        for (const char *q = p; q + dl <= end; q++) {
            q = memchr(q, '-', (size_t)(end - q));
            if (!q || q + dl > end) break;
            if ((q == body || q[-1] == '\n') && !memcmp(q, delim, dl)) {
                hit = q;
                break;
            }
        }
        if (!hit) break;
        if (part) {
            const char *pe = hit;
            if (pe > part && pe[-1] == '\n') pe--;
            if (pe > part && pe[-1] == '\r') pe--;
            entity_parse(ps, part, pe, depth + 1);
        }
        const char *after = hit + dl;
        if (after + 1 < end && after[0] == '-' && after[1] == '-') {
            part = NULL;
            break;  /* the closing delimiter */
        }
        const char *nl = memchr(after, '\n', (size_t)(end - after));
        part = nl ? nl + 1 : end;
        p = part;
    }
    if (part && part < end) entity_parse(ps, part, end, depth + 1);  /* no closing delimiter: the rest */
    g_free(delim);
}

static void entity_parse(Parse *ps, const char *start, const char *end, int depth)
{
    if (depth > MAX_DEPTH || ps->parts++ > MAX_PARTS || start >= end) return;
    GPtrArray *h = g_ptr_array_new_with_free_func((GDestroyNotify)hdr_free);
    const char *body = parse_headers(start, end, h);
    const char *ct = hdr(h, "content-type"), *cd = hdr(h, "content-disposition"), *cte = hdr(h, "content-transfer-encoding");
    char *type = mime_type(ct);
    char *boundary = g_str_has_prefix(type, "multipart/") ? param(ct, "boundary") : NULL;
    gboolean attach = cd && !g_ascii_strncasecmp(cd, "attachment", 10);
    char *fname = param(cd, "filename");
    if (!fname) fname = param(ct, "name");
    if (boundary && *boundary) {
        multipart(ps, body, end, boundary, depth);
    } else if (!attach && !fname && (!strcmp(type, "text/plain") || !strcmp(type, "text/html"))) {
        GBytes *b = decode_body(cte, body, (gsize)(end - body));
        gsize n;
        const char *d = g_bytes_get_data(b, &n);
        char *cs = param(ct, "charset"), *text = to_utf8(d, n, cs);
        char **slot = !strcmp(type, "text/plain") ? &ps->plain : &ps->html;
        if (!*slot) *slot = text;
        else g_free(text);
        g_free(cs);
        g_bytes_unref(b);
    } else if (ps->m->attachments->len < 100 && ps->decoded < 64u * 1024 * 1024) {
        Attachment *a = g_new0(Attachment, 1);
        a->type = g_strdup(type);
        a->data = decode_body(cte, body, (gsize)(end - body));
        ps->decoded += g_bytes_get_size(a->data);
        if (!fname && !strcmp(type, "message/rfc822")) fname = g_strdup("message.eml");
        a->filename = mime_safe_filename(fname);
        g_ptr_array_add(ps->m->attachments, a);
    }
    g_free(fname);
    g_free(boundary);
    g_free(type);
    g_ptr_array_free(h, TRUE);
}

static Message *new_message(GPtrArray *h)
{
    Message *m = g_new0(Message, 1);
    m->subject = mime_decode_words(hdr(h, "subject"));
    m->from = mime_addresses(hdr(h, "from"));
    m->to = mime_addresses(hdr(h, "to"));
    m->cc = mime_addresses(hdr(h, "cc"));
    m->reply_to = mime_addresses(hdr(h, "reply-to"));
    m->date = mime_date(hdr(h, "date"));
    const char *id = hdr(h, "message-id"), *irt = hdr(h, "in-reply-to"), *refs = hdr(h, "references");
    m->message_id = g_strstrip(g_strdup(id ? id : ""));
    m->in_reply_to = g_strstrip(g_strdup(irt ? irt : ""));
    m->references = g_strstrip(g_strdup(refs ? refs : ""));
    for (char *c = m->references; *c; c++)
        if ((unsigned char)*c < 32) *c = ' ';
    m->attachments = g_ptr_array_new_with_free_func((GDestroyNotify)attachment_free);
    return m;
}

Message *mime_parse_headers(const char *raw, gsize len)
{
    GPtrArray *h = g_ptr_array_new_with_free_func((GDestroyNotify)hdr_free);
    parse_headers(raw ? raw : "", raw ? raw + len : raw, h);
    Message *m = new_message(h);
    m->text = g_strdup("");
    g_ptr_array_free(h, TRUE);
    return m;
}

Message *mime_parse(const char *raw, gsize len)
{
    if (!raw) raw = "", len = 0;
    GPtrArray *h = g_ptr_array_new_with_free_func((GDestroyNotify)hdr_free);
    parse_headers(raw, raw + len, h);
    Message *m = new_message(h);
    g_ptr_array_free(h, TRUE);
    Parse ps = {m, NULL, NULL, 0, 0};
    entity_parse(&ps, raw, raw + len, 0);
    if (ps.plain && *g_strstrip(ps.plain)) {
        m->text = ps.plain;
        ps.plain = NULL;
    } else if (ps.html) {
        m->text = mime_html_to_text(ps.html);
        m->was_html = TRUE;
    } else {
        m->text = g_strdup("");
    }
    /* \r\n -> \n for the screen */
    GString *t = g_string_new(NULL);
    for (const char *c = m->text; *c; c++)
        if (*c != '\r') g_string_append_c(t, *c);
    g_free(m->text);
    m->text = g_string_free(t, FALSE);
    g_free(ps.plain);
    g_free(ps.html);
    return m;
}

/* ---- writing ---- */

/* A header value: never a line break in it (no header can be slipped in). */
static char *one_line(const char *s)
{
    GString *o = g_string_new(NULL);
    for (const char *c = s ? s : ""; *c; c++) g_string_append_c(o, (*c == '\r' || *c == '\n' || *c == '\t') ? ' ' : *c);
    return g_string_free(o, FALSE);
}

static gboolean plain_ascii(const char *s)
{
    for (const char *c = s; *c; c++)
        if ((unsigned char)*c >= 127 || (unsigned char)*c < 32) return FALSE;
    return TRUE;
}

/* =?UTF-8?B?...?= in pieces short enough for a line, never splitting a character. */
static char *encode_word(const char *s)
{
    char *line = one_line(s);
    if (plain_ascii(line)) return line;
    GString *o = g_string_new(NULL);
    const char *p = line;
    while (*p) {
        const char *q = p;
        while (*q && q - p < 39) q = g_utf8_next_char(q);
        char *b = g_base64_encode((const guchar *)p, (gsize)(q - p));
        g_string_append_printf(o, "%s=?UTF-8?B?%s?=", o->len ? " " : "", b);
        g_free(b);
        p = q;
    }
    g_free(line);
    return g_string_free(o, FALSE);
}

static void append_addresses(GString *o, const char *field, GPtrArray *list)
{
    if (!list || !list->len) return;
    g_string_append_printf(o, "%s: ", field);
    int written = 0;
    for (guint i = 0; i < list->len; i++) {
        Address *a = list->pdata[i];
        if (!address_valid(a->addr)) continue;
        if (written++) g_string_append(o, ",\r\n ");
        if (a->name && *a->name) {
            char *n = encode_word(a->name);
            if (plain_ascii(n) && strpbrk(n, "()<>[]:;@\\,.\"")) {
                GString *q = g_string_new("\"");
                for (const char *c = n; *c; c++) {
                    if (*c == '"' || *c == '\\') g_string_append_c(q, '\\');
                    g_string_append_c(q, *c);
                }
                g_string_append_c(q, '"');
                g_string_append_printf(o, "%s <%s>", q->str, a->addr);
                g_string_free(q, TRUE);
            } else {
                g_string_append_printf(o, "%s <%s>", n, a->addr);
            }
            g_free(n);
        } else {
            g_string_append(o, a->addr);
        }
    }
    g_string_append(o, "\r\n");
}

static void qp_encode(GString *o, const char *text)
{
    const char *p = text;
    while (*p) {
        const char *nl = strchr(p, '\n');
        size_t n = nl ? (size_t)(nl - p) : strlen(p);
        if (n && p[n - 1] == '\r') n--;
        int col = 0;
        for (size_t i = 0; i < n; i++) {
            unsigned char c = (unsigned char)p[i];
            char piece[4];
            gboolean last = i + 1 == n;
            if ((c >= 33 && c <= 126 && c != '=') || ((c == ' ' || c == '\t') && !last)) {
                piece[0] = (char)c;
                piece[1] = 0;
            } else {
                snprintf(piece, sizeof piece, "=%02X", c);
            }
            if (col + (int)strlen(piece) > 75) {
                g_string_append(o, "=\r\n");
                col = 0;
            }
            g_string_append(o, piece);
            col += (int)strlen(piece);
        }
        g_string_append(o, "\r\n");
        if (!nl) break;
        p = nl + 1;
    }
}

static char *random_token(int bytes)
{
    GString *s = g_string_new(NULL);
    for (int i = 0; i < bytes; i++) g_string_append_printf(s, "%02x", g_random_int_range(0, 256));
    return g_string_free(s, FALSE);
}

char *mime_build(const Draft *d, char **message_id, gsize *len)
{
    GString *o = g_string_new(NULL);
    gint64 t = d->date ? d->date : g_get_real_time() / G_USEC_PER_SEC;
    gint64 days = t / 86400, sec = t % 86400;
    int y, m, dd;
    {  /* civil from days */
        gint64 z = days + 719468, era = (z >= 0 ? z : z - 146096) / 146097, doe = z - era * 146097;
        gint64 yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365, doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        gint64 mp = (5 * doy + 2) / 153;
        dd = (int)(doy - (153 * mp + 2) / 5 + 1);
        m = (int)(mp < 10 ? mp + 3 : mp - 9);
        y = (int)(yoe + era * 400 + (m <= 2));
    }
    g_string_append_printf(o, "Date: %s, %02d %s %04d %02d:%02d:%02d +0000\r\n", DAY_NAMES[((days % 7) + 7) % 7], dd, MON_NAMES[m - 1], y,
                           (int)(sec / 3600), (int)(sec / 60 % 60), (int)(sec % 60));
    Address from = {(char *)(d->from_name ? d->from_name : ""), (char *)d->from_addr};
    GPtrArray one = {.pdata = (gpointer *)(Address *[]){&from}, .len = 1};
    append_addresses(o, "From", &one);
    append_addresses(o, "To", d->to);
    append_addresses(o, "Cc", d->cc);
    char *subj = encode_word(d->subject ? d->subject : "");
    g_string_append_printf(o, "Subject: %s\r\n", subj);
    g_free(subj);
    const char *at = d->from_addr ? strchr(d->from_addr, '@') : NULL;
    char *tok = random_token(12), *id = g_strdup_printf("<%s@%s>", tok, at ? at + 1 : "localhost");
    g_string_append_printf(o, "Message-ID: %s\r\n", id);
    if (d->in_reply_to && *d->in_reply_to) {
        char *a = one_line(d->in_reply_to), *b = one_line(d->references && *d->references ? d->references : "");
        g_string_append_printf(o, "In-Reply-To: %s\r\nReferences: %s%s%s\r\n", a, b, *b ? " " : "", a);
        g_free(a);
        g_free(b);
    }
    g_string_append(o, "MIME-Version: 1.0\r\n");
    gboolean mixed = d->attachments && d->attachments->len;
    char *bnd = random_token(16);
    if (mixed) {
        g_string_append_printf(o, "Content-Type: multipart/mixed; boundary=\"=_%s\"\r\n\r\n", bnd);
        g_string_append(o, "This is a message in several parts.\r\n\r\n");
        g_string_append_printf(o, "--=_%s\r\n", bnd);
    }
    g_string_append(o, "Content-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n");
    char *body = g_utf8_make_valid(d->body ? d->body : "", -1);
    qp_encode(o, body);
    g_free(body);
    for (guint i = 0; mixed && i < d->attachments->len; i++) {
        Attachment *a = d->attachments->pdata[i];
        char *safe = mime_safe_filename(a->filename), *enc = encode_word(safe);
        g_string_append_printf(o, "--=_%s\r\nContent-Type: %s; name=\"%s\"\r\nContent-Transfer-Encoding: base64\r\n", bnd,
                               a->type && strchr(a->type, '/') && plain_ascii(a->type) ? a->type : "application/octet-stream", enc);
        if (plain_ascii(safe) && !strchr(safe, '"')) {
            g_string_append_printf(o, "Content-Disposition: attachment; filename=\"%s\"\r\n\r\n", safe);
        } else {
            GString *pct = g_string_new(NULL);
            for (const unsigned char *c = (const unsigned char *)safe; *c; c++) {
                if (g_ascii_isalnum(*c) || strchr(".-_", *c)) g_string_append_c(pct, (char)*c);
                else g_string_append_printf(pct, "%%%02X", *c);
            }
            g_string_append_printf(o, "Content-Disposition: attachment; filename*=UTF-8''%s\r\n\r\n", pct->str);
            g_string_free(pct, TRUE);
        }
        gsize n;
        const guchar *data = g_bytes_get_data(a->data, &n);
        char *b64 = g_base64_encode(data, n);
        for (size_t k = 0, L = strlen(b64); k < L; k += 76) g_string_append_printf(o, "%.*s\r\n", 76, b64 + k);
        g_free(b64);
        g_free(safe);
        g_free(enc);
    }
    if (mixed) g_string_append_printf(o, "--=_%s--\r\n", bnd);
    g_free(bnd);
    g_free(tok);
    if (message_id) *message_id = id;
    else g_free(id);
    if (len) *len = o->len;
    return g_string_free(o, FALSE);
}

char *mime_reply_subject(const char *s, gboolean forward)
{
    char *t = g_strstrip(one_line(s ? s : ""));
    char *out;
    if (forward) out = g_ascii_strncasecmp(t, "fwd:", 4) && g_ascii_strncasecmp(t, "fw:", 3) ? g_strdup_printf("Fwd: %s", t) : g_strdup(t);
    else out = g_ascii_strncasecmp(t, "re:", 3) ? g_strdup_printf("Re: %s", t) : g_strdup(t);
    g_free(t);
    return out;
}

char *mime_quote(const Message *m)
{
    GString *o = g_string_new("\n\n");
    const Address *a = m->from && m->from->len ? m->from->pdata[0] : NULL;
    char *who = a ? (a->name && *a->name ? g_strdup(a->name) : g_strdup(a->addr)) : g_strdup("someone");
    if (m->date) {
        GDateTime *dt = g_date_time_new_from_unix_local(m->date);
        char *when = dt ? g_date_time_format(dt, "%e %B %Y, %H:%M") : g_strdup("");
        g_string_append_printf(o, "On %s, %s wrote:\n", g_strstrip(when), who);
        g_free(when);
        if (dt) g_date_time_unref(dt);
    } else {
        g_string_append_printf(o, "%s wrote:\n", who);
    }
    g_free(who);
    char **lines = g_strsplit(m->text ? m->text : "", "\n", -1);
    for (int i = 0; lines[i]; i++) g_string_append_printf(o, ">%s%s\n", lines[i][0] == '>' ? "" : " ", lines[i]);
    g_strfreev(lines);
    return g_string_free(o, FALSE);
}
