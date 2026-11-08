/* Mail: a NexaOS app in C (GTK 3): your email over IMAP and SMTP.
 *
 *   mail                    the window
 *   mail mailto:LINK        only a new message, filled in from the link
 *                           (the Browser, a PDF: Mail is the email app)
 *   mail --write NAME       only a new message to NAME, whose address is
 *                           found in mail they sent (Nexa Mind)
 *   mail --search WORDS     the window, searching every message for WORDS
 *
 * Folders on the left, the messages in the middle, the one you read on
 * the right. Write, Reply, Reply All, Forward (with attachments), flag,
 * delete (to Trash; in Trash, for good), search. Mail read once stays
 * readable offline. Emails are shown as text: nothing in them is run or
 * fetched (no tracking pictures). Connections are TLS or STARTTLS only
 * (a plain one only to this computer). The password is kept encrypted.
 *
 * Installed, it runs confined by nexa-run to network, files.documents
 * (attachments are saved to Documents/Mail attachments) and files.pictures
 * (to attach pictures).
 *
 * New mail in the Inbox becomes a Signal (org.freedesktop.Notifications)
 * while Mail is open and you're elsewhere; clicking it opens the message.
 * Quiet and Layer moods apply, as Signals decides. The window's title
 * carries the unread count, so Orbit and Constellation show it too.
 *
 * Keys: Ctrl+N write, Ctrl+R reply, Ctrl+Shift+R reply all, Ctrl+Shift+F
 * forward, Delete, F5 check now, Ctrl+F search, Ctrl+Enter send.
 *
 * MAIL_TEST=<dir>: for tests only, writes <dir>/state.json and takes
 * commands from the pipe <dir>/cmd (tests/headless.py). */
#include <curl/curl.h>
#include <fcntl.h>
#include <glib/gstdio.h>
#include <gtk/gtk.h>
#include <math.h>
#include <string.h>
#include <unistd.h>

#include "account.h"
#include "mime.h"

#define APP_ID "mail"
#define CHECK_EVERY 180

/* ---- the app ---- */

typedef struct Compose Compose;

typedef struct {
    GtkWidget *win, *stack, *search, *write_btn, *refresh_btn;
    GPtrArray *accounts;   /* Account* */
    Account *acc;          /* the one shown */
    GPtrArray *folders;    /* Folder* of acc */
    GHashTable *unseen;    /* raw folder name -> count */
    char *folder;          /* raw name */
    Box *box;
    GHashTable *shown;     /* UIDs the server's search found here, or NULL */
    char *shown_q;         /* the words it searched for */
    Entry *open;           /* the one being read */
    Message *msg;
    GtkWidget *folder_list, *msg_list, *list_empty, *list_head, *status;
    GtkWidget *read_box, *read_scroll, *read_empty, *read_subject, *read_from, *read_to, *read_date, *read_text, *read_atts, *flag_btn, *html_note;
    /* setup */
    GtkWidget *su_name, *su_addr, *su_pass, *su_imap, *su_imap_port, *su_imap_sec, *su_smtp, *su_smtp_port, *su_smtp_sec, *su_user, *su_msg, *su_btn;
    gboolean su_touched;
    /* work */
    GAsyncQueue *jobs;
    gboolean busy;
    guint check_id;
    char *last_problem;
    GPtrArray *composes;
    char *test_dir;
    guint state_id;
    int syncs, opens, sends;
    /* Signals */
    GDBusConnection *bus;
    GHashTable *notes;     /* Signal id -> Inbox UID (0: open the Inbox) */
    guint inbox_seen;      /* the Inbox's unread count at the last check */
    gboolean inbox_known;
    int signals_sent;
    /* started for something */
    gboolean compose_only; /* mailto: or --write: just a message, no window */
    gboolean pending_compose;
    char *pending_to, *pending_cc, *pending_subject, *pending_body, *pending_who, *pending_search;
} App;

static App app;

struct Compose {
    GtkWidget *win, *to, *cc, *subject, *body, *atts_box, *msg, *send_btn;
    GPtrArray *atts;       /* Attachment* */
    char *in_reply_to, *references;
    char *reply_folder;
    guint32 reply_uid;
    gboolean sending, dirty;
};

static void state_changed(void);
static void announce_new(GArray *uids);
static void announce_inbox(guint before, guint now);
static void update_title(void);
static void open_pending_compose(void);
static void on_search_activate(GtkEntry *e, gpointer d);
static void json_str(GString *s, const char *v);
static void fill_folders(void);
static void fill_list(void);
static void clear_shown(void);
static void show_message(void);
static void sync_now(void);
static void set_status(const char *text, gboolean bad);

static void add_class(GtkWidget *w, const char *c) { gtk_style_context_add_class(gtk_widget_get_style_context(w), c); }

static void destroy_child(GtkWidget *w, gpointer d)
{
    (void)d;
    gtk_widget_destroy(w);
}

static GtkWidget *label(const char *text, const char *cls, float x)
{
    GtkWidget *l = gtk_label_new(text);
    if (cls) add_class(l, cls);
    gtk_label_set_xalign(GTK_LABEL(l), x);
    gtk_label_set_line_wrap(GTK_LABEL(l), TRUE);
    gtk_label_set_line_wrap_mode(GTK_LABEL(l), PANGO_WRAP_WORD_CHAR);
    return l;
}

static GtkWidget *one_line(const char *text, const char *cls)
{
    GtkWidget *l = label(text, cls, 0);
    gtk_label_set_line_wrap(GTK_LABEL(l), FALSE);
    gtk_label_set_ellipsize(GTK_LABEL(l), PANGO_ELLIPSIZE_END);
    return l;
}

static GtkWidget *button(const char *text, const char *tip, GCallback cb, gpointer data, const char *cls)
{
    GtkWidget *b = gtk_button_new_with_label(text);
    if (tip) gtk_widget_set_tooltip_text(b, tip);
    if (cls) add_class(b, cls);
    if (cb) g_signal_connect(b, "clicked", cb, data);
    return b;
}

static void keep_hidden(GtkWidget *w)
{
    gtk_widget_show_all(w);
    gtk_widget_hide(w);
    gtk_widget_set_no_show_all(w, TRUE);
}

static char *when_short(gint64 t)
{
    if (t <= 0) return g_strdup("");
    GDateTime *d = g_date_time_new_from_unix_local(t), *now = g_date_time_new_now_local();
    char *s;
    if (g_date_time_get_year(d) == g_date_time_get_year(now) && g_date_time_get_day_of_year(d) == g_date_time_get_day_of_year(now))
        s = g_date_time_format(d, "%H:%M");
    else if (g_date_time_get_year(d) == g_date_time_get_year(now))
        s = g_date_time_format(d, "%e %b");
    else
        s = g_date_time_format(d, "%e %b %Y");
    g_date_time_unref(d);
    g_date_time_unref(now);
    return g_strstrip(s);
}

static Folder *folder_by_role(int role)
{
    for (guint i = 0; app.folders && i < app.folders->len; i++)
        if (((Folder *)app.folders->pdata[i])->role == role) return app.folders->pdata[i];
    return NULL;
}

static Folder *folder_named(const char *raw)
{
    for (guint i = 0; app.folders && raw && i < app.folders->len; i++)
        if (!strcmp(((Folder *)app.folders->pdata[i])->raw, raw)) return app.folders->pdata[i];
    return NULL;
}

/* ---- work, one job at a time on one connection ---- */

enum { J_SIGNIN, J_FOLDERS, J_SYNC, J_FETCH, J_FLAG, J_MOVE, J_FOREVER, J_SEARCH, J_SEND };

typedef struct Job {
    int kind;
    Server in, out;
    char *acc_id, *folder, *to, *flag, *words, *from;
    guint32 uid;
    gboolean on;
    GBytes *raw;
    GPtrArray *rcpt;
    char *sent_folder;
    Compose *compose;
    /* results */
    gboolean ok;
    GError *err;
    GPtrArray *folders, *summaries;
    GHashTable *flags, *unseen;
    guint32 uidvalidity, uidnext;
    GArray *found;
    GBytes *body;
} Job;

static Server copy_server(const Server *s)
{
    return (Server){g_strdup(s->host), s->port, s->security, g_strdup(s->user), g_strdup(s->pass)};
}

static void free_server(Server *s)
{
    g_free(s->host);
    g_free(s->user);
    if (s->pass) memset(s->pass, 0, strlen(s->pass));
    g_free(s->pass);
}

static void job_free(Job *j)
{
    free_server(&j->in);
    free_server(&j->out);
    g_free(j->acc_id); g_free(j->folder); g_free(j->to); g_free(j->flag); g_free(j->words); g_free(j->from); g_free(j->sent_folder);
    if (j->raw) g_bytes_unref(j->raw);
    if (j->rcpt) g_ptr_array_free(j->rcpt, TRUE);
    g_clear_error(&j->err);
    if (j->folders) g_ptr_array_free(j->folders, TRUE);
    if (j->summaries) g_ptr_array_free(j->summaries, TRUE);
    if (j->flags) g_hash_table_destroy(j->flags);
    if (j->unseen) g_hash_table_destroy(j->unseen);
    if (j->found) g_array_free(j->found, TRUE);
    if (j->body) g_bytes_unref(j->body);
    g_free(j);
}

static Job *new_job(int kind)
{
    Job *j = g_new0(Job, 1);
    j->kind = kind;
    if (app.acc) {
        j->in = copy_server(&app.acc->in);
        j->out = copy_server(&app.acc->out);
        j->acc_id = g_strdup(app.acc->id);
    }
    j->folder = g_strdup(app.folder);
    return j;
}

static gboolean job_done(gpointer data);

static gpointer worker(gpointer d)
{
    (void)d;
    Imap *m = NULL;
    char *m_key = NULL;
    for (;;) {
        Job *j = g_async_queue_pop(app.jobs);
        char *key = g_strdup_printf("%s|%s|%d|%s|%s", j->in.host, j->in.user, j->in.port, j->in.pass ? j->in.pass : "", j->acc_id);
        if (!m || g_strcmp0(key, m_key)) {
            imap_free(m);
            m = j->in.host ? imap_new(&j->in) : NULL;
            g_free(m_key);
            m_key = key;
        } else {
            g_free(key);
        }
        switch (j->kind) {
        case J_SIGNIN: {
            GPtrArray *none = g_ptr_array_new();
            j->ok = m && imap_check(m, &j->err) && smtp_send(&j->out, "", none, "", 0, &j->err);
            g_ptr_array_free(none, TRUE);
            if (j->ok) j->folders = imap_folders(m, &j->err);
            break;
        }
        case J_FOLDERS:
        case J_SYNC:
            j->folders = imap_folders(m, &j->err);
            j->ok = j->folders != NULL;
            if (j->ok) {
                j->unseen = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
                for (guint i = 0; i < j->folders->len; i++) {
                    Folder *f = j->folders->pdata[i];
                    guint32 un = 0;
                    if (!f->noselect && (f->role == FOLDER_INBOX || f->role == 0 || !g_strcmp0(f->raw, j->folder)) &&
                        imap_status(m, f->raw, NULL, &un, NULL, NULL, NULL))
                        g_hash_table_replace(j->unseen, g_strdup(f->raw), GUINT_TO_POINTER(un));
                }
            }
            if (j->ok && j->kind == J_SYNC && j->folder) {
                j->ok = imap_status(m, j->folder, NULL, NULL, &j->uidnext, &j->uidvalidity, &j->err);
                Box *b = j->ok ? box_load(j->acc_id, j->folder) : NULL;
                guint32 from = 1;
                if (b && b->uidvalidity == j->uidvalidity)
                    for (guint i = 0; i < b->entries->len; i++) from = MAX(from, ((Entry *)b->entries->pdata[i])->uid + 1);
                if (j->ok) j->summaries = imap_summaries(m, j->folder, from, &j->err);
                if (j->ok && j->summaries) j->flags = imap_flags_all(m, j->folder, &j->err);
                j->ok = j->ok && j->summaries && j->flags;
                box_free(b);
            }
            break;
        case J_FETCH:
            j->body = imap_fetch(m, j->folder, j->uid, &j->err);
            j->ok = j->body != NULL;
            break;
        case J_FLAG:
            j->ok = imap_flag(m, j->folder, j->uid, j->flag, j->on, &j->err);
            break;
        case J_MOVE:
            j->ok = imap_move(m, j->folder, j->uid, j->to, &j->err);
            break;
        case J_FOREVER:
            j->ok = imap_delete_forever(m, j->folder, j->uid, &j->err);
            break;
        case J_SEARCH:
            j->found = imap_search(m, j->folder, j->words, &j->err);
            j->ok = j->found != NULL;
            break;
        case J_SEND: {
            gsize n;
            const char *d2 = g_bytes_get_data(j->raw, &n);
            j->ok = smtp_send(&j->out, j->from, j->rcpt, d2, n, &j->err);
            if (j->ok && m && !j->sent_folder) {  /* started for one message: the folders aren't known yet */
                GPtrArray *fs = imap_folders(m, NULL);
                for (guint i = 0; fs && i < fs->len; i++)
                    if (((Folder *)fs->pdata[i])->role == FOLDER_SENT) j->sent_folder = g_strdup(((Folder *)fs->pdata[i])->raw);
                if (fs) g_ptr_array_free(fs, TRUE);
            }
            if (j->ok && m && j->sent_folder) imap_append(m, j->sent_folder, d2, n, NULL);  /* a copy in Sent */
            if (j->ok && m && j->folder && j->uid) imap_flag(m, j->folder, j->uid, "\\Answered", TRUE, NULL);
            break;
        }
        }
        if (!j->ok && m && j->err && (j->err->code == CURLE_SEND_ERROR || j->err->code == CURLE_RECV_ERROR)) {
            imap_free(m);  /* a broken connection: a fresh one next time */
            m = NULL;
        }
        g_idle_add(job_done, j);
    }
    return NULL;
}

