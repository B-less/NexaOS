#include "account.h"

#include <fcntl.h>
#include <gio/gio.h>
#include <glib/gstdio.h>
#include <unistd.h>
#include <openssl/evp.h>
#include <openssl/rand.h>
#include <string.h>

#include "mime.h"

#define APP "mail"

static char *conf_dir(void) { return g_build_filename(g_get_user_config_dir(), APP, NULL); }
static char *data_dir(void) { return g_build_filename(g_get_user_data_dir(), APP, NULL); }
static char *cache_dir(void) { return g_build_filename(g_get_user_cache_dir(), APP, NULL); }

void account_free(Account *a)
{
    if (!a) return;
    g_free(a->id);
    g_free(a->name);
    g_free(a->address);
    g_free(a->in.host);
    g_free(a->in.user);
    g_free(a->out.host);
    g_free(a->out.user);
    if (a->in.pass) memset(a->in.pass, 0, strlen(a->in.pass));
    g_free(a->in.pass);
    g_free(a->out.pass);
    g_free(a);
}

static int sec_of(const char *s)
{
    return !g_strcmp0(s, "starttls") ? SEC_STARTTLS : !g_strcmp0(s, "none") ? SEC_NONE : SEC_TLS;
}

static const char *sec_name(int s) { return s == SEC_STARTTLS ? "starttls" : s == SEC_NONE ? "none" : "tls"; }

static gboolean host_ok(const char *h)
{
    if (!h || !*h || strlen(h) > 253) return FALSE;
    for (const char *c = h; *c; c++)
        if (!(g_ascii_isalnum(*c) || *c == '.' || *c == '-' || *c == ':' || *c == '_')) return FALSE;
    return TRUE;
}

GPtrArray *accounts_load(void)
{
    GPtrArray *out = g_ptr_array_new_with_free_func((GDestroyNotify)account_free);
    char *dir = conf_dir(), *path = g_build_filename(dir, "accounts.ini", NULL);
    GKeyFile *k = g_key_file_new();
    if (g_key_file_load_from_file(k, path, G_KEY_FILE_NONE, NULL)) {
        gsize n = 0;
        char **groups = g_key_file_get_groups(k, &n);
        for (gsize i = 0; i < n; i++) {
            const char *g = groups[i];
            if (!g_str_has_prefix(g, "account ")) continue;
            Account *a = g_new0(Account, 1);
            a->id = g_strdup(g + 8);
            a->name = g_key_file_get_string(k, g, "name", NULL);
            a->address = g_key_file_get_string(k, g, "address", NULL);
            a->in.host = g_key_file_get_string(k, g, "imap_host", NULL);
            a->in.port = g_key_file_get_integer(k, g, "imap_port", NULL);
            char *s = g_key_file_get_string(k, g, "imap_security", NULL);
            a->in.security = sec_of(s);
            g_free(s);
            a->in.user = g_key_file_get_string(k, g, "user", NULL);
            a->out.host = g_key_file_get_string(k, g, "smtp_host", NULL);
            a->out.port = g_key_file_get_integer(k, g, "smtp_port", NULL);
            s = g_key_file_get_string(k, g, "smtp_security", NULL);
            a->out.security = sec_of(s);
            g_free(s);
            a->out.user = g_strdup(a->in.user);
            gboolean ok = a->address && address_valid(a->address) && host_ok(a->in.host) && host_ok(a->out.host) && a->in.port > 0 &&
                          a->in.port < 65536 && a->out.port > 0 && a->out.port < 65536 && a->in.user;
            for (const char *c = a->id; ok && *c; c++) ok = g_ascii_isalnum(*c);
            if (!ok) {
                account_free(a);
                continue;
            }
            if (!a->name) a->name = g_strdup("");
            a->in.pass = secret_get(a->id);
            a->out.pass = g_strdup(a->in.pass);
            g_ptr_array_add(out, a);
        }
        g_strfreev(groups);
    }
    g_key_file_free(k);
    g_free(path);
    g_free(dir);
    return out;
}

gboolean accounts_save(GPtrArray *list, GError **err)
{
    GKeyFile *k = g_key_file_new();
    for (guint i = 0; i < list->len; i++) {
        Account *a = list->pdata[i];
        char *g = g_strdup_printf("account %s", a->id);
        g_key_file_set_string(k, g, "name", a->name ? a->name : "");
        g_key_file_set_string(k, g, "address", a->address);
        g_key_file_set_string(k, g, "user", a->in.user);
        g_key_file_set_string(k, g, "imap_host", a->in.host);
        g_key_file_set_integer(k, g, "imap_port", a->in.port);
        g_key_file_set_string(k, g, "imap_security", sec_name(a->in.security));
        g_key_file_set_string(k, g, "smtp_host", a->out.host);
        g_key_file_set_integer(k, g, "smtp_port", a->out.port);
        g_key_file_set_string(k, g, "smtp_security", sec_name(a->out.security));
        g_free(g);
    }
    char *dir = conf_dir(), *path = g_build_filename(dir, "accounts.ini", NULL);
    g_mkdir_with_parents(dir, 0700);
    gboolean ok = g_key_file_save_to_file(k, path, err);
    g_chmod(path, 0600);
    g_key_file_free(k);
    g_free(path);
    g_free(dir);
    return ok;
}

