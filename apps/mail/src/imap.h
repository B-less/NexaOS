/* IMAP for Mail: a parser for what servers answer (tested on its own,
 * with damaged input), and a client over libcurl (TLS, or STARTTLS; a
 * plain connection only to this computer). Folder names, quoted strings
 * and search words are always quoted, so nothing in them can become a
 * command. */
#ifndef MAIL_IMAP_H
#define MAIL_IMAP_H

#include <glib.h>

/* ---- parsing ---- */

enum { IV_ATOM, IV_STRING, IV_LIST, IV_NIL };

typedef struct ImapVal {
    int kind;
    char *s;          /* ATOM, STRING (may hold NUL bytes: see n) */
    gsize n;
    GPtrArray *items; /* LIST: ImapVal* */
} ImapVal;

/* The untagged responses ("* ...") in a block of server output: each a
 * LIST ImapVal of its words. Damaged input gives what could be read. */
GPtrArray *imap_responses(const char *buf, gsize len);
void imap_val_free(ImapVal *v);
/* In a FETCH response's list, the value after `key` (e.g. "UID", "FLAGS",
 * "BODY[HEADER.FIELDS (FROM)]"), matched by prefix up to '['. */
ImapVal *imap_fetch_item(ImapVal *list, const char *key);
/* Modified UTF-7 folder names ("Entw&APw-rfe" -> "Entwürfe"). */
char *imap_utf7_decode(const char *s);
/* A string as an IMAP quoted string; NULL if it can't be one (a line break). */
char *imap_quote(const char *s);

/* ---- the client ---- */

enum { SEC_TLS, SEC_STARTTLS, SEC_NONE };

typedef struct {
    char *host;
    int port, security;
    char *user, *pass;
} Server;

enum { FOLDER_INBOX = 1, FOLDER_SENT, FOLDER_DRAFTS, FOLDER_TRASH, FOLDER_JUNK, FOLDER_ARCHIVE };

typedef struct {
    char *raw;    /* as the server names it (for commands) */
    char *name;   /* for people */
    int role;     /* FOLDER_* or 0 */
    gboolean noselect;
} Folder;

typedef struct {
    guint32 uid;
    gboolean seen, flagged, answered;
    gint64 size;
    GBytes *headers;  /* the header lines asked for */
} Summary;

typedef struct Imap Imap;

Imap *imap_new(const Server *s);
void imap_free(Imap *m);
gboolean imap_check(Imap *m, GError **err);   /* signs in */
GPtrArray *imap_folders(Imap *m, GError **err);  /* Folder* */
gboolean imap_status(Imap *m, const char *folder, guint32 *exists, guint32 *unseen, guint32 *uidnext, guint32 *uidvalidity, GError **err);
/* Summaries of messages with UIDs in "from:*" (Summary*). */
GPtrArray *imap_summaries(Imap *m, const char *folder, guint32 from, GError **err);
GBytes *imap_fetch(Imap *m, const char *folder, guint32 uid, GError **err);
gboolean imap_flag(Imap *m, const char *folder, guint32 uid, const char *flag, gboolean on, GError **err);
gboolean imap_move(Imap *m, const char *folder, guint32 uid, const char *to, GError **err);
gboolean imap_append(Imap *m, const char *folder, const char *msg, gsize len, GError **err);
/* Every message's UID and flags (bit 1 seen, 2 flagged, 4 answered): to
 * catch what changed elsewhere, and what went. */
GHashTable *imap_flags_all(Imap *m, const char *folder, GError **err);
/* In Trash: gone for good. */
gboolean imap_delete_forever(Imap *m, const char *folder, guint32 uid, GError **err);
GArray *imap_search(Imap *m, const char *folder, const char *words, GError **err);  /* guint32 UIDs */
void folder_free(Folder *f);
void summary_free(Summary *s);

/* ---- sending ---- */
gboolean smtp_send(const Server *s, const char *from, GPtrArray *rcpt, const char *msg, gsize len, GError **err);

/* Plain words for what went wrong (shown to people). */
const char *net_problem(int curl_code, const char *host);
#endif