static void submit(Job *j)
{
    app.busy = TRUE;
    gtk_widget_set_sensitive(app.refresh_btn, FALSE);
    g_async_queue_push(app.jobs, j);
}

/* ---- results, back on the main thread ---- */

static void set_status(const char *text, gboolean bad)
{
    gtk_label_set_text(GTK_LABEL(app.status), text ? text : "");
    GtkStyleContext *sc = gtk_widget_get_style_context(app.status);
    if (bad) gtk_style_context_add_class(sc, "bad");
    else gtk_style_context_remove_class(sc, "bad");
    g_free(app.last_problem);
    app.last_problem = bad ? g_strdup(text) : NULL;
    state_changed();
}

static void apply_folders(Job *j)
{
    if (!j->folders) return;
    if (app.folders) g_ptr_array_free(app.folders, TRUE);
    app.folders = j->folders;
    j->folders = NULL;
    if (j->unseen) {
        GHashTableIter it;
        gpointer k, v;
        g_hash_table_iter_init(&it, j->unseen);
        while (g_hash_table_iter_next(&it, &k, &v)) g_hash_table_replace(app.unseen, g_strdup(k), v);
    }
    if (!app.folder || !folder_named(app.folder)) {
        Folder *in = folder_by_role(FOLDER_INBOX);
        g_free(app.folder);
        app.folder = g_strdup(in ? in->raw : app.folders->len ? ((Folder *)app.folders->pdata[0])->raw : "INBOX");
    }
    fill_folders();
}

static gboolean is_inbox(const char *raw)
{
    Folder *in = folder_by_role(FOLDER_INBOX);
    return raw && (in ? !strcmp(in->raw, raw) : !g_ascii_strcasecmp(raw, "INBOX"));
}

static void apply_sync(Job *j)
{
    if (!app.box || g_strcmp0(j->folder, app.folder)) return;
    /* new mail is only news once there was mail before (not on the first check) */
    gboolean had = app.box->entries->len > 0 && app.box->uidvalidity == j->uidvalidity;
    GArray *fresh = g_array_new(FALSE, FALSE, sizeof(guint32));
    if (app.box->uidvalidity != j->uidvalidity) {  /* the server renumbered: start again */
        g_ptr_array_set_size(app.box->entries, 0);
        app.box->uidvalidity = j->uidvalidity;
    }
    app.box->uidnext = j->uidnext;
    for (guint i = 0; i < j->summaries->len; i++) {
        Summary *s = j->summaries->pdata[i];
        if (box_find(app.box, s->uid)) continue;
        if (!s->seen) g_array_append_val(fresh, s->uid);
        gsize n;
        const char *h = g_bytes_get_data(s->headers, &n);
        Message *m = mime_parse_headers(h, n);
        Entry *e = g_new0(Entry, 1);
        e->uid = s->uid;
        e->seen = s->seen;
        e->flagged = s->flagged;
        e->answered = s->answered;
        e->size = s->size;
        e->date = m->date;
        Address *a = m->from->len ? m->from->pdata[0] : NULL;
        e->from = a ? address_format(a) : g_strdup("");
        e->subject = g_strdup(m->subject);
        e->message_id = g_strdup(m->message_id);
        e->preview = g_strdup("");
        g_ptr_array_add(app.box->entries, e);
        message_free(m);
    }
    /* flags changed elsewhere, and messages that went */
    for (guint i = 0; i < app.box->entries->len;) {
        Entry *e = app.box->entries->pdata[i];
        guint bits = GPOINTER_TO_UINT(g_hash_table_lookup(j->flags, GUINT_TO_POINTER(e->uid)));
        if (!bits) {
            if (app.open == e) app.open = NULL;
            box_remove(app.box, e->uid);
            continue;
        }
        e->seen = bits & 1;
        e->flagged = (bits & 2) != 0;
        e->answered = (bits & 4) != 0;
        i++;
    }
    box_save(app.box);
    if (had && is_inbox(j->folder)) announce_new(fresh);
    g_array_free(fresh, TRUE);
}

static void compose_done(Compose *c, Job *j);

static gboolean job_done(gpointer data)
{
    Job *j = data;
    app.busy = g_async_queue_length(app.jobs) > 0;
    gtk_widget_set_sensitive(app.refresh_btn, !app.busy);
    switch (j->kind) {
    case J_SIGNIN:
        gtk_widget_set_sensitive(app.su_btn, TRUE);
        if (j->ok) {
            /* save it: servers in accounts.ini, the password encrypted */
            Account *a = g_new0(Account, 1);
            a->id = account_new_id(app.accounts);
            a->name = g_strdup(gtk_entry_get_text(GTK_ENTRY(app.su_name)));
            a->address = g_ascii_strdown(gtk_entry_get_text(GTK_ENTRY(app.su_addr)), -1);
            a->in = copy_server(&j->in);
            a->out = copy_server(&j->out);
            GError *e = NULL;
            if (!secret_set(a->id, j->in.pass, &e)) {
                gtk_label_set_text(GTK_LABEL(app.su_msg), e ? e->message : "Mail couldn't keep the password.");
                gtk_widget_show(app.su_msg);
                g_clear_error(&e);
                account_free(a);
                break;
            }
            g_ptr_array_add(app.accounts, a);
            accounts_save(app.accounts, NULL);
            app.acc = a;
            apply_folders(j);
            gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "mail");
            gtk_widget_set_sensitive(app.write_btn, TRUE);
            if (app.box) box_free(app.box);
            app.box = box_load(app.acc->id, app.folder);
            fill_list();
            sync_now();
            if (app.pending_compose) open_pending_compose();  /* a link brought us here before there was an account */
        } else {
            gtk_label_set_text(GTK_LABEL(app.su_msg), j->err ? j->err->message : "Signing in didn't work.");
            gtk_widget_show(app.su_msg);
        }
        break;
    case J_FOLDERS:
    case J_SYNC:
        app.syncs++;
        if (j->ok) {
            Folder *in = folder_by_role(FOLDER_INBOX);
            guint before = in ? GPOINTER_TO_UINT(g_hash_table_lookup(app.unseen, in->raw)) : 0;
            apply_folders(j);
            if (j->kind == J_SYNC) apply_sync(j);
            in = folder_by_role(FOLDER_INBOX);
            guint now = in ? GPOINTER_TO_UINT(g_hash_table_lookup(app.unseen, in->raw)) : 0;
            if (in && app.inbox_known && !is_inbox(app.folder)) announce_inbox(before, now);
            if (in) app.inbox_known = TRUE;
            fill_list();
            set_status("", FALSE);
            if (app.pending_search && j->kind == J_SYNC) {  /* mail --search: once the list is here */
                gtk_entry_set_text(GTK_ENTRY(app.search), app.pending_search);
                on_search_activate(GTK_ENTRY(app.search), NULL);
                g_clear_pointer(&app.pending_search, g_free);
            }
        } else {
            set_status(j->err ? j->err->message : "Mail couldn't check for new mail.", TRUE);
        }
        break;
    case J_FETCH:
        app.opens++;
        if (j->ok && app.box && !g_strcmp0(j->folder, app.folder)) {
            box_put_body(app.box, j->uid, j->body);
            Entry *e = box_find(app.box, j->uid);
            if (e && app.open == e) show_message();
        } else if (!j->ok) {
            set_status(j->err ? j->err->message : "That message couldn't be fetched.", TRUE);
            gtk_label_set_text(GTK_LABEL(app.read_text), "This message couldn't be fetched. Check that you're online, and try again.");
        }
        break;
    case J_MOVE:
    case J_FOREVER:
    case J_FLAG:
        if (!j->ok) {
            set_status(j->err ? j->err->message : "The server didn't do that.", TRUE);
            sync_now();  /* show what is really there */
        }
        break;
    case J_SEARCH:
        if (j->ok && app.box && !g_strcmp0(j->folder, app.folder) && !g_strcmp0(j->words, gtk_entry_get_text(GTK_ENTRY(app.search)))) {
            /* UIDs, not entries: a sync or another folder may free those */
            clear_shown();
            app.shown = g_hash_table_new(g_direct_hash, g_direct_equal);
            app.shown_q = g_strdup(j->words);
            for (guint k = 0; k < j->found->len; k++)
                g_hash_table_add(app.shown, GUINT_TO_POINTER(g_array_index(j->found, guint32, k)));
            fill_list();
        } else if (!j->ok) {
            set_status(j->err ? j->err->message : "Searching didn't work.", TRUE);
        }
        break;
    case J_SEND:
        app.sends++;
        compose_done(j->compose, j);
        if (j->ok && !app.compose_only) sync_now();
        break;
    }
    job_free(j);
    state_changed();
    return G_SOURCE_REMOVE;
}

static void sync_now(void)
{
    if (!app.acc) return;
    submit(new_job(J_SYNC));
}

static gboolean every_few_minutes(gpointer d)
{
    (void)d;
    if (!app.busy) sync_now();
    return G_SOURCE_CONTINUE;
}

/* ---- folders ---- */

static const char *folder_title(const Folder *f)
{
    switch (f->role) {
    case FOLDER_INBOX: return "Inbox";
    case FOLDER_SENT: return "Sent";
    case FOLDER_DRAFTS: return "Drafts";
    case FOLDER_TRASH: return "Trash";
    case FOLDER_JUNK: return "Junk";
    case FOLDER_ARCHIVE: return "Archive";
    default: return f->name;
    }
}

static void open_folder(const char *raw)
{
    if (!raw || (app.folder && !strcmp(raw, app.folder) && app.box)) return;
    g_free(app.folder);
    app.folder = g_strdup(raw);
    if (app.box) box_free(app.box);
    app.box = box_load(app.acc->id, app.folder);
    app.open = NULL;
    clear_shown();
    gtk_entry_set_text(GTK_ENTRY(app.search), "");
    fill_folders();
    fill_list();
    show_message();
    sync_now();
}

static void on_folder_row(GtkListBox *b, GtkListBoxRow *row, gpointer d)
{
    (void)b; (void)d;
    open_folder(g_object_get_data(G_OBJECT(row), "raw"));
}

static gint folder_order(gconstpointer a, gconstpointer b)
{
    const Folder *x = *(Folder *const *)a, *y = *(Folder *const *)b;
    int rx = x->role ? x->role : 99, ry = y->role ? y->role : 99;
    return rx != ry ? rx - ry : g_utf8_collate(x->name, y->name);
}

static void fill_folders(void)
{
    gtk_container_foreach(GTK_CONTAINER(app.folder_list), destroy_child, NULL);
    if (!app.folders) return;
    update_title();
    GPtrArray *order = g_ptr_array_new();
    for (guint i = 0; i < app.folders->len; i++)
        if (!((Folder *)app.folders->pdata[i])->noselect) g_ptr_array_add(order, app.folders->pdata[i]);
    g_ptr_array_sort(order, folder_order);
    if (app.acc) {
        GtkWidget *who = one_line(app.acc->address, "sub");
        gtk_widget_set_margin_start(who, 12);
        gtk_widget_set_margin_bottom(who, 8);
        GtkWidget *row = gtk_list_box_row_new();
        gtk_list_box_row_set_activatable(GTK_LIST_BOX_ROW(row), FALSE);
        gtk_list_box_row_set_selectable(GTK_LIST_BOX_ROW(row), FALSE);
        gtk_container_add(GTK_CONTAINER(row), who);
        gtk_container_add(GTK_CONTAINER(app.folder_list), row);
    }
    for (guint i = 0; i < order->len; i++) {
        Folder *f = order->pdata[i];
        GtkWidget *row = gtk_list_box_row_new(), *box = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        g_object_set_data_full(G_OBJECT(row), "raw", g_strdup(f->raw), g_free);
        add_class(row, "folder");
        if (!g_strcmp0(f->raw, app.folder)) add_class(row, "chosen");
        g_object_set(box, "margin", 8, NULL);
        gtk_box_pack_start(GTK_BOX(box), one_line(folder_title(f), "name"), TRUE, TRUE, 0);
        guint un = GPOINTER_TO_UINT(g_hash_table_lookup(app.unseen, f->raw));
        if (un && f->role != FOLDER_TRASH && f->role != FOLDER_JUNK && f->role != FOLDER_SENT) {
            char n[16];
            g_snprintf(n, sizeof n, "%u", un);
            gtk_box_pack_end(GTK_BOX(box), label(n, "count", 1), FALSE, FALSE, 0);
        }
        gtk_container_add(GTK_CONTAINER(row), box);
        gtk_container_add(GTK_CONTAINER(app.folder_list), row);
    }
    g_ptr_array_free(order, TRUE);
    gtk_widget_show_all(app.folder_list);
}