char *account_new_id(GPtrArray *list)
{
    for (int n = 1;; n++) {
        char *id = g_strdup_printf("a%d", n);
        gboolean used = FALSE;
        for (guint i = 0; i < list->len; i++) used |= !strcmp(((Account *)list->pdata[i])->id, id);
        if (!used) return id;
        g_free(id);
    }
}

void account_guess(const char *address, Server *in, Server *out)
{
    static const struct { const char *domains, *imap, *smtp; int sport, ssec; } P[] = {
        {"gmail.com googlemail.com", "imap.gmail.com", "smtp.gmail.com", 465, SEC_TLS},
        {"outlook.com hotmail.com live.com msn.com outlook.fr hotmail.co.uk", "outlook.office365.com", "smtp.office365.com", 587, SEC_STARTTLS},
        {"yahoo.com yahoo.co.uk ymail.com", "imap.mail.yahoo.com", "smtp.mail.yahoo.com", 465, SEC_TLS},
        {"icloud.com me.com mac.com", "imap.mail.me.com", "smtp.mail.me.com", 587, SEC_STARTTLS},
        {"zoho.com zohomail.com", "imap.zoho.com", "smtp.zoho.com", 465, SEC_TLS},
        {"gmx.com gmx.net gmx.de", "imap.gmx.com", "mail.gmx.com", 587, SEC_STARTTLS},
        {"aol.com", "imap.aol.com", "smtp.aol.com", 465, SEC_TLS},
        {NULL, NULL, NULL, 0, 0},
    };
    const char *at = address ? strrchr(address, '@') : NULL;
    char *dom = g_ascii_strdown(at ? at + 1 : "example.com", -1);
    g_free(in->host);
    g_free(out->host);
    in->host = out->host = NULL;
    for (int i = 0; P[i].domains && !in->host; i++) {
        char **d = g_strsplit(P[i].domains, " ", -1);
        for (int k = 0; d[k]; k++)
            if (!strcmp(d[k], dom)) {
                in->host = g_strdup(P[i].imap);
                out->host = g_strdup(P[i].smtp);
                out->port = P[i].sport;
                out->security = P[i].ssec;
            }
        g_strfreev(d);
    }
    if (!in->host) {
        in->host = g_strdup_printf("imap.%s", dom);
        out->host = g_strdup_printf("smtp.%s", dom);
        out->port = 587;
        out->security = SEC_STARTTLS;
    }
    in->port = 993;
    in->security = SEC_TLS;
    g_free(dom);
}

/* ---- passwords ---- */

char *seal(const guint8 key[32], const char *id, const char *plain)
{
    guint8 nonce[12];
    if (RAND_bytes(nonce, sizeof nonce) != 1) return NULL;
    size_t n = strlen(plain);
    guint8 *out = g_malloc(12 + n + 16);
    memcpy(out, nonce, 12);
    EVP_CIPHER_CTX *c = EVP_CIPHER_CTX_new();
    int len = 0, ok = c && EVP_EncryptInit_ex(c, EVP_chacha20_poly1305(), NULL, key, nonce) == 1;
    char *aad = g_strdup_printf("mail-v1:%s", id);
    ok = ok && EVP_EncryptUpdate(c, NULL, &len, (const guint8 *)aad, (int)strlen(aad)) == 1;
    ok = ok && EVP_EncryptUpdate(c, out + 12, &len, (const guint8 *)plain, (int)n) == 1;
    ok = ok && EVP_EncryptFinal_ex(c, out + 12 + len, &len) == 1;
    ok = ok && EVP_CIPHER_CTX_ctrl(c, EVP_CTRL_AEAD_GET_TAG, 16, out + 12 + n) == 1;
    EVP_CIPHER_CTX_free(c);
    g_free(aad);
    char *b = ok ? g_base64_encode(out, 12 + n + 16) : NULL;
    memset(out, 0, 12 + n + 16);
    g_free(out);
    return b;
}

