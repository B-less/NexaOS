#include "imap.h"

#include <curl/curl.h>
#include <string.h>

#define MAX_LIST_DEPTH 32
#define MAX_RESPONSE (256u * 1024 * 1024)

/* ---- parsing ---- */

void imap_val_free(ImapVal *v)
{
    if (!v) return;
    g_free(v->s);
    if (v->items) g_ptr_array_free(v->items, TRUE);
    g_free(v);
}

static ImapVal *val(int kind)
{
    ImapVal *v = g_new0(ImapVal, 1);
    v->kind = kind;
    if (kind == IV_LIST) v->items = g_ptr_array_new_with_free_func((GDestroyNotify)imap_val_free);
    return v;
}

static void skip_sp(const char **p, const char *end)
{
    while (*p < end && **p == ' ') (*p)++;
}

/* One value at *p; NULL at the end of the line (or of the input). */
static ImapVal *parse_value(const char **pp, const char *end, int depth)
{
    const char *p = *pp;
    skip_sp(&p, end);
    if (p >= end || *p == '\r' || *p == '\n' || *p == ')') {
        *pp = p;
        return NULL;
    }
    ImapVal *v = NULL;
    if (*p == '(') {
        p++;
        v = val(IV_LIST);
        while (depth < MAX_LIST_DEPTH) {
            ImapVal *x = parse_value(&p, end, depth + 1);
            if (!x) break;
            g_ptr_array_add(v->items, x);
        }
        skip_sp(&p, end);
        if (p < end && *p == ')') p++;
    } else if (*p == '"') {
        GString *s = g_string_new(NULL);
        for (p++; p < end && *p != '"' && *p != '\r' && *p != '\n'; p++) {
            if (*p == '\\' && p + 1 < end) p++;
            g_string_append_c(s, *p);
        }
        if (p < end && *p == '"') p++;
        v = val(IV_STRING);
        v->n = s->len;
        v->s = g_string_free(s, FALSE);
    } else if (*p == '{') {
        const char *q = p + 1;
        guint64 n = 0;
        while (q < end && g_ascii_isdigit(*q) && n < (1ull << 40)) n = n * 10 + (guint64)(*q++ - '0');
        if (q < end && *q == '+') q++;
        if (q < end && *q == '}') q++;
        if (q < end && *q == '\r') q++;
        if (q < end && *q == '\n') q++;
        if (n > (guint64)(end - q)) n = (guint64)(end - q);  /* cut short: what is there */
        v = val(IV_STRING);
        v->s = g_malloc(n + 1);
        memcpy(v->s, q, n);
        v->s[n] = 0;
        v->n = n;
        p = q + n;
    } else {
        const char *s = p;
        int bracket = 0;
        while (p < end && *p != '\r' && *p != '\n') {
            if (*p == '[') bracket++;
            else if (*p == ']' && bracket) bracket--;
            else if (!bracket && (*p == ' ' || *p == '(' || *p == ')')) break;
            p++;
        }
        if (p == s) p++;  /* never stuck */
        v = val(IV_ATOM);
        v->s = g_strndup(s, (gsize)(p - s));
        v->n = (gsize)(p - s);
        if (!g_ascii_strcasecmp(v->s, "NIL")) v->kind = IV_NIL;
    }
    *pp = p;
    return v;
}

GPtrArray *imap_responses(const char *buf, gsize len)
{
    GPtrArray *out = g_ptr_array_new_with_free_func((GDestroyNotify)imap_val_free);
    const char *p = buf, *end = buf + len;
    while (p && p < end) {
        if (p + 1 < end && p[0] == '*' && p[1] == ' ') {
            p += 2;
            ImapVal *line = val(IV_LIST);
            for (int n = 0; n < 100000; n++) {
                ImapVal *x = parse_value(&p, end, 0);
                if (!x) {
                    if (p < end && *p == ')') {  /* a stray ")": skipped */
                        p++;
                        continue;
                    }
                    break;
                }
                g_ptr_array_add(line->items, x);
            }
            g_ptr_array_add(out, line);
        }
        const char *nl = p < end ? memchr(p, '\n', (size_t)(end - p)) : NULL;
        p = nl ? nl + 1 : NULL;
    }
    return out;
}

