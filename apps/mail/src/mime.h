/* Mail's message reader and writer (no GTK, no network; tests/unit.c).
 *
 * Reading: headers (folded lines, =?charset?B/Q?...?= words in any
 * charset), multipart parts to any depth (10 at most), base64 and
 * quoted-printable, the text to show (text/plain, or HTML turned into
 * plain text: nothing from an email is ever run or fetched, so no
 * tracking pictures), and its attachments.
 *
 * Writing: UTF-8 text as quoted-printable, headers encoded where needed,
 * attachments as base64, replies with In-Reply-To and References. */
#ifndef MAIL_MIME_H
#define MAIL_MIME_H

#include <glib.h>

typedef struct {
    char *name;      /* for people; "" if none */
    char *addr;      /* user@example.com */
} Address;

typedef struct {
    char *filename;  /* safe to save as: no folders, no control characters */
    char *type;      /* "application/pdf" */
    GBytes *data;    /* decoded */
} Attachment;

typedef struct {
    char *subject, *message_id, *in_reply_to, *references;
    GPtrArray *from, *to, *cc, *reply_to;  /* Address* */
    gint64 date;                           /* unix time; 0 if unknown */
    char *text;                            /* what to show, UTF-8 */
    gboolean was_html;
    GPtrArray *attachments;                /* Attachment* */
} Message;

/* Every byte of a message: never NULL (a broken one gives what could be read). */
Message *mime_parse(const char *raw, gsize len);
void message_free(Message *m);

/* Headers only (for the list): subject, from, date, message id. */
Message *mime_parse_headers(const char *raw, gsize len);

/* Pieces, tested on their own. */
char *mime_decode_words(const char *s);              /* RFC 2047 -> UTF-8 */
GPtrArray *mime_addresses(const char *s);            /* "A <a@x>, b@y" -> Address* */
gint64 mime_date(const char *s);                     /* RFC 5322 date -> unix time, 0 if bad */
char *mime_html_to_text(const char *html);
GBytes *mime_decode_qp(const char *s, gsize n);
char *mime_safe_filename(const char *name);          /* "../../x.sh" -> "x.sh" */
void address_free(Address *a);
char *address_format(const Address *a);              /* "Ama <ama@x.com>" */
gboolean address_valid(const char *addr);

/* ---- writing ---- */

typedef struct {
    const char *from_name, *from_addr;
    GPtrArray *to, *cc;            /* Address* */
    const char *subject, *body;    /* UTF-8 */
    GPtrArray *attachments;        /* Attachment*, may be NULL */
    const char *in_reply_to, *references;
    gint64 date;                   /* 0: now */
} Draft;

/* The message to send (CRLF lines), and its new Message-ID. */
char *mime_build(const Draft *d, char **message_id, gsize *len);
/* "Re: x" / "Fwd: x", not twice. */
char *mime_reply_subject(const char *s, gboolean forward);
/* The quoted text under a reply: "On ..., Ama wrote:" and "> " lines. */
char *mime_quote(const Message *m);

/* ---- links from elsewhere ---- */

/* A mailto: link (RFC 6068), as the Browser or a PDF hands it over: the
 * addresses (to, cc: only valid ones, comma-separated), subject (one line)
 * and body. Other fields (bcc, headers) are ignored: a link never adds a
 * hidden recipient. FALSE if it isn't a mailto: link. */
gboolean mailto_parse(const char *uri, char **to, char **cc, char **subject, char **body);
/* "Write to Kofi": the one address among `senders` (From lines, "Name
 * <addr>") whose name or address holds every word of `who`, ignoring case,
 * formatted; NULL if none, or if two different people match. */
char *address_pick(GPtrArray *senders, const char *who);
#endif