/* ---- the message list ---- */

static gboolean draw_unread(GtkWidget *w, cairo_t *cr, gpointer d)
{
    (void)d;
    Entry *e = g_object_get_data(G_OBJECT(w), "entry");
    if (!e) return TRUE;
    GdkRGBA c;
    if (!gtk_style_context_lookup_color(gtk_widget_get_style_context(w), e->flagged ? "nexa_warn" : "nexa_accent", &c))
        gdk_rgba_parse(&c, e->flagged ? "#ffb547" : "#4da3ff");
    double cx = gtk_widget_get_allocated_width(w) / 2.0, cy = 12, r = 5;
    for (int i = 0; i < 6; i++) {  /* a small hexagon: unread (filled) or flagged */
        double a = (-90 + 60 * i) * G_PI / 180;
        if (i) cairo_line_to(cr, cx + r * cos(a), cy + r * sin(a));
        else cairo_move_to(cr, cx + r * cos(a), cy + r * sin(a));
    }
    cairo_close_path(cr);
    gdk_cairo_set_source_rgba(cr, &c);
    if (!e->seen) cairo_fill(cr);
    else if (e->flagged) {
        cairo_set_line_width(cr, 1.5);
        cairo_stroke(cr);
    }
    return TRUE;
}

static void on_msg_row(GtkListBox *b, GtkListBoxRow *row, gpointer d)
{
    (void)b; (void)d;
    Entry *e = row ? g_object_get_data(G_OBJECT(row), "entry") : NULL;
    if (!e || e == app.open) return;
    app.open = e;
    if (!e->seen) {
        e->seen = TRUE;  /* fetching it marks it read on the server too */
        guint un = GPOINTER_TO_UINT(g_hash_table_lookup(app.unseen, app.folder));
        if (un) g_hash_table_replace(app.unseen, g_strdup(app.folder), GUINT_TO_POINTER(un - 1));
        fill_folders();
        box_save(app.box);
        Job *j = new_job(J_FLAG);
        j->uid = e->uid;
        j->flag = g_strdup("\\Seen");
        j->on = TRUE;
        submit(j);
    }
    GtkWidget *dot = g_object_get_data(G_OBJECT(row), "dot");
    if (dot) gtk_widget_queue_draw(dot);
    show_message();
}

static gboolean matches(const Entry *e, const char *q)
{
    if (!q || !*q) return TRUE;
    char *hay = g_strjoin(" ", e->from ? e->from : "", e->subject ? e->subject : "", e->preview ? e->preview : "", NULL);
    char *h = g_utf8_casefold(hay, -1), *n = g_utf8_casefold(q, -1);
    gboolean ok = strstr(h, n) != NULL;
    g_free(hay);
    g_free(h);
    g_free(n);
    return ok;
}

static void clear_shown(void)
{
    if (app.shown) g_hash_table_destroy(app.shown);
    app.shown = NULL;
    g_clear_pointer(&app.shown_q, g_free);
}

static void fill_list(void)
{
    gtk_container_foreach(GTK_CONTAINER(app.msg_list), destroy_child, NULL);
    const char *q = gtk_entry_get_text(GTK_ENTRY(app.search));
    GPtrArray *list = g_ptr_array_new();
    GPtrArray *src = app.box ? app.box->entries : NULL;
    for (guint i = 0; src && i < src->len; i++) {
        Entry *e = src->pdata[i];
        if (app.shown ? g_hash_table_contains(app.shown, GUINT_TO_POINTER(e->uid)) : matches(e, q)) g_ptr_array_add(list, e);
    }
    Folder *f = folder_named(app.folder);
    char *head = *q ? g_strdup_printf("Results for “%s”", q) : g_strdup(f ? folder_title(f) : "Mail");
    gtk_label_set_text(GTK_LABEL(app.list_head), head);
    g_free(head);
    for (guint i = 0; i < list->len && i < 2000; i++) {
        Entry *e = list->pdata[i];
        GtkWidget *row = gtk_list_box_row_new(), *box = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
        g_object_set_data(G_OBJECT(row), "entry", e);
        add_class(row, "msg");
        if (!e->seen) add_class(row, "unread");
        GtkWidget *dot = gtk_drawing_area_new();
        gtk_widget_set_size_request(dot, 14, -1);
        g_object_set_data(G_OBJECT(dot), "entry", e);
        g_signal_connect(dot, "draw", G_CALLBACK(draw_unread), NULL);
        g_object_set_data(G_OBJECT(row), "dot", dot);
        gtk_box_pack_start(GTK_BOX(box), dot, FALSE, FALSE, 0);
        GtkWidget *texts = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2), *top = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
        /* the sender's name, not their address, when there is one */
        char *who = g_strdup(e->from && *e->from ? e->from : "(no sender)"), *lt = strchr(who, '<');
        if (lt && lt > who) *lt = 0;
        gtk_box_pack_start(GTK_BOX(top), one_line(g_strstrip(who), "from"), TRUE, TRUE, 0);
        g_free(who);
        char *when = when_short(e->date);
        gtk_box_pack_end(GTK_BOX(top), label(when, "when", 1), FALSE, FALSE, 0);
        g_free(when);
        gtk_box_pack_start(GTK_BOX(texts), top, FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(texts), one_line(e->subject && *e->subject ? e->subject : "(no subject)", "subject"), FALSE, FALSE, 0);
        if (e->preview && *e->preview) gtk_box_pack_start(GTK_BOX(texts), one_line(e->preview, "preview"), FALSE, FALSE, 0);
        gtk_box_pack_start(GTK_BOX(box), texts, TRUE, TRUE, 0);
        g_object_set(box, "margin", 8, NULL);
        gtk_container_add(GTK_CONTAINER(row), box);
        gtk_container_add(GTK_CONTAINER(app.msg_list), row);
        if (e == app.open) gtk_list_box_select_row(GTK_LIST_BOX(app.msg_list), GTK_LIST_BOX_ROW(row));
    }
    gtk_widget_show_all(app.msg_list);
    gtk_label_set_text(GTK_LABEL(app.list_empty), *q ? "Nothing matches. Press Enter to search every message on the server."
                                                     : app.busy ? "Checking for mail…" : "No messages here.");
    gtk_widget_set_visible(app.list_empty, list->len == 0);
    g_ptr_array_free(list, TRUE);
    state_changed();
}

static void on_search_changed(GtkSearchEntry *e, gpointer d)
{
    (void)e; (void)d;
    /* GTK says so a moment late: keep the server's results for these same words */
    if (app.shown && g_strcmp0(gtk_entry_get_text(GTK_ENTRY(app.search)), app.shown_q)) clear_shown();
    fill_list();
}

static void on_search_activate(GtkEntry *e, gpointer d)
{
    (void)d;
    const char *q = gtk_entry_get_text(e);
    if (!*q || !app.acc) return;
    Job *j = new_job(J_SEARCH);
    j->words = g_strdup(q);
    submit(j);
}

/* ---- reading ---- */

/* FileTrail: "where did that attachment go?". Mail can't write FileTrail's
 * own journal (nor should an app); it keeps a record in its own data
 * folder, which FileTrail reads, believing only paths inside the folders
 * Mail may write (nexa_sys::filetrail::app_saves). The newest 200 count. */
static void note_saved(const char *path)
{
    char *dir = g_build_filename(g_get_user_data_dir(), APP_ID, NULL), *log = g_build_filename(dir, "nexa-saved.jsonl", NULL);
    g_mkdir_with_parents(dir, 0700);
    char *old = NULL;
    gsize n = 0;
    GString *s = g_string_new(NULL);
    if (g_file_get_contents(log, &old, &n, NULL) && n > (64u << 10)) {  /* too long: keep the newer half */
        const char *half = strchr(old + n / 2, '\n');
        g_string_append(s, half ? half + 1 : "");
    } else if (old) {
        g_string_append(s, old);
    }
    g_string_append_printf(s, "{\"at\": %" G_GINT64_FORMAT ", \"path\": ", g_get_real_time() / G_USEC_PER_SEC);
    json_str(s, path);
    g_string_append(s, "}\n");
    g_file_set_contents(log, s->str, (gssize)s->len, NULL);
    g_chmod(log, 0600);
    g_string_free(s, TRUE);
    g_free(old);
    g_free(log);
    g_free(dir);
}

static void on_save_att(GtkButton *b, gpointer d)
{
    (void)b;
    Attachment *a = d;
    const char *docs = g_get_user_special_dir(G_USER_DIRECTORY_DOCUMENTS);
    char *base = docs ? g_strdup(docs) : g_build_filename(g_get_home_dir(), "Documents", NULL);
    char *dir = g_build_filename(base, "Mail attachments", NULL);
    g_mkdir_with_parents(dir, 0700);
    char *path = g_build_filename(dir, a->filename, NULL);
    for (int n = 2; g_file_test(path, G_FILE_TEST_EXISTS) && n < 1000; n++) {
        g_free(path);
        const char *dot = strrchr(a->filename, '.');
        char *name = dot && dot != a->filename ? g_strdup_printf("%.*s (%d)%s", (int)(dot - a->filename), a->filename, n, dot)
                                               : g_strdup_printf("%s (%d)", a->filename, n);
        path = g_build_filename(dir, name, NULL);
        g_free(name);
    }
    gsize n;
    const char *data = g_bytes_get_data(a->data, &n);
    GError *e = NULL;
    if (g_file_set_contents(path, data ? data : "", (gssize)n, &e)) {
        char *base_name = g_path_get_basename(path), *m = g_strdup_printf("Saved to Documents/Mail attachments as %s.", base_name);
        set_status(m, FALSE);
        note_saved(path);
        g_free(m);
        g_free(base_name);
    } else {
        set_status("That attachment couldn't be saved. Is the disk full?", TRUE);
        g_clear_error(&e);
    }
    g_free(path);
    g_free(dir);
    g_free(base);
}

static char *addresses_text(GPtrArray *l)
{
    GString *s = g_string_new(NULL);
    for (guint i = 0; l && i < l->len; i++) {
        char *a = address_format(l->pdata[i]);
        g_string_append_printf(s, "%s%s", i ? ", " : "", a);
        g_free(a);
    }
    return g_string_free(s, FALSE);
}

static void show_message(void)
{
    Entry *e = app.open;
    gtk_widget_set_visible(app.read_scroll, e != NULL);
    gtk_widget_set_visible(app.read_empty, e == NULL);
    if (app.msg) message_free(app.msg);
    app.msg = NULL;
    if (!e) {
        state_changed();
        return;
    }
    gtk_label_set_text(GTK_LABEL(app.read_subject), e->subject && *e->subject ? e->subject : "(no subject)");
    gtk_label_set_text(GTK_LABEL(app.read_from), e->from);
    GDateTime *dt = e->date ? g_date_time_new_from_unix_local(e->date) : NULL;
    char *when = dt ? g_date_time_format(dt, "%A %e %B %Y, %H:%M") : g_strdup("");
    gtk_label_set_text(GTK_LABEL(app.read_date), g_strstrip(when));
    g_free(when);
    if (dt) g_date_time_unref(dt);
    gtk_button_set_label(GTK_BUTTON(app.flag_btn), e->flagged ? "⬢ Flagged" : "⬡ Flag");
    gtk_container_foreach(GTK_CONTAINER(app.read_atts), destroy_child, NULL);
    gtk_widget_hide(app.html_note);
    GBytes *raw = box_body(app.box, e->uid);
    if (!raw) {
        gtk_label_set_text(GTK_LABEL(app.read_to), "");
        gtk_label_set_text(GTK_LABEL(app.read_text), "Fetching…");
        Job *j = new_job(J_FETCH);
        j->uid = e->uid;
        submit(j);
        state_changed();
        return;
    }
    gsize n;
    const char *d = g_bytes_get_data(raw, &n);
    app.msg = mime_parse(d ? d : "", n);
    g_bytes_unref(raw);
    char *to = addresses_text(app.msg->to), *cc = addresses_text(app.msg->cc);
    char *tl = *cc ? g_strdup_printf("To %s · Cc %s", to, cc) : g_strdup_printf("To %s", to);
    gtk_label_set_text(GTK_LABEL(app.read_to), tl);
    g_free(tl); g_free(to); g_free(cc);
    gtk_label_set_text(GTK_LABEL(app.read_text), *app.msg->text ? app.msg->text : "(This message has no text.)");
    gtk_widget_set_visible(app.html_note, app.msg->was_html);
    for (guint i = 0; i < app.msg->attachments->len; i++) {
        Attachment *a = app.msg->attachments->pdata[i];
        char *sz = g_format_size(g_bytes_get_size(a->data)), *t = g_strdup_printf("%s · %s", a->filename, sz);
        GtkWidget *b = button(t, "Save it in Documents/Mail attachments", G_CALLBACK(on_save_att), a, "attachment");
        gtk_container_add(GTK_CONTAINER(app.read_atts), b);
        g_free(sz);
        g_free(t);
    }
    gtk_widget_show_all(app.read_atts);
    /* a preview line for the list, once read */
    if (!e->preview || !*e->preview) {
        g_free(e->preview);
        char *flat = g_strdup(app.msg->text);
        for (char *c = flat; *c; c++)
            if (*c == '\n' || *c == '\t') *c = ' ';
        e->preview = g_utf8_substring(g_strstrip(flat), 0, MIN(120, g_utf8_strlen(flat, -1)));
        g_free(flat);
        box_save(app.box);
    }
    state_changed();
}