ImapVal *imap_fetch_item(ImapVal *list, const char *key)
{
    if (!list || list->kind != IV_LIST) return NULL;
    size_t kl = strcspn(key, "[");
    for (guint i = 0; i + 1 < list->items->len; i += 2) {
        ImapVal *k = list->items->pdata[i];
        if (k->kind == IV_ATOM && strlen(k->s) >= kl && !g_ascii_strncasecmp(k->s, key, kl) && (k->s[kl] == 0 || k->s[kl] == '['))
            return list->items->pdata[i + 1];
    }
    return NULL;
}

char *imap_utf7_decode(const char *s)
{
    GString *o = g_string_new(NULL);
    for (const char *p = s ? s : ""; *p; p++) {
        if (*p != '&') {
            g_string_append_c(o, *p);
            continue;
        }
        const char *e = strchr(p + 1, '-');
        if (!e) {
            g_string_append(o, p);
            break;
        }
        if (e == p + 1) {
            g_string_append_c(o, '&');  /* "&-" */
        } else {
            GString *bs = g_string_new_len(p + 1, (gssize)(e - p - 1));
            while (bs->len % 4) g_string_append_c(bs, '=');  /* modified UTF-7 leaves the padding out */
            char *b = g_string_free(bs, FALSE);
            for (char *c = b; *c; c++)
                if (*c == ',') *c = '/';
            gsize n = 0;
            gint state = 0;
            guint save = 0;
            guchar *raw = g_malloc(strlen(b) + 3);
            n = g_base64_decode_step(b, strlen(b), raw, &state, &save);
            char *u = g_convert((const char *)raw, (gssize)(n & ~(gsize)1), "UTF-8", "UTF-16BE", NULL, NULL, NULL);
            g_string_append(o, u ? u : "?");
            g_free(u);
            g_free(raw);
            g_free(b);
        }
        p = e;
    }
    char *out = g_utf8_make_valid(o->str, -1);
    g_string_free(o, TRUE);
    return out;
}

char *imap_quote(const char *s)
{
    GString *o = g_string_new("\"");
    for (const char *c = s ? s : ""; *c; c++) {
        if (*c == '\r' || *c == '\n' || *c == 0) {
            g_string_free(o, TRUE);
            return NULL;
        }
        if (*c == '"' || *c == '\\') g_string_append_c(o, '\\');
        g_string_append_c(o, *c);
    }
    g_string_append_c(o, '"');
    return g_string_free(o, FALSE);
}

/* ---- problems in plain words ---- */

const char *net_problem(int code, const char *host)
{
    static char buf[400];
    const char *h = host && *host ? host : "the server";
    switch (code) {
    case CURLE_LOGIN_DENIED:
        return "The server didn't accept the email address or password. Gmail, Outlook and Yahoo need an app password: "
               "make one in your account's security settings, and use it here.";
    case CURLE_COULDNT_RESOLVE_HOST:
        g_snprintf(buf, sizeof buf, "Mail can't find %s. Check the server's name, and that you're online.", h);
        break;
    case CURLE_COULDNT_CONNECT:
    case CURLE_OPERATION_TIMEDOUT:
        g_snprintf(buf, sizeof buf, "%s didn't answer. Check the server and port, or try again later.", h);
        break;
    case CURLE_PEER_FAILED_VERIFICATION:
    case CURLE_SSL_CONNECT_ERROR:
    case CURLE_SSL_CACERT_BADFILE:
        g_snprintf(buf, sizeof buf, "%s's security certificate couldn't be checked, so Mail didn't connect (your password "
                                    "wasn't sent).", h);
        break;
    case CURLE_USE_SSL_FAILED:
        g_snprintf(buf, sizeof buf, "%s doesn't offer an encrypted connection, so Mail didn't connect.", h);
        break;
    case CURLE_SEND_ERROR:
    case CURLE_RECV_ERROR:
    case CURLE_GOT_NOTHING:
        g_snprintf(buf, sizeof buf, "The connection to %s broke off. Try again.", h);
        break;
    case CURLE_REMOTE_ACCESS_DENIED:
        return "The server refused: you may not have access to that folder.";
    case CURLE_QUOTE_ERROR:
        return "The server refused that request.";
    case CURLE_UNSUPPORTED_PROTOCOL:
        return "A plain (unencrypted) connection is only allowed to this computer itself.";
    default:
        g_snprintf(buf, sizeof buf, "Something went wrong talking to %s (%s).", h, curl_easy_strerror((CURLcode)code));
    }
    return buf;
}