char *unseal(const guint8 key[32], const char *id, const char *sealed)
{
    gsize n = 0;
    guint8 *raw = sealed ? g_base64_decode(sealed, &n) : NULL;
    if (!raw || n < 28) {
        g_free(raw);
        return NULL;
    }
    size_t pn = n - 28;
    char *plain = g_malloc0(pn + 1);
    EVP_CIPHER_CTX *c = EVP_CIPHER_CTX_new();
    int len = 0, ok = c && EVP_DecryptInit_ex(c, EVP_chacha20_poly1305(), NULL, key, raw) == 1;
    char *aad = g_strdup_printf("mail-v1:%s", id);
    ok = ok && EVP_DecryptUpdate(c, NULL, &len, (const guint8 *)aad, (int)strlen(aad)) == 1;
    ok = ok && EVP_DecryptUpdate(c, (guint8 *)plain, &len, raw + 12, (int)pn) == 1;
    ok = ok && EVP_CIPHER_CTX_ctrl(c, EVP_CTRL_AEAD_SET_TAG, 16, raw + 12 + pn) == 1;
    ok = ok && EVP_DecryptFinal_ex(c, (guint8 *)plain + len, &len) == 1;  /* the tag: tampered or the wrong key fails here */
    EVP_CIPHER_CTX_free(c);
    g_free(aad);
    g_free(raw);
    if (!ok || strlen(plain) != pn) {
        memset(plain, 0, pn);
        g_free(plain);
        return NULL;
    }
    return plain;
}

static gboolean key(guint8 out[32])
{
    char *dir = data_dir(), *path = g_build_filename(dir, "key", NULL);
    gchar *raw = NULL;
    gsize n = 0;
    gboolean ok = g_file_get_contents(path, &raw, &n, NULL) && n == 32;
    if (ok) {
        memcpy(out, raw, 32);
    } else if (RAND_bytes(out, 32) == 1) {
        g_mkdir_with_parents(dir, 0700);
        int fd = g_open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
        ok = fd >= 0 && write(fd, out, 32) == 32;
        if (fd >= 0) close(fd);
    }
    if (raw) memset(raw, 0, n);
    g_free(raw);
    g_free(path);
    g_free(dir);
    return ok;
}

static char *secrets_path(void)
{
    char *dir = data_dir(), *p = g_build_filename(dir, "secrets", NULL);
    g_free(dir);
    return p;
}

gboolean secret_set(const char *id, const char *password, GError **err)
{
    guint8 k[32];
    if (!key(k)) {
        g_set_error_literal(err, G_IO_ERROR, G_IO_ERROR_FAILED, "Mail couldn't make a key to keep the password safe.");
        return FALSE;
    }
    char *s = seal(k, id, password);
    memset(k, 0, sizeof k);
    GKeyFile *f = g_key_file_new();
    char *path = secrets_path();
    g_key_file_load_from_file(f, path, G_KEY_FILE_NONE, NULL);
    g_key_file_set_string(f, "secrets", id, s ? s : "");
    gboolean ok = s && g_key_file_save_to_file(f, path, err);
    g_chmod(path, 0600);
    g_key_file_free(f);
    g_free(path);
    g_free(s);
    return ok;
}

char *secret_get(const char *id)
{
    guint8 k[32];
    char *path = secrets_path();
    GKeyFile *f = g_key_file_new();
    char *out = NULL;
    if (g_key_file_load_from_file(f, path, G_KEY_FILE_NONE, NULL)) {
        char *s = g_key_file_get_string(f, "secrets", id, NULL);
        if (s && key(k)) out = unseal(k, id, s);
        g_free(s);
    }
    memset(k, 0, sizeof k);
    g_key_file_free(f);
    g_free(path);
    return out;
}

void secret_forget(const char *id)
{
    char *path = secrets_path();
    GKeyFile *f = g_key_file_new();
    if (g_key_file_load_from_file(f, path, G_KEY_FILE_NONE, NULL)) {
        g_key_file_remove_key(f, "secrets", id, NULL);
        g_key_file_save_to_file(f, path, NULL);
    }
    g_key_file_free(f);
    g_free(path);
}

/* ---- the offline copy ---- */

void entry_free(Entry *e)
{
    if (!e) return;
    g_free(e->from);
    g_free(e->subject);
    g_free(e->message_id);
    g_free(e->preview);
    g_free(e);
}

void box_free(Box *b)
{
    if (!b) return;
    g_free(b->dir);
    g_ptr_array_free(b->entries, TRUE);
    g_free(b);
}

static gint newest(gconstpointer x, gconstpointer y)
{
    const Entry *a = *(Entry *const *)x, *b = *(Entry *const *)y;
    if (a->date != b->date) return a->date > b->date ? -1 : 1;
    return a->uid > b->uid ? -1 : a->uid < b->uid;
}