static void on_flag(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    Entry *e = app.open;
    if (!e) return;
    e->flagged = !e->flagged;
    box_save(app.box);
    Job *j = new_job(J_FLAG);
    j->uid = e->uid;
    j->flag = g_strdup("\\Flagged");
    j->on = e->flagged;
    submit(j);
    show_message();
    fill_list();
}

static void on_delete(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    Entry *e = app.open;
    if (!e) return;
    Folder *trash = folder_by_role(FOLDER_TRASH), *here = folder_named(app.folder);
    Job *j;
    if (here && here->role == FOLDER_TRASH) {
        j = new_job(J_FOREVER);
        set_status("Deleted for good.", FALSE);
    } else if (trash) {
        j = new_job(J_MOVE);
        j->to = g_strdup(trash->raw);
        set_status("Moved to Trash.", FALSE);
    } else {
        set_status("This account has no Trash folder, so Mail didn't delete it.", TRUE);
        return;
    }
    j->uid = e->uid;
    submit(j);
    /* the next one down in the list as shown (newest first) gets the focus */
    int at = 0, k = 0;
    GList *rows = gtk_container_get_children(GTK_CONTAINER(app.msg_list));
    for (GList *r = rows; r; r = r->next, k++)
        if (g_object_get_data(G_OBJECT(r->data), "entry") == e) at = k;
    g_list_free(rows);
    box_remove(app.box, e->uid);
    box_save(app.box);
    app.open = NULL;
    fill_list();
    rows = gtk_container_get_children(GTK_CONTAINER(app.msg_list));
    GtkWidget *next = g_list_nth_data(rows, (guint)at);
    if (!next && rows) next = g_list_last(rows)->data;  /* it was the last: the one above */
    g_list_free(rows);
    if (next) {
        gtk_list_box_select_row(GTK_LIST_BOX(app.msg_list), GTK_LIST_BOX_ROW(next));
        on_msg_row(NULL, GTK_LIST_BOX_ROW(next), NULL);
    } else {
        show_message();
    }
}

/* ---- writing ---- */

static void compose_message(Compose *c, const char *text, gboolean bad)
{
    gtk_label_set_text(GTK_LABEL(c->msg), text);
    GtkStyleContext *sc = gtk_widget_get_style_context(c->msg);
    if (bad) gtk_style_context_add_class(sc, "bad");
    else gtk_style_context_remove_class(sc, "bad");
    gtk_widget_set_visible(c->msg, *text != 0);
}

static void compose_free(Compose *c)
{
    if (c->atts) g_ptr_array_free(c->atts, TRUE);
    g_free(c->in_reply_to);
    g_free(c->references);
    g_free(c->reply_folder);
    g_free(c);
}

static void att_free(Attachment *a)
{
    g_free(a->filename);
    g_free(a->type);
    if (a->data) g_bytes_unref(a->data);
    g_free(a);
}

static void fill_compose_atts(Compose *c);

static void on_remove_att(GtkButton *b, gpointer d)
{
    Compose *c = d;
    Attachment *a = g_object_get_data(G_OBJECT(b), "att");
    g_ptr_array_remove(c->atts, a);
    fill_compose_atts(c);
}

static void fill_compose_atts(Compose *c)
{
    gtk_container_foreach(GTK_CONTAINER(c->atts_box), destroy_child, NULL);
    for (guint i = 0; i < c->atts->len; i++) {
        Attachment *a = c->atts->pdata[i];
        char *sz = g_format_size(g_bytes_get_size(a->data)), *t = g_strdup_printf("%s · %s  ✕", a->filename, sz);
        GtkWidget *b = button(t, "Take it out", G_CALLBACK(on_remove_att), c, "attachment");
        g_object_set_data(G_OBJECT(b), "att", a);
        gtk_container_add(GTK_CONTAINER(c->atts_box), b);
        g_free(sz);
        g_free(t);
    }
    gtk_widget_show_all(c->atts_box);
}

static gboolean attach_file(Compose *c, const char *path)
{
    char *data = NULL;
    gsize n = 0;
    GError *e = NULL;
    GStatBuf st;
    if (g_stat(path, &st) == 0 && st.st_size > 25 * 1024 * 1024) {
        compose_message(c, "That file is bigger than 25 MB: most mail servers won't take it.", TRUE);
        return FALSE;
    }
    if (!g_file_get_contents(path, &data, &n, &e)) {
        compose_message(c, "Mail can't read that file. It can attach files from your Documents and Pictures.", TRUE);
        g_clear_error(&e);
        return FALSE;
    }
    Attachment *a = g_new0(Attachment, 1);
    char *base = g_path_get_basename(path);
    a->filename = mime_safe_filename(base);
    g_free(base);
    char *ct = g_content_type_guess(path, (const guchar *)data, MIN(n, 4096), NULL), *mt = g_content_type_get_mime_type(ct);
    a->type = mt ? mt : g_strdup("application/octet-stream");
    g_free(ct);
    a->data = g_bytes_new_take(data, n);
    g_ptr_array_add(c->atts, a);
    c->dirty = TRUE;
    fill_compose_atts(c);
    return TRUE;
}

static void on_attach(GtkButton *b, gpointer d)
{
    (void)b;
    Compose *c = d;
    GtkFileChooserNative *ch = gtk_file_chooser_native_new("Attach a File", GTK_WINDOW(c->win), GTK_FILE_CHOOSER_ACTION_OPEN, "Attach", "Cancel");
    gtk_file_chooser_set_select_multiple(GTK_FILE_CHOOSER(ch), TRUE);
    const char *docs = g_get_user_special_dir(G_USER_DIRECTORY_DOCUMENTS);
    if (docs) gtk_file_chooser_set_current_folder(GTK_FILE_CHOOSER(ch), docs);
    if (gtk_native_dialog_run(GTK_NATIVE_DIALOG(ch)) == GTK_RESPONSE_ACCEPT) {
        GSList *files = gtk_file_chooser_get_filenames(GTK_FILE_CHOOSER(ch));
        for (GSList *f = files; f; f = f->next) attach_file(c, f->data);
        g_slist_free_full(files, g_free);
    }
    g_object_unref(ch);
}

/* "a@x, Kofi <k@y>" -> addresses, or NULL and *bad: the first one that isn't */
static GPtrArray *parse_field(const char *text, char **bad)
{
    GPtrArray *out = g_ptr_array_new_with_free_func((GDestroyNotify)address_free);
    char **parts = g_strsplit_set(text, ",;", -1);
    for (int i = 0; parts[i]; i++) {
        char *p = g_strstrip(parts[i]);
        if (!*p) continue;
        GPtrArray *one = mime_addresses(p);
        if (!one->len) {
            *bad = g_strdup(p);
            g_ptr_array_free(one, TRUE);
            g_ptr_array_free(out, TRUE);
            g_strfreev(parts);
            return NULL;
        }
        g_ptr_array_add(out, g_ptr_array_steal_index(one, 0));
        g_ptr_array_free(one, TRUE);
    }
    g_strfreev(parts);
    return out;
}

static char *text_of(GtkWidget *tv)
{
    GtkTextBuffer *b = gtk_text_view_get_buffer(GTK_TEXT_VIEW(tv));
    GtkTextIter s, e;
    gtk_text_buffer_get_bounds(b, &s, &e);
    return gtk_text_buffer_get_text(b, &s, &e, FALSE);
}

static void send_compose(Compose *c)
{
    if (c->sending || !app.acc) return;
    char *bad = NULL;
    GPtrArray *to = parse_field(gtk_entry_get_text(GTK_ENTRY(c->to)), &bad);
    GPtrArray *cc = to ? parse_field(gtk_entry_get_text(GTK_ENTRY(c->cc)), &bad) : NULL;
    if (!to || !cc) {
        char *m = g_strdup_printf("“%s” isn't an email address. Check it, and try again.", bad);
        compose_message(c, m, TRUE);
        g_free(m);
        g_free(bad);
        if (to) g_ptr_array_free(to, TRUE);
        return;
    }
    if (!to->len && !cc->len) {
        compose_message(c, "Who is it for? Type an address in To.", TRUE);
        g_ptr_array_free(to, TRUE);
        g_ptr_array_free(cc, TRUE);
        return;
    }
    char *body = text_of(c->body);
    Draft d = {app.acc->name, app.acc->address, to, cc, gtk_entry_get_text(GTK_ENTRY(c->subject)), body, c->atts,
               c->in_reply_to, c->references, 0};
    gsize len = 0;
    char *raw = mime_build(&d, NULL, &len);
    Job *j = new_job(J_SEND);
    j->raw = g_bytes_new_take(raw, len);
    j->from = g_strdup(app.acc->address);
    j->rcpt = g_ptr_array_new_with_free_func(g_free);
    for (guint i = 0; i < to->len; i++) g_ptr_array_add(j->rcpt, g_strdup(((Address *)to->pdata[i])->addr));
    for (guint i = 0; i < cc->len; i++) g_ptr_array_add(j->rcpt, g_strdup(((Address *)cc->pdata[i])->addr));
    Folder *sent = folder_by_role(FOLDER_SENT);
    j->sent_folder = sent ? g_strdup(sent->raw) : NULL;
    g_free(j->folder);
    j->folder = g_strdup(c->reply_folder);
    j->uid = c->reply_uid;
    j->compose = c;
    c->sending = TRUE;
    gtk_widget_set_sensitive(c->send_btn, FALSE);
    compose_message(c, "Sending…", FALSE);
    submit(j);
    g_free(body);
    g_ptr_array_free(to, TRUE);
    g_ptr_array_free(cc, TRUE);
}

static void compose_done(Compose *c, Job *j)
{
    if (!c || !g_ptr_array_find(app.composes, c, NULL)) return;
    c->sending = FALSE;
    gtk_widget_set_sensitive(c->send_btn, TRUE);
    if (j->ok) {
        set_status("Sent.", FALSE);
        c->dirty = FALSE;
        gtk_widget_destroy(c->win);
    } else {
        compose_message(c, j->err ? j->err->message : "Sending didn't work. Your message is still here.", TRUE);
    }
}

static void on_send(GtkButton *b, gpointer d)
{
    (void)b;
    send_compose(d);
}

static void on_compose_destroy(GtkWidget *w, gpointer d)
{
    (void)w;
    Compose *c = d;
    g_ptr_array_remove(app.composes, c);
    compose_free(c);
    state_changed();
    /* started just for a message: done when it's sent or closed */
    if (app.compose_only && !app.composes->len && !gtk_widget_get_visible(app.win)) gtk_main_quit();
}

static gboolean on_compose_delete(GtkWidget *w, GdkEvent *e, gpointer d)
{
    (void)w; (void)e;
    Compose *c = d;
    char *body = text_of(c->body);
    gboolean has = c->dirty || *gtk_entry_get_text(GTK_ENTRY(c->to)) || strlen(body) > 0;
    g_free(body);
    if (!has || app.test_dir) return FALSE;
    GtkWidget *q = gtk_message_dialog_new(GTK_WINDOW(c->win), GTK_DIALOG_MODAL, GTK_MESSAGE_OTHER, GTK_BUTTONS_NONE, "Throw this message away?");
    gtk_message_dialog_format_secondary_text(GTK_MESSAGE_DIALOG(q), "It hasn't been sent.");
    gtk_dialog_add_button(GTK_DIALOG(q), "Keep Writing", GTK_RESPONSE_CANCEL);
    add_class(gtk_dialog_add_button(GTK_DIALOG(q), "Throw Away", GTK_RESPONSE_OK), "danger");
    gboolean away = gtk_dialog_run(GTK_DIALOG(q)) == GTK_RESPONSE_OK;
    gtk_widget_destroy(q);
    return !away;
}

static gboolean on_compose_key(GtkWidget *w, GdkEventKey *e, gpointer d)
{
    (void)w;
    if ((e->state & GDK_CONTROL_MASK) && (e->keyval == GDK_KEY_Return || e->keyval == GDK_KEY_KP_Enter)) {
        send_compose(d);
        return TRUE;
    }
    return FALSE;
}