/* ---- the client ---- */

struct Imap {
    CURL *c;
    Server s;
    char *base;
    GString *buf;
    GString *lines;   /* every line the server sent (see run) */
    const char *up;   /* uploading */
    gsize up_left;
};

static GQuark quark(void) { return g_quark_from_static_string("mail-net"); }

static gboolean local_host(const char *h)
{
    return h && (!strcmp(h, "localhost") || !strcmp(h, "127.0.0.1") || !strcmp(h, "::1"));
}

static size_t on_write(char *d, size_t sz, size_t n, void *u)
{
    Imap *m = u;
    if (m->buf->len + sz * n > MAX_RESPONSE) return 0;
    g_string_append_len(m->buf, d, (gssize)(sz * n));
    return sz * n;
}

/* curl gives every line the server sends to the header callback, a
 * message's text inside FETCH replies included. */
static size_t on_line(char *d, size_t sz, size_t n, void *u)
{
    Imap *m = u;
    if (m->lines->len + sz * n > MAX_RESPONSE) return 0;
    g_string_append_len(m->lines, d, (gssize)(sz * n));
    return sz * n;
}

static size_t on_read(char *d, size_t sz, size_t n, void *u)
{
    Imap *m = u;
    size_t k = MIN(sz * n, m->up_left);
    memcpy(d, m->up, k);
    m->up += k;
    m->up_left -= k;
    return k;
}

static void common(CURL *c, const Server *s)
{
    curl_easy_setopt(c, CURLOPT_USERNAME, s->user);
    curl_easy_setopt(c, CURLOPT_PASSWORD, s->pass);
    curl_easy_setopt(c, CURLOPT_NOSIGNAL, 1L);
    curl_easy_setopt(c, CURLOPT_CONNECTTIMEOUT, 20L);
    curl_easy_setopt(c, CURLOPT_LOW_SPEED_LIMIT, 1L);
    curl_easy_setopt(c, CURLOPT_LOW_SPEED_TIME, 60L);
    curl_easy_setopt(c, CURLOPT_SSL_VERIFYPEER, 1L);
    curl_easy_setopt(c, CURLOPT_SSL_VERIFYHOST, 2L);
    if (s->security == SEC_STARTTLS) curl_easy_setopt(c, CURLOPT_USE_SSL, (long)CURLUSESSL_ALL);
    const char *ca = g_getenv("MAIL_CA_FILE");  /* tests: their own certificate authority */
    if (ca && *ca) curl_easy_setopt(c, CURLOPT_CAINFO, ca);
}

static char *host_part(const Server *s)
{
    return strchr(s->host, ':') ? g_strdup_printf("[%s]:%d", s->host, s->port) : g_strdup_printf("%s:%d", s->host, s->port);
}