Box *box_load(const char *account, const char *folder)
{
    Box *b = g_new0(Box, 1);
    char *sum = g_compute_checksum_for_string(G_CHECKSUM_SHA1, folder, -1), *base = cache_dir();
    b->dir = g_build_filename(base, account, sum, NULL);
    b->entries = g_ptr_array_new_with_free_func((GDestroyNotify)entry_free);
    g_free(sum);
    g_free(base);
    char *path = g_build_filename(b->dir, "index", NULL), *text = NULL;
    if (g_file_get_contents(path, &text, NULL, NULL)) {
        char **lines = g_strsplit(text, "\n", -1);
        if (lines[0] && g_str_has_prefix(lines[0], "mailbox ")) sscanf(lines[0] + 8, "%u %u", &b->uidvalidity, &b->uidnext);
        for (int i = 1; lines[i]; i++) {
            char **f = g_strsplit(lines[i], "\t", -1);
            if (g_strv_length(f) == 9) {
                Entry *e = g_new0(Entry, 1);
                e->uid = (guint32)g_ascii_strtoull(f[0], NULL, 10);
                e->seen = strchr(f[1], 's') != NULL;
                e->flagged = strchr(f[1], 'f') != NULL;
                e->answered = strchr(f[1], 'a') != NULL;
                e->date = g_ascii_strtoll(f[2], NULL, 10);
                e->size = g_ascii_strtoll(f[3], NULL, 10);
                e->from = g_strcompress(f[4]);
                e->subject = g_strcompress(f[5]);
                e->message_id = g_strcompress(f[6]);
                e->preview = g_strcompress(f[7]);
                if (e->uid) g_ptr_array_add(b->entries, e);
                else entry_free(e);
            }
            g_strfreev(f);
        }
        g_strfreev(lines);
        g_free(text);
    }
    g_free(path);
    g_ptr_array_sort(b->entries, newest);
    return b;
}

gboolean box_save(Box *b)
{
    g_ptr_array_sort(b->entries, newest);
    GString *s = g_string_new(NULL);
    g_string_append_printf(s, "mailbox %u %u\n", b->uidvalidity, b->uidnext);
    for (guint i = 0; i < b->entries->len; i++) {
        Entry *e = b->entries->pdata[i];
        char *f = g_strescape(e->from ? e->from : "", NULL), *sj = g_strescape(e->subject ? e->subject : "", NULL);
        char *id = g_strescape(e->message_id ? e->message_id : "", NULL), *pv = g_strescape(e->preview ? e->preview : "", NULL);
        g_string_append_printf(s, "%u\t%s%s%s-\t%" G_GINT64_FORMAT "\t%" G_GINT64_FORMAT "\t%s\t%s\t%s\t%s\t\n", e->uid, e->seen ? "s" : "",
                               e->flagged ? "f" : "", e->answered ? "a" : "", e->date, e->size, f, sj, id, pv);
        g_free(f); g_free(sj); g_free(id); g_free(pv);
    }
    g_mkdir_with_parents(b->dir, 0700);
    char *path = g_build_filename(b->dir, "index", NULL);
    gboolean ok = g_file_set_contents(path, s->str, (gssize)s->len, NULL);
    g_free(path);
    g_string_free(s, TRUE);
    return ok;
}

Entry *box_find(Box *b, guint32 uid)
{
    for (guint i = 0; i < b->entries->len; i++)
        if (((Entry *)b->entries->pdata[i])->uid == uid) return b->entries->pdata[i];
    return NULL;
}

void box_remove(Box *b, guint32 uid)
{
    Entry *e = box_find(b, uid);
    if (e) g_ptr_array_remove(b->entries, e);
    char name[32];
    g_snprintf(name, sizeof name, "%u.eml", uid);
    char *p = g_build_filename(b->dir, name, NULL);
    g_unlink(p);
    g_free(p);
}

GBytes *box_body(Box *b, guint32 uid)
{
    char name[32];
    g_snprintf(name, sizeof name, "%u.eml", uid);
    char *p = g_build_filename(b->dir, name, NULL), *data = NULL;
    gsize n = 0;
    GBytes *out = g_file_get_contents(p, &data, &n, NULL) ? g_bytes_new_take(data, n) : NULL;
    g_free(p);
    return out;
}

void box_put_body(Box *b, guint32 uid, GBytes *raw)
{
    char name[32];
    g_snprintf(name, sizeof name, "%u.eml", uid);
    g_mkdir_with_parents(b->dir, 0700);
    char *p = g_build_filename(b->dir, name, NULL);
    gsize n;
    const char *d = g_bytes_get_data(raw, &n);
    g_file_set_contents(p, d, (gssize)n, NULL);
    g_chmod(p, 0600);
    g_free(p);
}