static GtkWidget *field_row(GtkWidget *grid, int row, const char *name, GtkWidget *w)
{
    gtk_grid_attach(GTK_GRID(grid), label(name, "sub", 1), 0, row, 1, 1);
    gtk_widget_set_hexpand(w, TRUE);
    gtk_grid_attach(GTK_GRID(grid), w, 1, row, 1, 1);
    return w;
}

static Compose *compose_new(const char *to, const char *cc, const char *subject, const char *body)
{
    Compose *c = g_new0(Compose, 1);
    c->atts = g_ptr_array_new_with_free_func((GDestroyNotify)att_free);
    c->win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_default_size(GTK_WINDOW(c->win), 680, 560);
    gtk_window_set_title(GTK_WINDOW(c->win), subject && *subject ? subject : "New Message");
    if (gtk_widget_get_visible(app.win)) gtk_window_set_transient_for(GTK_WINDOW(c->win), GTK_WINDOW(app.win));
    GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
    add_class(box, "page");
    g_object_set(box, "margin", 16, NULL);
    GtkWidget *grid = gtk_grid_new();
    gtk_grid_set_row_spacing(GTK_GRID(grid), 8);
    gtk_grid_set_column_spacing(GTK_GRID(grid), 12);
    char *from = app.acc ? (app.acc->name && *app.acc->name ? g_strdup_printf("%s <%s>", app.acc->name, app.acc->address) : g_strdup(app.acc->address)) : g_strdup("");
    field_row(grid, 0, "From", one_line(from, "name"));
    g_free(from);
    c->to = field_row(grid, 1, "To", gtk_entry_new());
    c->cc = field_row(grid, 2, "Cc", gtk_entry_new());
    c->subject = field_row(grid, 3, "Subject", gtk_entry_new());
    gtk_entry_set_text(GTK_ENTRY(c->to), to ? to : "");
    gtk_entry_set_text(GTK_ENTRY(c->cc), cc ? cc : "");
    gtk_entry_set_text(GTK_ENTRY(c->subject), subject ? subject : "");
    gtk_entry_set_placeholder_text(GTK_ENTRY(c->to), "someone@example.com, another@example.com");
    gtk_box_pack_start(GTK_BOX(box), grid, FALSE, FALSE, 0);
    c->body = gtk_text_view_new();
    gtk_text_view_set_wrap_mode(GTK_TEXT_VIEW(c->body), GTK_WRAP_WORD_CHAR);
    gtk_text_view_set_left_margin(GTK_TEXT_VIEW(c->body), 12);
    gtk_text_view_set_right_margin(GTK_TEXT_VIEW(c->body), 12);
    gtk_text_view_set_top_margin(GTK_TEXT_VIEW(c->body), 12);
    add_class(c->body, "body");
    if (body) gtk_text_buffer_set_text(gtk_text_view_get_buffer(GTK_TEXT_VIEW(c->body)), body, -1);
    GtkWidget *sc = gtk_scrolled_window_new(NULL, NULL);
    gtk_container_add(GTK_CONTAINER(sc), c->body);
    add_class(sc, "bodyframe");
    gtk_box_pack_start(GTK_BOX(box), sc, TRUE, TRUE, 0);
    c->atts_box = gtk_flow_box_new();
    gtk_flow_box_set_selection_mode(GTK_FLOW_BOX(c->atts_box), GTK_SELECTION_NONE);
    gtk_box_pack_start(GTK_BOX(box), c->atts_box, FALSE, FALSE, 0);
    c->msg = label("", NULL, 0);
    keep_hidden(c->msg);
    gtk_box_pack_start(GTK_BOX(box), c->msg, FALSE, FALSE, 0);
    GtkWidget *bar = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(bar), button("Attach…", "Add a file from Documents or Pictures", G_CALLBACK(on_attach), c, NULL), FALSE, FALSE, 0);
    c->send_btn = button("Send", "Send it (Ctrl+Enter)", G_CALLBACK(on_send), c, "primary");
    gtk_box_pack_end(GTK_BOX(bar), c->send_btn, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(box), bar, FALSE, FALSE, 0);
    gtk_container_add(GTK_CONTAINER(c->win), box);
    g_signal_connect(c->win, "destroy", G_CALLBACK(on_compose_destroy), c);
    g_signal_connect(c->win, "delete-event", G_CALLBACK(on_compose_delete), c);
    g_signal_connect(c->win, "key-press-event", G_CALLBACK(on_compose_key), c);
    g_ptr_array_add(app.composes, c);
    gtk_widget_show_all(c->win);
    if (!to || !*to) gtk_widget_grab_focus(c->to);
    else {
        gtk_widget_grab_focus(c->body);
        GtkTextIter start;
        gtk_text_buffer_get_start_iter(gtk_text_view_get_buffer(GTK_TEXT_VIEW(c->body)), &start);
        gtk_text_buffer_place_cursor(gtk_text_view_get_buffer(GTK_TEXT_VIEW(c->body)), &start);
    }
    state_changed();
    return c;
}

static void on_write(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    if (app.acc) compose_new(NULL, NULL, NULL, NULL);
}

/* mode: 0 reply, 1 reply all, 2 forward */
static void respond(int mode)
{
    if (!app.msg || !app.open || !app.acc) return;
    Message *m = app.msg;
    GPtrArray *who = m->reply_to->len ? m->reply_to : m->from;
    char *to = mode == 2 ? g_strdup("") : addresses_text(who);
    GString *cc = g_string_new(NULL);
    if (mode == 1) {  /* everyone else, but not me */
        GPtrArray *all[2] = {m->to, m->cc};
        for (int k = 0; k < 2; k++)
            for (guint i = 0; i < all[k]->len; i++) {
                Address *a = all[k]->pdata[i];
                if (!g_ascii_strcasecmp(a->addr, app.acc->address) || strstr(to, a->addr)) continue;
                char *f = address_format(a);
                g_string_append_printf(cc, "%s%s", cc->len ? ", " : "", f);
                g_free(f);
            }
    }
    char *subject = mime_reply_subject(m->subject, mode == 2), *quote;
    if (mode == 2) {
        char *from = addresses_text(m->from), *tl = addresses_text(m->to);
        quote = g_strdup_printf("\n\n——— Forwarded message ———\nFrom: %s\nTo: %s\nSubject: %s\n\n%s", from, tl, m->subject, m->text);
        g_free(from);
        g_free(tl);
    } else {
        quote = mime_quote(m);
    }
    Compose *c = compose_new(to, cc->str, subject, quote);
    if (mode != 2) {
        c->in_reply_to = g_strdup(m->message_id);
        c->references = g_strdup(m->references);
        c->reply_folder = g_strdup(app.folder);
        c->reply_uid = app.open->uid;
    } else {
        for (guint i = 0; i < m->attachments->len; i++) {  /* a forward keeps the attachments */
            Attachment *src = m->attachments->pdata[i], *a = g_new0(Attachment, 1);
            a->filename = g_strdup(src->filename);
            a->type = g_strdup(src->type);
            a->data = g_bytes_ref(src->data);
            g_ptr_array_add(c->atts, a);
        }
        fill_compose_atts(c);
    }
    g_free(to);
    g_string_free(cc, TRUE);
    g_free(subject);
    g_free(quote);
}

static void on_reply(GtkButton *b, gpointer d) { (void)b; respond(GPOINTER_TO_INT(d)); }

/* ---- the first start: an account ---- */

static void guess_servers(void)
{
    if (app.su_touched) return;
    Server in = {0}, out = {0};
    account_guess(gtk_entry_get_text(GTK_ENTRY(app.su_addr)), &in, &out);
    char port[8];
    gtk_entry_set_text(GTK_ENTRY(app.su_imap), in.host);
    g_snprintf(port, sizeof port, "%d", in.port);
    gtk_entry_set_text(GTK_ENTRY(app.su_imap_port), port);
    gtk_combo_box_set_active(GTK_COMBO_BOX(app.su_imap_sec), in.security);
    gtk_entry_set_text(GTK_ENTRY(app.su_smtp), out.host);
    g_snprintf(port, sizeof port, "%d", out.port);
    gtk_entry_set_text(GTK_ENTRY(app.su_smtp_port), port);
    gtk_combo_box_set_active(GTK_COMBO_BOX(app.su_smtp_sec), out.security);
    gtk_entry_set_text(GTK_ENTRY(app.su_user), gtk_entry_get_text(GTK_ENTRY(app.su_addr)));
    app.su_touched = FALSE;
    g_free(in.host);
    g_free(out.host);
}

static void on_addr_changed(GtkEditable *e, gpointer d)
{
    (void)e; (void)d;
    guess_servers();
}

static void on_server_edited(GtkEditable *e, gpointer d)
{
    (void)e; (void)d;
    if (gtk_widget_has_focus(GTK_WIDGET(e))) app.su_touched = TRUE;  /* typed by hand: no more guessing */
}

static void sign_in(void)
{
    const char *addr = gtk_entry_get_text(GTK_ENTRY(app.su_addr)), *pass = gtk_entry_get_text(GTK_ENTRY(app.su_pass));
    const char *problem = NULL;
    int ip = atoi(gtk_entry_get_text(GTK_ENTRY(app.su_imap_port))), sp = atoi(gtk_entry_get_text(GTK_ENTRY(app.su_smtp_port)));
    if (!address_valid(addr)) problem = "Type your whole email address, like ama@example.com.";
    else if (!*pass) problem = "Type your password (or your app password).";
    else if (ip <= 0 || ip > 65535 || sp <= 0 || sp > 65535) problem = "The port numbers aren't right: 993 and 465 or 587 are usual.";
    if (problem) {
        gtk_label_set_text(GTK_LABEL(app.su_msg), problem);
        gtk_widget_show(app.su_msg);
        return;
    }
    Job *j = g_new0(Job, 1);
    j->kind = J_SIGNIN;
    j->in = (Server){g_strdup(gtk_entry_get_text(GTK_ENTRY(app.su_imap))), ip, gtk_combo_box_get_active(GTK_COMBO_BOX(app.su_imap_sec)),
                     g_strdup(gtk_entry_get_text(GTK_ENTRY(app.su_user))), g_strdup(pass)};
    j->out = (Server){g_strdup(gtk_entry_get_text(GTK_ENTRY(app.su_smtp))), sp, gtk_combo_box_get_active(GTK_COMBO_BOX(app.su_smtp_sec)),
                      g_strdup(gtk_entry_get_text(GTK_ENTRY(app.su_user))), g_strdup(pass)};
    j->acc_id = g_strdup("new");
    gtk_label_set_text(GTK_LABEL(app.su_msg), "Signing in…");
    gtk_widget_show(app.su_msg);
    gtk_widget_set_sensitive(app.su_btn, FALSE);
    submit(j);
}

static void on_sign_in(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    sign_in();
}

static GtkWidget *sec_combo(void)
{
    GtkWidget *c = gtk_combo_box_text_new();
    gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(c), "TLS (encrypted)");
    gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(c), "STARTTLS (encrypted)");
    gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(c), "None (this computer only)");
    return c;
}