Imap *imap_new(const Server *s)
{
    Imap *m = g_new0(Imap, 1);
    m->s.host = g_strdup(s->host);
    m->s.port = s->port;
    m->s.security = s->security;
    m->s.user = g_strdup(s->user);
    m->s.pass = g_strdup(s->pass);
    char *hp = host_part(s);
    m->base = g_strdup_printf("%s://%s", s->security == SEC_TLS ? "imaps" : "imap", hp);
    g_free(hp);
    m->buf = g_string_new(NULL);
    m->lines = g_string_new(NULL);
    m->c = curl_easy_init();
    common(m->c, s);
    curl_easy_setopt(m->c, CURLOPT_PROTOCOLS_STR, "imap,imaps");
    curl_easy_setopt(m->c, CURLOPT_WRITEFUNCTION, on_write);
    curl_easy_setopt(m->c, CURLOPT_WRITEDATA, m);
    curl_easy_setopt(m->c, CURLOPT_HEADERFUNCTION, on_line);
    curl_easy_setopt(m->c, CURLOPT_HEADERDATA, m);
    curl_easy_setopt(m->c, CURLOPT_READFUNCTION, on_read);
    curl_easy_setopt(m->c, CURLOPT_READDATA, m);
    return m;
}

void imap_free(Imap *m)
{
    if (!m) return;
    if (m->c) curl_easy_cleanup(m->c);
    g_free(m->s.host);
    g_free(m->s.user);
    if (m->s.pass) memset(m->s.pass, 0, strlen(m->s.pass));
    g_free(m->s.pass);
    g_free(m->base);
    g_string_free(m->buf, TRUE);
    g_string_free(m->lines, TRUE);
    g_free(m);
}

/* One request: in `folder` (or none), a command (or none: download). */
static gboolean run(Imap *m, const char *folder, const char *tail, const char *command, GError **err)
{
    if (m->s.security == SEC_NONE && !local_host(m->s.host)) {
        g_set_error_literal(err, quark(), CURLE_UNSUPPORTED_PROTOCOL, net_problem(CURLE_UNSUPPORTED_PROTOCOL, m->s.host));
        return FALSE;
    }
    char *url;
    if (folder) {
        char *e = curl_easy_escape(m->c, folder, 0);
        url = g_strdup_printf("%s/%s%s", m->base, e, tail ? tail : "");
        curl_free(e);
    } else {
        url = g_strdup_printf("%s/", m->base);
    }
    g_string_truncate(m->buf, 0);
    g_string_truncate(m->lines, 0);
    curl_easy_setopt(m->c, CURLOPT_URL, url);
    curl_easy_setopt(m->c, CURLOPT_CUSTOMREQUEST, command);
    curl_easy_setopt(m->c, CURLOPT_UPLOAD, m->up ? 1L : 0L);
    if (m->up) curl_easy_setopt(m->c, CURLOPT_INFILESIZE_LARGE, (curl_off_t)m->up_left);
    CURLcode r = curl_easy_perform(m->c);
    g_free(url);
    if (r != CURLE_OK) {
        g_set_error_literal(err, quark(), r, net_problem(r, m->s.host));
        return FALSE;
    }
    /* For a command, what it answered is in the server's lines: libcurl
     * passes only some answers on as the body (for UID FETCH, none). The
     * lines may also hold the greeting and SELECT's answers, which the
     * parsers here pass over (they look for FETCH, LIST, STATUS, SEARCH). */
    if (command) {
        GString *t = m->buf;
        m->buf = m->lines;
        m->lines = t;
    }
    return TRUE;
}

gboolean imap_check(Imap *m, GError **err) { return run(m, NULL, NULL, "NOOP", err); }

void folder_free(Folder *f)
{
    if (!f) return;
    g_free(f->raw);
    g_free(f->name);
    g_free(f);
}

void summary_free(Summary *s)
{
    if (s && s->headers) g_bytes_unref(s->headers);
    g_free(s);
}

