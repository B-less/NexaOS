/* Accounts, their passwords and the offline copy of the mail.
 *
 *   ~/.config/mail/accounts.ini      servers and names (no password)
 *   ~/.local/share/mail/secrets      passwords, encrypted (ChaCha20-Poly1305)
 *   ~/.local/share/mail/key          the key for them (0600)
 *   ~/.cache/mail/<account>/...      the messages already fetched
 *
 * The encryption keeps passwords out of plain sight (a copied settings
 * folder, a backup); someone with your login could still read the key. */
#ifndef MAIL_ACCOUNT_H
#define MAIL_ACCOUNT_H

#include "imap.h"

typedef struct {
    char *id;           /* "a1" */
    char *name, *address;
    Server in, out;     /* IMAP, SMTP (their pass filled from the secrets) */
} Account;

GPtrArray *accounts_load(void);   /* Account* */
gboolean accounts_save(GPtrArray *list, GError **err);
void account_free(Account *a);
char *account_new_id(GPtrArray *list);

/* The usual servers for an address ("ama@gmail.com" -> imap.gmail.com 993 ...). */
void account_guess(const char *address, Server *in, Server *out);

/* Passwords, encrypted. */
gboolean secret_set(const char *id, const char *password, GError **err);
char *secret_get(const char *id);
void secret_forget(const char *id);

/* Encrypt/decrypt with a 32-byte key (tested on its own). */
char *seal(const guint8 key[32], const char *id, const char *plain);
char *unseal(const guint8 key[32], const char *id, const char *sealed);

/* ---- the offline copy ---- */

typedef struct {
    guint32 uid;
    gboolean seen, flagged, answered;
    gint64 date, size;
    char *from, *subject, *message_id, *preview;
} Entry;

typedef struct {
    char *dir;          /* this folder's cache */
    guint32 uidvalidity, uidnext;
    GPtrArray *entries; /* Entry*, newest first */
} Box;

Box *box_load(const char *account, const char *folder);
gboolean box_save(Box *b);
void box_free(Box *b);
void entry_free(Entry *e);
Entry *box_find(Box *b, guint32 uid);
void box_remove(Box *b, guint32 uid);
/* A message's whole text, once fetched (NULL if not here yet). */
GBytes *box_body(Box *b, guint32 uid);
void box_put_body(Box *b, guint32 uid, GBytes *raw);
#endif