static GtkWidget *build_setup(void)
{
    GtkWidget *outer = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    add_class(outer, "page");
    GtkWidget *card = gtk_box_new(GTK_ORIENTATION_VERTICAL, 10);
    add_class(card, "card");
    gtk_widget_set_halign(card, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(card, GTK_ALIGN_CENTER);
    gtk_widget_set_size_request(card, 480, -1);
    gtk_widget_set_vexpand(card, TRUE);
    gtk_box_pack_start(GTK_BOX(card), label("Add your email", "heading", 0), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(card), label("Mail works with any email that offers IMAP: Gmail, Outlook, Yahoo, iCloud, your school or work. "
                                            "Gmail, Outlook and Yahoo want an app password: make one in your account's security settings.",
                                            "sub", 0), FALSE, FALSE, 0);
    GtkWidget *g = gtk_grid_new();
    gtk_grid_set_row_spacing(GTK_GRID(g), 8);
    gtk_grid_set_column_spacing(GTK_GRID(g), 12);
    app.su_name = field_row(g, 0, "Your name", gtk_entry_new());
    app.su_addr = field_row(g, 1, "Email", gtk_entry_new());
    app.su_pass = field_row(g, 2, "Password", gtk_entry_new());
    gtk_entry_set_visibility(GTK_ENTRY(app.su_pass), FALSE);
    gtk_entry_set_input_purpose(GTK_ENTRY(app.su_pass), GTK_INPUT_PURPOSE_PASSWORD);
    gtk_entry_set_placeholder_text(GTK_ENTRY(app.su_addr), "ama@example.com");
    g_signal_connect(app.su_addr, "changed", G_CALLBACK(on_addr_changed), NULL);
    g_signal_connect(app.su_pass, "activate", G_CALLBACK(on_sign_in), NULL);
    gtk_box_pack_start(GTK_BOX(card), g, FALSE, FALSE, 0);
    GtkWidget *exp = gtk_expander_new("Server settings (filled in for you)");
    GtkWidget *sg = gtk_grid_new();
    gtk_grid_set_row_spacing(GTK_GRID(sg), 8);
    gtk_grid_set_column_spacing(GTK_GRID(sg), 12);
    g_object_set(sg, "margin-top", 8, NULL);
    app.su_imap = field_row(sg, 0, "Incoming (IMAP)", gtk_entry_new());
    app.su_imap_port = field_row(sg, 1, "Port", gtk_entry_new());
    app.su_imap_sec = field_row(sg, 2, "Security", sec_combo());
    app.su_smtp = field_row(sg, 3, "Outgoing (SMTP)", gtk_entry_new());
    app.su_smtp_port = field_row(sg, 4, "Port", gtk_entry_new());
    app.su_smtp_sec = field_row(sg, 5, "Security", sec_combo());
    app.su_user = field_row(sg, 6, "User name", gtk_entry_new());
    GtkWidget *edits[] = {app.su_imap, app.su_imap_port, app.su_smtp, app.su_smtp_port, app.su_user};
    for (int i = 0; i < 5; i++) g_signal_connect(edits[i], "changed", G_CALLBACK(on_server_edited), NULL);
    gtk_container_add(GTK_CONTAINER(exp), sg);
    gtk_box_pack_start(GTK_BOX(card), exp, FALSE, FALSE, 0);
    app.su_msg = label("", NULL, 0);
    keep_hidden(app.su_msg);
    gtk_box_pack_start(GTK_BOX(card), app.su_msg, FALSE, FALSE, 0);
    app.su_btn = button("Sign In", "Check the address and password with the server", G_CALLBACK(on_sign_in), NULL, "primary");
    gtk_widget_set_halign(app.su_btn, GTK_ALIGN_END);
    gtk_box_pack_start(GTK_BOX(card), app.su_btn, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(card), label("Your password is kept encrypted on this computer, and only ever sent to your mail server "
                                            "over an encrypted connection.", "sub", 0), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(outer), card, TRUE, TRUE, 0);
    gtk_combo_box_set_active(GTK_COMBO_BOX(app.su_imap_sec), 0);
    gtk_combo_box_set_active(GTK_COMBO_BOX(app.su_smtp_sec), 1);
    return outer;
}

/* ---- the main view ---- */

static GtkWidget *scroller(GtkWidget *child)
{
    GtkWidget *s = gtk_scrolled_window_new(NULL, NULL);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(s), GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    gtk_container_add(GTK_CONTAINER(s), child);
    return s;
}

static GtkWidget *build_mail(void)
{
    GtkWidget *outer = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    add_class(outer, "page");
    GtkWidget *panes = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
    /* folders */
    app.folder_list = gtk_list_box_new();
    add_class(app.folder_list, "folders");
    g_signal_connect(app.folder_list, "row-activated", G_CALLBACK(on_folder_row), NULL);
    gtk_list_box_set_selection_mode(GTK_LIST_BOX(app.folder_list), GTK_SELECTION_NONE);
    GtkWidget *fs = scroller(app.folder_list);
    gtk_widget_set_size_request(fs, 200, -1);
    add_class(fs, "sidebar");
    gtk_box_pack_start(GTK_BOX(panes), fs, FALSE, FALSE, 0);
    /* messages */
    GtkWidget *mid = gtk_box_new(GTK_ORIENTATION_VERTICAL, 4);
    gtk_widget_set_size_request(mid, 340, -1);
    add_class(mid, "listpane");
    app.list_head = one_line("Mail", "title");
    g_object_set(app.list_head, "margin-start", 12, "margin-top", 12, "margin-bottom", 4, NULL);
    gtk_box_pack_start(GTK_BOX(mid), app.list_head, FALSE, FALSE, 0);
    app.list_empty = label("No messages here.", "sub", 0.5);
    g_object_set(app.list_empty, "margin", 24, NULL);
    keep_hidden(app.list_empty);
    gtk_box_pack_start(GTK_BOX(mid), app.list_empty, FALSE, FALSE, 0);
    app.msg_list = gtk_list_box_new();
    gtk_list_box_set_activate_on_single_click(GTK_LIST_BOX(app.msg_list), TRUE);
    g_signal_connect(app.msg_list, "row-activated", G_CALLBACK(on_msg_row), NULL);
    gtk_box_pack_start(GTK_BOX(mid), scroller(app.msg_list), TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(panes), mid, FALSE, FALSE, 0);
    /* reading */
    GtkWidget *right = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    app.read_empty = label("Pick a message to read it.", "sub", 0.5);
    gtk_widget_set_vexpand(app.read_empty, TRUE);
    gtk_box_pack_start(GTK_BOX(right), app.read_empty, TRUE, TRUE, 0);
    app.read_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    g_object_set(app.read_box, "margin", 20, NULL);
    GtkWidget *acts = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    gtk_box_pack_start(GTK_BOX(acts), button("Reply", "Reply (Ctrl+R)", G_CALLBACK(on_reply), GINT_TO_POINTER(0), NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(acts), button("Reply All", "Reply to everyone (Ctrl+Shift+R)", G_CALLBACK(on_reply), GINT_TO_POINTER(1), NULL), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(acts), button("Forward", "Forward (Ctrl+Shift+F)", G_CALLBACK(on_reply), GINT_TO_POINTER(2), NULL), FALSE, FALSE, 0);
    app.flag_btn = button("⬡ Flag", "Flag it to come back to", G_CALLBACK(on_flag), NULL, NULL);
    gtk_box_pack_start(GTK_BOX(acts), app.flag_btn, FALSE, FALSE, 0);
    gtk_box_pack_end(GTK_BOX(acts), button("Delete", "Move it to Trash (Delete)", G_CALLBACK(on_delete), NULL, "danger-text"), FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.read_box), acts, FALSE, FALSE, 0);
    app.read_subject = label("", "heading", 0);
    gtk_label_set_selectable(GTK_LABEL(app.read_subject), TRUE);
    gtk_widget_set_margin_top(app.read_subject, 10);
    app.read_from = label("", "name", 0);
    app.read_to = label("", "sub", 0);
    app.read_date = label("", "sub", 0);
    gtk_label_set_selectable(GTK_LABEL(app.read_from), TRUE);
    gtk_box_pack_start(GTK_BOX(app.read_box), app.read_subject, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.read_box), app.read_from, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.read_box), app.read_to, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(app.read_box), app.read_date, FALSE, FALSE, 0);
    app.html_note = label("Shown as plain text: pictures and anything else that would load from the internet are left out, so the "
                          "sender can't tell you opened it.", "note", 0);
    keep_hidden(app.html_note);
    gtk_box_pack_start(GTK_BOX(app.read_box), app.html_note, FALSE, FALSE, 0);
    app.read_atts = gtk_flow_box_new();
    gtk_flow_box_set_selection_mode(GTK_FLOW_BOX(app.read_atts), GTK_SELECTION_NONE);
    gtk_box_pack_start(GTK_BOX(app.read_box), app.read_atts, FALSE, FALSE, 0);
    app.read_text = label("", "body", 0);
    gtk_label_set_selectable(GTK_LABEL(app.read_text), TRUE);
    gtk_widget_set_valign(app.read_text, GTK_ALIGN_START);
    gtk_widget_set_margin_top(app.read_text, 12);
    gtk_box_pack_start(GTK_BOX(app.read_box), app.read_text, FALSE, FALSE, 0);
    GtkWidget *rs = scroller(app.read_box);
    gtk_widget_set_vexpand(rs, TRUE);
    gtk_box_pack_start(GTK_BOX(right), rs, TRUE, TRUE, 0);
    app.read_scroll = rs;
    keep_hidden(rs);
    gtk_box_pack_start(GTK_BOX(panes), right, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(outer), panes, TRUE, TRUE, 0);
    app.status = label("", "statusbar", 0);
    gtk_box_pack_start(GTK_BOX(outer), app.status, FALSE, FALSE, 0);
    return outer;
}

/* ---- look ---- */

static const char *CSS =
    "@define-color nexa_bg #0c1228; @define-color nexa_bg_deep #080c1e;"
    "@define-color nexa_surface #101732; @define-color nexa_surface_hi #222a46;"
    "@define-color nexa_hover #333a54; @define-color nexa_field #0f152d;"
    "@define-color nexa_border #363d5a; @define-color nexa_text #d9e0f6;"
    "@define-color nexa_text_strong #e6ecff; @define-color nexa_muted #8f9bc4;"
    "@define-color nexa_disabled #525b79; @define-color nexa_accent #4da3ff;"
    "@define-color nexa_accent_hover #6db4ff; @define-color nexa_on_accent #080c1e;"
    "@define-color nexa_selected #264779; @define-color nexa_selected_text #e6ecff;"
    "@define-color nexa_crit #ff5c7a; @define-color nexa_ok #5ee6a8; @define-color nexa_warn #ffb547;"
    "@define-color nexa_edge rgba(255, 255, 255, 0.13);"
    "* { font-family: Inter, sans-serif; }"
    "window, .page { background-color: @nexa_bg_deep; color: @nexa_text; }"
    ".sidebar, .folders { background-color: @nexa_surface; }"
    ".listpane { border-right: 1px solid @nexa_border; }"
    ".heading { font-size: 15pt; font-weight: 700; color: @nexa_text_strong; }"
    ".title { font-size: 12pt; font-weight: 700; color: @nexa_text_strong; }"
    ".name, .from { color: @nexa_text_strong; font-weight: 600; }"
    ".sub, .when, .preview { color: @nexa_muted; }"
    ".when { font-size: 8.5pt; }"
    ".subject { color: @nexa_text; }"
    "row.unread .subject, row.unread .from { font-weight: 700; color: @nexa_text_strong; }"
    ".note { color: @nexa_muted; font-size: 8.5pt; background-color: @nexa_surface; border-radius: 6px; padding: 8px 12px; }"
    ".count { color: @nexa_on_accent; background-color: @nexa_accent; border-radius: 10px; padding: 0 8px; font-size: 8.5pt; font-weight: 700; }"
    ".body { font-size: 10.5pt; color: @nexa_text; }"
    ".bad { color: @nexa_crit; }"
    ".statusbar { color: @nexa_muted; font-size: 8.5pt; padding: 4px 12px; border-top: 1px solid @nexa_border; background-color: @nexa_surface; }"
    ".statusbar.bad { color: @nexa_crit; }"
    ".card { background-color: @nexa_surface; border-radius: 16px; padding: 24px; }"
    "list { background-color: transparent; }"
    "row.folder { border-radius: 10px; margin: 0 8px 2px 8px; }"
    "row.folder:hover, row.msg:hover { background-color: @nexa_hover; }"
    "row.folder.chosen { background-color: @nexa_selected; }"
    "row.msg { border-bottom: 1px solid alpha(@nexa_border, 0.5); }"
    "row.msg:selected { background-color: @nexa_selected; }"
    "button { background-image: none; background-color: @nexa_surface_hi; color: @nexa_text_strong;"
    "  border: 1px solid @nexa_border; border-radius: 10px; padding: 4px 12px; box-shadow: none; text-shadow: none; }"
    "button:hover { background-color: @nexa_hover; }"
    "button:disabled { color: @nexa_disabled; background-color: @nexa_surface; }"
    "button.primary { background-color: @nexa_accent; color: @nexa_on_accent; font-weight: 700; border-color: @nexa_accent; }"
    "button.primary:hover { background-color: @nexa_accent_hover; }"
    "button.danger { background-color: @nexa_crit; color: @nexa_on_accent; font-weight: 700; }"
    "button.danger-text { color: @nexa_crit; }"
    "button.attachment { border-radius: 6px; font-size: 9pt; }"
    "entry { background-image: none; background-color: @nexa_field; color: @nexa_text; border: 1px solid @nexa_border;"
    "  border-radius: 6px; padding: 6px 8px; box-shadow: none; caret-color: @nexa_accent; }"
    "entry:focus { border-color: @nexa_accent; }"
    "textview, textview text { background-color: @nexa_field; color: @nexa_text; caret-color: @nexa_accent; }"
    ".bodyframe { border: 1px solid @nexa_border; border-radius: 6px; }"
    "combobox button { background-color: @nexa_field; }"
    "expander title { color: @nexa_muted; }"
    "scrollbar slider { background-color: alpha(@nexa_text, 0.25); border-radius: 6px; min-width: 6px; min-height: 6px; }"
    "headerbar { background-image: none; background-color: @nexa_surface; color: @nexa_text_strong;"
    "  border-bottom: 1px solid @nexa_border; box-shadow: inset 0 1px @nexa_edge; min-height: 42px; padding: 0 8px; }"
    "headerbar .title { font-weight: 600; font-size: 10pt; }"
    "headerbar button.titlebutton { background-color: transparent; color: @nexa_text; border: none; border-radius: 50%;"
    "  min-width: 26px; min-height: 26px; padding: 0; }"
    "headerbar button.titlebutton:hover { background-color: @nexa_hover; }"
    "headerbar button.titlebutton.close:hover { background-color: @nexa_crit; color: @nexa_on_accent; }";

static GtkCssProvider *live_theme;