static int role_of(ImapVal *flags, const char *name)
{
    for (guint i = 0; flags && flags->kind == IV_LIST && i < flags->items->len; i++) {
        const char *f = ((ImapVal *)flags->items->pdata[i])->s;
        if (!f) continue;
        if (!g_ascii_strcasecmp(f, "\\Sent")) return FOLDER_SENT;
        if (!g_ascii_strcasecmp(f, "\\Drafts")) return FOLDER_DRAFTS;
        if (!g_ascii_strcasecmp(f, "\\Trash")) return FOLDER_TRASH;
        if (!g_ascii_strcasecmp(f, "\\Junk")) return FOLDER_JUNK;
        if (!g_ascii_strcasecmp(f, "\\Archive")) return FOLDER_ARCHIVE;
    }
    if (!g_ascii_strcasecmp(name, "INBOX")) return FOLDER_INBOX;
    static const struct { const char *n; int r; } NAMES[] = {
        {"Sent", FOLDER_SENT}, {"Sent Items", FOLDER_SENT}, {"Sent Messages", FOLDER_SENT}, {"Sent Mail", FOLDER_SENT},
        {"Drafts", FOLDER_DRAFTS}, {"Trash", FOLDER_TRASH}, {"Deleted Items", FOLDER_TRASH}, {"Deleted Messages", FOLDER_TRASH},
        {"Bin", FOLDER_TRASH}, {"Junk", FOLDER_JUNK}, {"Spam", FOLDER_JUNK}, {"Junk E-mail", FOLDER_JUNK}, {"Archive", FOLDER_ARCHIVE},
        {NULL, 0}};
    const char *leaf = strrchr(name, '/');
    leaf = leaf ? leaf + 1 : name;
    for (int i = 0; NAMES[i].n; i++)
        if (!g_ascii_strcasecmp(leaf, NAMES[i].n)) return NAMES[i].r;
    return 0;
}

GPtrArray *imap_folders(Imap *m, GError **err)
{
    if (!run(m, NULL, NULL, "LIST \"\" \"*\"", err)) return NULL;
    GPtrArray *rs = imap_responses(m->buf->str, m->buf->len), *out = g_ptr_array_new_with_free_func((GDestroyNotify)folder_free);
    for (guint i = 0; i < rs->len; i++) {
        ImapVal *r = rs->pdata[i];
        if (r->items->len < 4 || g_ascii_strcasecmp(((ImapVal *)r->items->pdata[0])->s ? ((ImapVal *)r->items->pdata[0])->s : "", "LIST"))
            continue;
        ImapVal *flags = r->items->pdata[1], *delim = r->items->pdata[2], *name = r->items->pdata[3];
        if (!name->s || !*name->s || strchr(name->s, '\r') || strchr(name->s, '\n')) continue;
        Folder *f = g_new0(Folder, 1);
        f->raw = g_strdup(name->s);
        char *shown = imap_utf7_decode(name->s);
        if (delim->s && *delim->s && *delim->s != '/') {
            for (char *c = shown; *c; c++)
                if (*c == *delim->s) *c = '/';
        }
        f->name = shown;
        f->role = role_of(flags, f->name);
        for (guint k = 0; flags->kind == IV_LIST && k < flags->items->len; k++) {
            const char *x = ((ImapVal *)flags->items->pdata[k])->s;
            if (x && (!g_ascii_strcasecmp(x, "\\Noselect") || !g_ascii_strcasecmp(x, "\\NonExistent"))) f->noselect = TRUE;
        }
        g_ptr_array_add(out, f);
    }
    g_ptr_array_free(rs, TRUE);
    return out;
}

gboolean imap_status(Imap *m, const char *folder, guint32 *exists, guint32 *unseen, guint32 *uidnext, guint32 *uidvalidity, GError **err)
{
    char *q = imap_quote(folder);
    if (!q) {
        g_set_error_literal(err, quark(), CURLE_QUOTE_ERROR, "That folder's name can't be used.");
        return FALSE;
    }
    char *cmd = g_strdup_printf("STATUS %s (MESSAGES UNSEEN UIDNEXT UIDVALIDITY)", q);
    gboolean ok = run(m, NULL, NULL, cmd, err);
    g_free(cmd);
    g_free(q);
    if (!ok) return FALSE;
    GPtrArray *rs = imap_responses(m->buf->str, m->buf->len);
    for (guint i = 0; i < rs->len; i++) {
        ImapVal *r = rs->pdata[i];
        if (r->items->len < 3) continue;
        ImapVal *list = r->items->pdata[2];
        if (list->kind != IV_LIST) continue;
        guint32 *dst[4] = {exists, unseen, uidnext, uidvalidity};
        const char *keys[4] = {"MESSAGES", "UNSEEN", "UIDNEXT", "UIDVALIDITY"};
        for (int k = 0; k < 4; k++) {
            ImapVal *v = imap_fetch_item(list, keys[k]);
            if (dst[k] && v && v->s) *dst[k] = (guint32)g_ascii_strtoull(v->s, NULL, 10);
        }
    }
    g_ptr_array_free(rs, TRUE);
    return TRUE;
}

GPtrArray *imap_summaries(Imap *m, const char *folder, guint32 from, GError **err)
{
    char *cmd = g_strdup_printf("UID FETCH %u:* (UID FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID "
                                "IN-REPLY-TO REFERENCES REPLY-TO)])", MAX(1u, from));
    /* With the folder already open on this connection, the server tells it
     * about new mail only after a command: a NOOP first, or "n:*" misses it. */
    gboolean ok = run(m, folder, NULL, "NOOP", err) && run(m, folder, NULL, cmd, err);
    g_free(cmd);
    if (!ok) return NULL;
    GPtrArray *rs = imap_responses(m->buf->str, m->buf->len), *out = g_ptr_array_new_with_free_func((GDestroyNotify)summary_free);
    for (guint i = 0; i < rs->len; i++) {
        ImapVal *r = rs->pdata[i];
        if (r->items->len < 3 || !((ImapVal *)r->items->pdata[1])->s || g_ascii_strcasecmp(((ImapVal *)r->items->pdata[1])->s, "FETCH")) continue;
        ImapVal *list = r->items->pdata[2], *uid = imap_fetch_item(list, "UID"), *flags = imap_fetch_item(list, "FLAGS");
        ImapVal *size = imap_fetch_item(list, "RFC822.SIZE"), *h = imap_fetch_item(list, "BODY[");
        if (!uid || !uid->s) continue;
        Summary *s = g_new0(Summary, 1);
        s->uid = (guint32)g_ascii_strtoull(uid->s, NULL, 10);
        if (s->uid < from || !s->uid) {  /* "n:*" always answers with the last one, even below n */
            g_free(s);
            continue;
        }
        s->size = size && size->s ? g_ascii_strtoll(size->s, NULL, 10) : 0;
        for (guint k = 0; flags && flags->kind == IV_LIST && k < flags->items->len; k++) {
            const char *f = ((ImapVal *)flags->items->pdata[k])->s;
            if (!f) continue;
            s->seen |= !g_ascii_strcasecmp(f, "\\Seen");
            s->flagged |= !g_ascii_strcasecmp(f, "\\Flagged");
            s->answered |= !g_ascii_strcasecmp(f, "\\Answered");
        }
        s->headers = h && h->s ? g_bytes_new(h->s, h->n) : g_bytes_new("", 0);
        g_ptr_array_add(out, s);
    }
    g_ptr_array_free(rs, TRUE);
    return out;
}

GBytes *imap_fetch(Imap *m, const char *folder, guint32 uid, GError **err)
{
    char *tail = g_strdup_printf("/;UID=%u", uid);
    gboolean ok = run(m, folder, tail, NULL, err);
    g_free(tail);
    return ok ? g_bytes_new(m->buf->str, m->buf->len) : NULL;
}