static void reload_theme(void)
{
    char *p = g_build_filename(g_get_user_config_dir(), "gtk-3.0", "gtk.css", NULL);
    gtk_css_provider_load_from_path(live_theme, p, NULL);
    g_free(p);
}

static void on_theme(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent ev, gpointer d)
{
    (void)m; (void)f; (void)o; (void)d;
    if (ev != G_FILE_MONITOR_EVENT_DELETED) reload_theme();
}

static void follow_theme(void)
{
    live_theme = gtk_css_provider_new();
    reload_theme();
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(live_theme), GTK_STYLE_PROVIDER_PRIORITY_USER + 1);
    char *p = g_build_filename(g_get_user_config_dir(), "gtk-3.0", "gtk.css", NULL);
    GFile *f = g_file_new_for_path(p);
    GFileMonitor *m = g_file_monitor_file(f, G_FILE_MONITOR_NONE, NULL, NULL);
    if (m) g_signal_connect(m, "changed", G_CALLBACK(on_theme), NULL);
    g_object_unref(f);
    g_free(p);
}

static gboolean wm_run(const char *const *argv)
{
    char *out = NULL;
    int status = 0;
    gboolean ok = g_spawn_sync(NULL, (char **)argv, NULL, G_SPAWN_SEARCH_PATH | G_SPAWN_STDERR_TO_DEV_NULL, NULL, NULL, &out, NULL,
                               &status, NULL) &&
                  g_spawn_check_wait_status(status, NULL) && out && (strstr(out, "ok") || strstr(out, "\"success\": true"));
    g_free(out);
    return ok;
}

static void on_minimize(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    char sel[64];
    gboolean done = FALSE;
    if (g_getenv("HYPRLAND_INSTANCE_SIGNATURE")) {
        g_snprintf(sel, sizeof sel, "special:minimized,pid:%d", (int)getpid());
        const char *argv[] = {"hyprctl", "dispatch", "movetoworkspacesilent", sel, NULL};
        done = wm_run(argv);
    } else if (g_getenv("SWAYSOCK")) {
        g_snprintf(sel, sizeof sel, "[pid=%d] move scratchpad", (int)getpid());
        const char *argv[] = {"swaymsg", sel, NULL};
        done = wm_run(argv);
    }
    if (!done) gtk_window_iconify(GTK_WINDOW(app.win));
}

static void on_close(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    gtk_window_close(GTK_WINDOW(app.win));
}

static GtkWidget *title_button(const char *icon, const char *tip, const char *role, GCallback cb)
{
    GtkWidget *b = gtk_button_new_from_icon_name(icon, GTK_ICON_SIZE_MENU);
    gtk_widget_set_tooltip_text(b, tip);
    gtk_widget_set_can_focus(b, FALSE);
    gtk_widget_set_valign(b, GTK_ALIGN_CENTER);
    add_class(b, "titlebutton");
    add_class(b, role);
    g_signal_connect(b, "clicked", cb, NULL);
    return b;
}

static void on_refresh(GtkButton *b, gpointer d)
{
    (void)b; (void)d;
    sync_now();
}

/* ---- for tests ---- */

static void json_str(GString *s, const char *v)
{
    g_string_append_c(s, '"');
    for (const char *c = v ? v : ""; *c; c++) {
        if (*c == '"' || *c == '\\') g_string_append_printf(s, "\\%c", *c);
        else if ((unsigned char)*c < 32) g_string_append_printf(s, "\\u%04x", *c);
        else g_string_append_c(s, *c);
    }
    g_string_append_c(s, '"');
}

static gboolean write_state(gpointer d)
{
    (void)d;
    app.state_id = 0;
    GString *s = g_string_new("{\"page\": ");
    json_str(s, gtk_stack_get_visible_child_name(GTK_STACK(app.stack)));
    g_string_append_printf(s, ", \"busy\": %d, \"syncs\": %d, \"opens\": %d, \"sends\": %d, \"accounts\": %u, \"main_shown\": %d, "
                              "\"signals\": %d, \"title\": ", app.busy, app.syncs, app.opens, app.sends, app.accounts->len,
                           gtk_widget_get_visible(app.win), app.signals_sent);
    json_str(s, gtk_window_get_title(GTK_WINDOW(app.win)));
    g_string_append(s, ", \"setup_msg\": ");
    json_str(s, gtk_widget_get_visible(app.su_msg) ? gtk_label_get_text(GTK_LABEL(app.su_msg)) : "");
    g_string_append(s, ", \"status\": ");
    json_str(s, gtk_label_get_text(GTK_LABEL(app.status)));
    g_string_append(s, ", \"folder\": ");
    Folder *f = folder_named(app.folder);
    json_str(s, f ? folder_title(f) : "");
    g_string_append(s, ", \"folders\": [");
    for (guint i = 0; app.folders && i < app.folders->len; i++) {
        Folder *x = app.folders->pdata[i];
        if (i) g_string_append(s, ", ");
        g_string_append(s, "{\"name\": ");
        json_str(s, folder_title(x));
        g_string_append_printf(s, ", \"unseen\": %u}", GPOINTER_TO_UINT(g_hash_table_lookup(app.unseen, x->raw)));
    }
    g_string_append(s, "], \"list\": [");
    GList *rows = gtk_container_get_children(GTK_CONTAINER(app.msg_list));
    int first = 1;
    for (GList *r = rows; r; r = r->next) {
        Entry *e = g_object_get_data(G_OBJECT(r->data), "entry");
        if (!e) continue;
        g_string_append(s, first ? "{\"subject\": " : ", {\"subject\": ");
        first = 0;
        json_str(s, e->subject);
        g_string_append(s, ", \"from\": ");
        json_str(s, e->from);
        g_string_append_printf(s, ", \"seen\": %d, \"flagged\": %d, \"answered\": %d}", e->seen, e->flagged, e->answered);
    }
    g_list_free(rows);
    g_string_append(s, "], \"reading\": ");
    if (app.open) {
        g_string_append(s, "{\"subject\": ");
        json_str(s, gtk_label_get_text(GTK_LABEL(app.read_subject)));
        g_string_append(s, ", \"text\": ");
        json_str(s, gtk_label_get_text(GTK_LABEL(app.read_text)));
        g_string_append(s, ", \"to\": ");
        json_str(s, gtk_label_get_text(GTK_LABEL(app.read_to)));
        g_string_append_printf(s, ", \"html\": %d, \"attachments\": %u}", app.msg ? app.msg->was_html : 0, app.msg ? app.msg->attachments->len : 0);
    } else {
        g_string_append(s, "null");
    }
    g_string_append_printf(s, ", \"composes\": %u", app.composes->len);
    if (app.composes->len) {
        Compose *c = app.composes->pdata[app.composes->len - 1];
        char *body = text_of(c->body);
        g_string_append(s, ", \"compose\": {\"to\": ");
        json_str(s, gtk_entry_get_text(GTK_ENTRY(c->to)));
        g_string_append(s, ", \"cc\": ");
        json_str(s, gtk_entry_get_text(GTK_ENTRY(c->cc)));
        g_string_append(s, ", \"subject\": ");
        json_str(s, gtk_entry_get_text(GTK_ENTRY(c->subject)));
        g_string_append(s, ", \"body\": ");
        json_str(s, body);
        g_string_append(s, ", \"msg\": ");
        json_str(s, gtk_widget_get_visible(c->msg) ? gtk_label_get_text(GTK_LABEL(c->msg)) : "");
        g_string_append_printf(s, ", \"attachments\": %u}", c->atts->len);
        g_free(body);
    }
    g_string_append(s, "}\n");
    char *path = g_build_filename(app.test_dir, "state.json", NULL), *tmp = g_strconcat(path, ".tmp", NULL);
    if (g_file_set_contents(tmp, s->str, (gssize)s->len, NULL)) g_rename(tmp, path);
    g_free(path);
    g_free(tmp);
    g_string_free(s, TRUE);
    return G_SOURCE_REMOVE;
}

static void state_changed(void)
{
    if (app.test_dir && !app.state_id) app.state_id = g_timeout_add(30, write_state, NULL);
}

static void test_command(char *line)
{
    g_strchomp(line);
    char **w = g_strsplit(line, " ", 2);
    const char *c = w[0], *a = w[1] ? w[1] : "";
    Compose *cp = app.composes->len ? app.composes->pdata[app.composes->len - 1] : NULL;
    if (!strcmp(c, "setup")) {  /* setup name|address|password */
        char **f = g_strsplit(a, "|", 3);
        if (g_strv_length(f) == 3) {
            gtk_entry_set_text(GTK_ENTRY(app.su_name), f[0]);
            gtk_entry_set_text(GTK_ENTRY(app.su_addr), f[1]);
            gtk_entry_set_text(GTK_ENTRY(app.su_pass), f[2]);
        }
        g_strfreev(f);
    } else if (!strcmp(c, "server")) {  /* server imap|port|sec|smtp|port|sec */
        char **f = g_strsplit(a, "|", 6);
        if (g_strv_length(f) == 6) {
            app.su_touched = TRUE;
            gtk_entry_set_text(GTK_ENTRY(app.su_imap), f[0]);
            gtk_entry_set_text(GTK_ENTRY(app.su_imap_port), f[1]);
            gtk_combo_box_set_active(GTK_COMBO_BOX(app.su_imap_sec), atoi(f[2]));
            gtk_entry_set_text(GTK_ENTRY(app.su_smtp), f[3]);
            gtk_entry_set_text(GTK_ENTRY(app.su_smtp_port), f[4]);
            gtk_combo_box_set_active(GTK_COMBO_BOX(app.su_smtp_sec), atoi(f[5]));
        }
        g_strfreev(f);
    } else if (!strcmp(c, "signin")) sign_in();
    else if (!strcmp(c, "folder")) {
        for (guint i = 0; app.folders && i < app.folders->len; i++)
            if (!strcmp(folder_title(app.folders->pdata[i]), a)) open_folder(((Folder *)app.folders->pdata[i])->raw);
    } else if (!strcmp(c, "open")) {
        GList *rows = gtk_container_get_children(GTK_CONTAINER(app.msg_list));
        GtkWidget *r = g_list_nth_data(rows, (guint)atoi(a));
        if (r) on_msg_row(NULL, GTK_LIST_BOX_ROW(r), NULL);
        g_list_free(rows);
    } else if (!strcmp(c, "reply")) respond(atoi(a));
    else if (!strcmp(c, "write")) on_write(NULL, NULL);
    else if (!strcmp(c, "to") && cp) gtk_entry_set_text(GTK_ENTRY(cp->to), a);
    else if (!strcmp(c, "cc") && cp) gtk_entry_set_text(GTK_ENTRY(cp->cc), a);
    else if (!strcmp(c, "subject") && cp) gtk_entry_set_text(GTK_ENTRY(cp->subject), a);
    else if (!strcmp(c, "body") && cp) {
        char *t = g_strcompress(a);
        gtk_text_buffer_set_text(gtk_text_view_get_buffer(GTK_TEXT_VIEW(cp->body)), t, -1);
        g_free(t);
    } else if (!strcmp(c, "attach") && cp) attach_file(cp, a);
    else if (!strcmp(c, "send") && cp) send_compose(cp);
    else if (!strcmp(c, "close") && cp) gtk_widget_destroy(cp->win);
    else if (!strcmp(c, "flag")) on_flag(NULL, NULL);
    else if (!strcmp(c, "delete")) on_delete(NULL, NULL);
    else if (!strcmp(c, "search")) gtk_entry_set_text(GTK_ENTRY(app.search), a);
    else if (!strcmp(c, "search-server")) on_search_activate(GTK_ENTRY(app.search), NULL);
    else if (!strcmp(c, "save") && app.msg && (guint)atoi(a) < app.msg->attachments->len) on_save_att(NULL, app.msg->attachments->pdata[atoi(a)]);
    else if (!strcmp(c, "refresh")) sync_now();
    g_strfreev(w);
    state_changed();
}

static gboolean on_test_input(GIOChannel *ch, GIOCondition cond, gpointer d)
{
    (void)cond; (void)d;
    char *line = NULL;
    while (g_io_channel_read_line(ch, &line, NULL, NULL, NULL) == G_IO_STATUS_NORMAL && line) {
        test_command(line);
        g_free(line);
        line = NULL;
    }
    return G_SOURCE_CONTINUE;
}

static void test_setup(void)
{
    char *fifo = g_build_filename(app.test_dir, "cmd", NULL);
    int fd = open(fifo, O_RDWR | O_NONBLOCK | O_CLOEXEC);
    g_free(fifo);
    if (fd < 0) return;
    GIOChannel *ch = g_io_channel_unix_new(fd);
    g_io_channel_set_flags(ch, G_IO_FLAG_NONBLOCK, NULL);
    g_io_add_watch(ch, G_IO_IN, on_test_input, NULL);
}

/* ---- Signals: new mail, and where a click on one goes ---- */

static void update_title(void)
{
    Folder *in = folder_by_role(FOLDER_INBOX);
    guint n = in ? GPOINTER_TO_UINT(g_hash_table_lookup(app.unseen, in->raw)) : 0;
    char *t = n ? g_strdup_printf("Mail · %u unread", n) : g_strdup("Mail");
    if (g_strcmp0(gtk_window_get_title(GTK_WINDOW(app.win)), t)) gtk_window_set_title(GTK_WINDOW(app.win), t);
    g_free(t);
}

/* Open the Inbox, and the message `uid` if it's there. */
static void show_inbox_message(guint32 uid)
{
    Folder *in = folder_by_role(FOLDER_INBOX);
    if (in && g_strcmp0(app.folder, in->raw)) open_folder(in->raw);
    gtk_entry_set_text(GTK_ENTRY(app.search), "");
    gtk_widget_show(app.win);
    gtk_window_present(GTK_WINDOW(app.win));
    if (!uid) return;
    GList *rows = gtk_container_get_children(GTK_CONTAINER(app.msg_list));
    for (GList *r = rows; r; r = r->next) {
        Entry *e = g_object_get_data(G_OBJECT(r->data), "entry");
        if (e && e->uid == uid) {
            gtk_list_box_select_row(GTK_LIST_BOX(app.msg_list), GTK_LIST_BOX_ROW(r->data));
            on_msg_row(NULL, GTK_LIST_BOX_ROW(r->data), NULL);
            break;
        }
    }
    g_list_free(rows);
}

static void on_signal_action(GDBusConnection *c, const char *sender, const char *path, const char *iface, const char *signal,
                             GVariant *params, gpointer d)
{
    (void)c; (void)sender; (void)path; (void)iface; (void)signal; (void)d;
    guint32 id = 0;
    const char *key = NULL;
    g_variant_get(params, "(u&s)", &id, &key);
    gpointer uid;
    if (!app.notes || !g_hash_table_lookup_extended(app.notes, GUINT_TO_POINTER(id), NULL, &uid)) return;  /* not ours */
    g_hash_table_remove(app.notes, GUINT_TO_POINTER(id));
    show_inbox_message(GPOINTER_TO_UINT(uid));
    state_changed();
}

static void on_bus(GObject *src, GAsyncResult *res, gpointer d)
{
    (void)src; (void)d;
    app.bus = g_bus_get_finish(res, NULL);  /* no session bus: no Signals, nothing else changes */
    if (!app.bus) return;
    g_dbus_connection_signal_subscribe(app.bus, NULL, "org.freedesktop.Notifications", "ActionInvoked", "/org/freedesktop/Notifications",
                                       NULL, G_DBUS_SIGNAL_FLAGS_NONE, on_signal_action, NULL, NULL);
}

static void signals_init(void)
{
    app.notes = g_hash_table_new(NULL, NULL);
    g_bus_get(G_BUS_TYPE_SESSION, NULL, on_bus, NULL);
}

static void on_notified(GObject *src, GAsyncResult *res, gpointer uid)
{
    GVariant *r = g_dbus_connection_call_finish(G_DBUS_CONNECTION(src), res, NULL);
    if (!r) return;
    guint32 id = 0;
    g_variant_get(r, "(u)", &id);
    g_variant_unref(r);
    if (id) g_hash_table_replace(app.notes, GUINT_TO_POINTER(id), uid);
    app.signals_sent++;
    state_changed();
}

/* One Signal. Text from an email is plain (escaped): Signals may read markup. */
static void signal_new(const char *summary, const char *body, guint32 uid)
{
    if (!app.bus) return;
    char *s = g_markup_escape_text(summary, -1), *b = g_markup_escape_text(body ? body : "", -1);
    GVariantBuilder actions, hints;
    g_variant_builder_init(&actions, G_VARIANT_TYPE("as"));
    g_variant_builder_add(&actions, "s", "default");
    g_variant_builder_add(&actions, "s", uid ? "Read" : "Open Inbox");
    g_variant_builder_init(&hints, G_VARIANT_TYPE("a{sv}"));
    g_variant_builder_add(&hints, "{sv}", "desktop-entry", g_variant_new_string("nexa-" APP_ID));
    g_variant_builder_add(&hints, "{sv}", "category", g_variant_new_string("email.arrived"));
    g_dbus_connection_call(app.bus, "org.freedesktop.Notifications", "/org/freedesktop/Notifications", "org.freedesktop.Notifications",
                           "Notify", g_variant_new("(susssasa{sv}i)", "Mail", 0u, "nexa-" APP_ID, s, b, &actions, &hints, -1),
                           G_VARIANT_TYPE("(u)"), G_DBUS_CALL_FLAGS_NONE, 5000, NULL, on_notified, GUINT_TO_POINTER(uid));
    g_free(s);
    g_free(b);
}

static gboolean looking_at_mail(void)
{
    return gtk_widget_get_visible(app.win) && gtk_window_is_active(GTK_WINDOW(app.win));
}

/* New unread messages in the Inbox: one Signal each (a few), or one for all. */
static void announce_new(GArray *uids)
{
    if (!uids->len || looking_at_mail()) return;
    if (uids->len <= 3) {
        for (guint i = 0; i < uids->len; i++) {
            Entry *e = box_find(app.box, g_array_index(uids, guint32, i));
            if (!e || e->seen) continue;
            char *who = g_strdup(e->from && *e->from ? e->from : "Someone"), *lt = strchr(who, '<');
            if (lt && lt > who) *lt = 0;
            signal_new(g_strstrip(who), e->subject && *e->subject ? e->subject : "(no subject)", e->uid);
            g_free(who);
        }
        return;
    }
    GString *b = g_string_new(NULL);
    for (guint i = 0; i < uids->len && i < 3; i++) {
        Entry *e = box_find(app.box, g_array_index(uids, guint32, i));
        if (e) g_string_append_printf(b, "%s%s", b->len ? "\n" : "", e->subject && *e->subject ? e->subject : "(no subject)");
    }
    char *t = g_strdup_printf("%u new messages", uids->len);
    signal_new(t, b->str, 0);
    g_free(t);
    g_string_free(b, TRUE);
}

/* Looking at another folder: the Inbox's count went up. */
static void announce_inbox(guint before, guint now)
{
    if (now <= before || looking_at_mail()) return;
    char *t = now - before == 1 ? g_strdup("A new message in your Inbox") : g_strdup_printf("%u new messages in your Inbox", now - before);
    signal_new(t, "", 0);
    g_free(t);
}

/* mailto: or --write: the new message, once there's an account. */
static void open_pending_compose(void)
{
    if (!app.pending_compose || !app.acc) return;
    app.pending_compose = FALSE;
    char *to = g_strdup(app.pending_to ? app.pending_to : "");
    if (app.pending_who && *app.pending_who) {  /* their address from mail they sent */
        Box *b = box_load(app.acc->id, "INBOX");
        GPtrArray *senders = g_ptr_array_new();
        for (guint i = 0; b && i < b->entries->len; i++) g_ptr_array_add(senders, ((Entry *)b->entries->pdata[i])->from);
        char *found = address_pick(senders, app.pending_who);
        g_free(to);
        to = found ? found : g_strdup(app.pending_who);  /* not found: what was asked, to correct */
        g_ptr_array_free(senders, TRUE);
        box_free(b);
    }
    compose_new(to, app.pending_cc, app.pending_subject, app.pending_body);
    g_free(to);
}

/* ---- keys and start ---- */

static gboolean on_key(GtkWidget *w, GdkEventKey *e, gpointer d)
{
    (void)w; (void)d;
    gboolean ctrl = (e->state & GDK_CONTROL_MASK) != 0, shift = (e->state & GDK_SHIFT_MASK) != 0;
    guint k = gdk_keyval_to_lower(e->keyval);
    if (ctrl && (k == GDK_KEY_q || k == GDK_KEY_w)) {
        gtk_window_close(GTK_WINDOW(app.win));
        return TRUE;
    }
    if (!app.acc) return FALSE;
    if (ctrl && k == GDK_KEY_n) { on_write(NULL, NULL); return TRUE; }
    if (ctrl && k == GDK_KEY_r) { respond(shift ? 1 : 0); return TRUE; }
    if (ctrl && shift && k == GDK_KEY_f) { respond(2); return TRUE; }
    if (ctrl && k == GDK_KEY_f) { gtk_widget_grab_focus(app.search); return TRUE; }
    if (k == GDK_KEY_F5) { sync_now(); return TRUE; }
    if (k == GDK_KEY_Delete && !gtk_widget_has_focus(app.search)) { on_delete(NULL, NULL); return TRUE; }
    return FALSE;
}

int main(int argc, char **argv)
{
    curl_global_init(CURL_GLOBAL_DEFAULT);
    gtk_init(&argc, &argv);
    g_set_prgname(APP_ID);
    g_set_application_name("Mail");
    app.test_dir = g_strdup(g_getenv("MAIL_TEST"));
    app.unseen = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    app.composes = g_ptr_array_new();
    app.jobs = g_async_queue_new();
    app.accounts = accounts_load();
    for (int i = 1; i < argc; i++) {
        if (!g_ascii_strncasecmp(argv[i], "mailto:", 7)) {
            app.pending_compose = mailto_parse(argv[i], &app.pending_to, &app.pending_cc, &app.pending_subject, &app.pending_body);
        } else if (!strcmp(argv[i], "--write") && i + 1 < argc) {
            app.pending_compose = TRUE;
            app.pending_who = g_strndup(argv[++i], 200);
        } else if (!strcmp(argv[i], "--search") && i + 1 < argc) {
            app.pending_search = g_strndup(argv[++i], 200);
        }
    }
    app.compose_only = app.pending_compose && app.accounts->len;  /* no account yet: the window, to add one */
    g_thread_unref(g_thread_new("mail", worker, NULL));
    signals_init();

    GtkCssProvider *css = gtk_css_provider_new();
    gtk_css_provider_load_from_data(css, CSS, -1, NULL);
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(css), GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    follow_theme();

    app.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_default_size(GTK_WINDOW(app.win), 1180, 720);
    gtk_window_set_icon_name(GTK_WINDOW(app.win), APP_ID);
    GtkWidget *bar = gtk_header_bar_new();
    gtk_header_bar_set_show_close_button(GTK_HEADER_BAR(bar), FALSE);
    app.write_btn = button("Write", "A new message (Ctrl+N)", G_CALLBACK(on_write), NULL, "primary");
    app.refresh_btn = button("Check", "Check for new mail now (F5)", G_CALLBACK(on_refresh), NULL, NULL);
    gtk_header_bar_pack_start(GTK_HEADER_BAR(bar), app.write_btn);
    gtk_header_bar_pack_start(GTK_HEADER_BAR(bar), app.refresh_btn);
    gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), title_button("window-close-symbolic", "Close (Ctrl+Q)", "close", G_CALLBACK(on_close)));
    gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), title_button("window-minimize-symbolic", "Minimize", "minimize", G_CALLBACK(on_minimize)));
    app.search = gtk_search_entry_new();
    gtk_entry_set_placeholder_text(GTK_ENTRY(app.search), "Search mail");
    gtk_widget_set_tooltip_text(app.search, "Search this folder (Ctrl+F); Enter searches every message on the server");
    gtk_entry_set_width_chars(GTK_ENTRY(app.search), 26);
    g_signal_connect(app.search, "search-changed", G_CALLBACK(on_search_changed), NULL);
    g_signal_connect(app.search, "activate", G_CALLBACK(on_search_activate), NULL);
    gtk_header_bar_pack_end(GTK_HEADER_BAR(bar), app.search);
    gtk_window_set_titlebar(GTK_WINDOW(app.win), bar);
    gtk_window_set_title(GTK_WINDOW(app.win), "Mail");
    app.stack = gtk_stack_new();
    gtk_stack_add_named(GTK_STACK(app.stack), build_setup(), "setup");
    gtk_stack_add_named(GTK_STACK(app.stack), build_mail(), "mail");
    gtk_container_add(GTK_CONTAINER(app.win), app.stack);
    g_signal_connect(app.win, "key-press-event", G_CALLBACK(on_key), NULL);
    g_signal_connect(app.win, "destroy", G_CALLBACK(gtk_main_quit), NULL);
    gtk_widget_show_all(gtk_bin_get_child(GTK_BIN(app.win)));
    if (!app.compose_only) gtk_widget_show_all(app.win);
    if (app.test_dir) test_setup();
    if (app.accounts->len) {
        app.acc = app.accounts->pdata[0];
        if (!app.acc->in.pass) set_status("Mail can't find this account's password. Remove and add the account again.", TRUE);
        app.folder = g_strdup("INBOX");
        app.box = box_load(app.acc->id, app.folder);
        gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "mail");
        fill_list();
        if (app.compose_only) open_pending_compose();
        else sync_now();
    } else {
        gtk_stack_set_visible_child_name(GTK_STACK(app.stack), "setup");
        gtk_widget_set_sensitive(app.write_btn, FALSE);
        gtk_widget_set_sensitive(app.refresh_btn, FALSE);
    }
    if (!app.compose_only) app.check_id = g_timeout_add_seconds(CHECK_EVERY, every_few_minutes, NULL);
    state_changed();
    gtk_main();
    return 0;
}