gboolean imap_flag(Imap *m, const char *folder, guint32 uid, const char *flag, gboolean on, GError **err)
{
    if (strcmp(flag, "\\Seen") && strcmp(flag, "\\Flagged") && strcmp(flag, "\\Answered") && strcmp(flag, "\\Deleted")) return FALSE;
    char *cmd = g_strdup_printf("UID STORE %u %cFLAGS.SILENT (%s)", uid, on ? '+' : '-', flag);
    gboolean ok = run(m, folder, NULL, cmd, err);
    g_free(cmd);
    return ok;
}

gboolean imap_move(Imap *m, const char *folder, guint32 uid, const char *to, GError **err)
{
    char *q = imap_quote(to);
    if (!q) return FALSE;
    char *cmd = g_strdup_printf("UID MOVE %u %s", uid, q);
    gboolean ok = run(m, folder, NULL, cmd, NULL);
    g_free(cmd);
    if (!ok) {  /* no MOVE: copy, mark deleted, expunge */
        cmd = g_strdup_printf("UID COPY %u %s", uid, q);
        ok = run(m, folder, NULL, cmd, err) && imap_flag(m, folder, uid, "\\Deleted", TRUE, err) && run(m, folder, NULL, "EXPUNGE", err);
        g_free(cmd);
    }
    g_free(q);
    return ok;
}

gboolean imap_append(Imap *m, const char *folder, const char *msg, gsize len, GError **err)
{
    m->up = msg;
    m->up_left = len;
    gboolean ok = run(m, folder, NULL, NULL, err);
    m->up = NULL;
    m->up_left = 0;
    curl_easy_setopt(m->c, CURLOPT_UPLOAD, 0L);
    return ok;
}

GHashTable *imap_flags_all(Imap *m, const char *folder, GError **err)
{
    if (!run(m, folder, NULL, "UID FETCH 1:* (UID FLAGS)", err)) return NULL;
    GHashTable *h = g_hash_table_new(NULL, NULL);
    GPtrArray *rs = imap_responses(m->buf->str, m->buf->len);
    for (guint i = 0; i < rs->len; i++) {
        ImapVal *r = rs->pdata[i];
        if (r->items->len < 3) continue;
        ImapVal *list = r->items->pdata[2], *uid = imap_fetch_item(list, "UID"), *flags = imap_fetch_item(list, "FLAGS");
        if (!uid || !uid->s) continue;
        guint bits = 8;  /* (8: there, so never 0) */
        for (guint k = 0; flags && flags->kind == IV_LIST && k < flags->items->len; k++) {
            const char *f = ((ImapVal *)flags->items->pdata[k])->s;
            if (!f) continue;
            if (!g_ascii_strcasecmp(f, "\\Seen")) bits |= 1;
            if (!g_ascii_strcasecmp(f, "\\Flagged")) bits |= 2;
            if (!g_ascii_strcasecmp(f, "\\Answered")) bits |= 4;
        }
        g_hash_table_insert(h, GUINT_TO_POINTER((guint)g_ascii_strtoull(uid->s, NULL, 10)), GUINT_TO_POINTER(bits));
    }
    g_ptr_array_free(rs, TRUE);
    return h;
}

gboolean imap_delete_forever(Imap *m, const char *folder, guint32 uid, GError **err)
{
    if (!imap_flag(m, folder, uid, "\\Deleted", TRUE, err)) return FALSE;
    char *cmd = g_strdup_printf("UID EXPUNGE %u", uid);
    gboolean ok = run(m, folder, NULL, cmd, NULL) || run(m, folder, NULL, "EXPUNGE", err);
    g_free(cmd);
    return ok;
}

GArray *imap_search(Imap *m, const char *folder, const char *words, GError **err)
{
    char *q = imap_quote(words);
    if (!q) return g_array_new(FALSE, FALSE, sizeof(guint32));
    char *cmd = g_strdup_printf("UID SEARCH CHARSET UTF-8 TEXT %s", q);
    gboolean ok = run(m, folder, NULL, cmd, err);
    g_free(cmd);
    g_free(q);
    if (!ok) return NULL;
    GArray *out = g_array_new(FALSE, FALSE, sizeof(guint32));
    GPtrArray *rs = imap_responses(m->buf->str, m->buf->len);
    for (guint i = 0; i < rs->len; i++) {
        ImapVal *r = rs->pdata[i];
        if (!r->items->len || !((ImapVal *)r->items->pdata[0])->s || g_ascii_strcasecmp(((ImapVal *)r->items->pdata[0])->s, "SEARCH")) continue;
        for (guint k = 1; k < r->items->len; k++) {
            ImapVal *v = r->items->pdata[k];
            guint32 u = v->s ? (guint32)g_ascii_strtoull(v->s, NULL, 10) : 0;
            if (u) g_array_append_val(out, u);
        }
    }
    g_ptr_array_free(rs, TRUE);
    return out;
}

/* ---- sending ---- */

typedef struct {
    const char *p;
    gsize left;
} Up;

static size_t up_read(char *d, size_t sz, size_t n, void *u)
{
    Up *x = u;
    size_t k = MIN(sz * n, x->left);
    memcpy(d, x->p, k);
    x->p += k;
    x->left -= k;
    return k;
}

static size_t discard(char *d, size_t sz, size_t n, void *u)
{
    (void)d; (void)u;
    return sz * n;
}

gboolean smtp_send(const Server *s, const char *from, GPtrArray *rcpt, const char *msg, gsize len, GError **err)
{
    if (s->security == SEC_NONE && !local_host(s->host)) {
        g_set_error_literal(err, quark(), CURLE_UNSUPPORTED_PROTOCOL, net_problem(CURLE_UNSUPPORTED_PROTOCOL, s->host));
        return FALSE;
    }
    CURL *c = curl_easy_init();
    common(c, s);
    char *hp = host_part(s), *url = g_strdup_printf("%s://%s", s->security == SEC_TLS ? "smtps" : "smtp", hp);
    g_free(hp);
    char *mf = g_strdup_printf("<%s>", from);
    struct curl_slist *to = NULL;
    for (guint i = 0; i < rcpt->len; i++) {
        char *r = g_strdup_printf("<%s>", (char *)rcpt->pdata[i]);
        to = curl_slist_append(to, r);
        g_free(r);
    }
    Up up = {msg, len};
    curl_easy_setopt(c, CURLOPT_PROTOCOLS_STR, "smtp,smtps");
    curl_easy_setopt(c, CURLOPT_URL, url);
    curl_easy_setopt(c, CURLOPT_MAIL_FROM, mf);
    curl_easy_setopt(c, CURLOPT_MAIL_RCPT, to);
    curl_easy_setopt(c, CURLOPT_READFUNCTION, up_read);
    curl_easy_setopt(c, CURLOPT_READDATA, &up);
    curl_easy_setopt(c, CURLOPT_UPLOAD, 1L);
    curl_easy_setopt(c, CURLOPT_WRITEFUNCTION, discard);
    if (!rcpt->len) curl_easy_setopt(c, CURLOPT_CONNECT_ONLY, 1L);  /* only checking the sign-in */
    CURLcode r = curl_easy_perform(c);
    if (r != CURLE_OK) {
        long resp = 0;
        curl_easy_getinfo(c, CURLINFO_RESPONSE_CODE, &resp);
        if (resp == 550 || resp == 553 || resp == 501)
            g_set_error_literal(err, quark(), r, "The server refused one of the addresses. Check them, and try again.");
        else if (resp == 552)
            g_set_error_literal(err, quark(), r, "The server says the message is too big. Try smaller attachments.");
        else
            g_set_error_literal(err, quark(), r, net_problem(r, s->host));
    }
    curl_slist_free_all(to);
    g_free(mf);
    g_free(url);
    curl_easy_cleanup(c);
    return r == CURLE_OK;
}
