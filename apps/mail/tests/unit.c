/* Unit tests for Mail's engine (make test): reading messages (encoded
 * words, charsets, nested multipart, HTML, attachments, damage), writing
 * them (and that no header can be slipped in), addresses and dates, the
 * IMAP parser (literals, lists, garbage), folder names, passwords sealed
 * and tampered with, guessing servers, and the offline copy. */
#include <glib/gstdio.h>
#include <stdio.h>
#include <string.h>

#include "../src/account.h"
#include "../src/imap.h"
#include "../src/mime.h"

static int fails, checks;

static void check(int ok, const char *what)
{
    checks++;
    if (!ok) fails++;
    printf("%s %s\n", ok ? "PASS" : "FAIL", what);
}

static const char *MSG =
    "Return-Path: <x@y>\r\n"
    "From: =?UTF-8?B?QW3DoSBCw7Rh?= <Ama@Example.COM>\r\n"
    "To: \"Kofi, Mensah\" <kofi@example.com>, ben@example.org, undisclosed-recipients:;\r\n"
    "Subject: =?ISO-8859-1?Q?Caf=E9_tonight?=\r\n"
    " =?UTF-8?Q?_=E2=98=95?=\r\n"
    "Date: Thu, 5 Nov 2026 18:04:05 +0100\r\n"
    "Message-ID: <abc@example.com>\r\n"
    "MIME-Version: 1.0\r\n"
    "Content-Type: multipart/mixed; boundary=\"outer\"\r\n"
    "\r\n"
    "preamble\r\n"
    "--outer\r\n"
    "Content-Type: multipart/alternative; boundary=inner\r\n"
    "\r\n"
    "--inner\r\n"
    "Content-Type: text/plain; charset=utf-8\r\n"
    "Content-Transfer-Encoding: quoted-printable\r\n"
    "\r\n"
    "Hello Kofi,=0D=0Asee you at the caf=C3=A9 =\r\n"
    "at 8.\r\n"
    "--inner\r\n"
    "Content-Type: text/html\r\n"
    "\r\n"
    "<p>Hello</p>\r\n"
    "--inner--\r\n"
    "--outer\r\n"
    "Content-Type: application/pdf; name=\"menu.pdf\"\r\n"
    "Content-Disposition: attachment; filename=\"../../.bashrc\"\r\n"
    "Content-Transfer-Encoding: base64\r\n"
    "\r\n"
    "JVBERi0xLjQK\r\n"
    "--outer\r\n"
    "Content-Type: image/png\r\n"
    "Content-Disposition: attachment; filename*=utf-8''caf%C3%A9%20photo.png\r\n"
    "Content-Transfer-Encoding: base64\r\n"
    "\r\n"
    "iVBORw0K\r\n"
    "--outer--\r\n";

static const char *HTML_MSG =
    "From: news@shop.example\r\nSubject: Deals\r\nContent-Type: text/html; charset=windows-1252\r\n\r\n"
    "<html><head><title>x</title><style>p{color:red}</style><script>alert(1)</script></head>"
    "<body><p>Big&nbsp;sale &amp; more &#8364;5 &#x263A; caf\xe9</p><img src=\"https://track.example/p.gif\" alt=\"\">"
    "<img src=\"x.png\" alt=\"Logo\"><a href=\"https://shop.example/deal\">See deals</a><br>"
    "<ul><li>one</li><li>two</li></ul><!-- hidden --></body></html>";

int main(void)
{
    /* ---- reading ---- */
    Message *m = mime_parse(MSG, strlen(MSG));
    Address *from = m->from->len ? m->from->pdata[0] : NULL;
    check(from && !strcmp(from->name, "Amá Bôa") && !strcmp(from->addr, "ama@example.com"), "From: an encoded name, the address in lower case");
    check(m->to->len == 2 && !strcmp(((Address *)m->to->pdata[0])->name, "Kofi, Mensah") && !strcmp(((Address *)m->to->pdata[1])->addr, "ben@example.org"),
          "To: a quoted name with a comma, a bare address, an empty group");
    check(!strcmp(m->subject, "Café tonight ☕"), "Subject: two encoded words in two charsets, joined");
    check(m->date == 1793898245LL, "Date with its time zone");
    check(!strcmp(m->text, "Hello Kofi,\nsee you at the café at 8.") && !m->was_html, "the plain text part, quoted-printable, soft line break");
    check(m->attachments->len == 2, "two attachments");
    Attachment *a0 = m->attachments->len > 0 ? m->attachments->pdata[0] : NULL, *a1 = m->attachments->len > 1 ? m->attachments->pdata[1] : NULL;
    check(a0 && !strcmp(a0->filename, "bashrc") && !strcmp(a0->type, "application/pdf") && g_bytes_get_size(a0->data) == 9 &&
              !memcmp(g_bytes_get_data(a0->data, NULL), "%PDF-1.4\n", 9),
          "an attachment's name can't lead outside, or be hidden; base64 decoded");
    check(a1 && !strcmp(a1->filename, "café photo.png"), "a UTF-8 file name (RFC 2231)");
    message_free(m);

    m = mime_parse(HTML_MSG, strlen(HTML_MSG));
    check(m->was_html && strstr(m->text, "Big sale & more €5 ☺ café") && !strstr(m->text, "alert") && !strstr(m->text, "color") &&
              !strstr(m->text, "track.example") && strstr(m->text, "[Logo]") && strstr(m->text, "See deals <https://shop.example/deal>") &&
              strstr(m->text, "• one") && !strstr(m->text, "hidden"),
          "HTML as text: entities, no script, no style, no tracking picture, links kept");
    message_free(m);

    const char *plain8 = "Subject: hi\nContent-Type: text/plain; charset=iso-8859-1\n\nna\xefve\n";
    m = mime_parse(plain8, strlen(plain8));
    check(!strcmp(m->text, "naïve") && !strcmp(m->subject, "hi"), "LF-only lines and a Latin-1 body");
    message_free(m);
    m = mime_parse("", 0);
    check(m && !*m->text && !m->from->len, "an empty message: no crash");
    message_free(m);

    int ok = 1;
    for (gsize n = 0; n <= strlen(MSG); n += 7) {
        Message *x = mime_parse(MSG, n);
        ok &= x != NULL && g_utf8_validate(x->text, -1, NULL) && g_utf8_validate(x->subject, -1, NULL);
        message_free(x);
    }
    check(ok, "every cut-short copy read safely");
    guint32 seed = 7;
    ok = 1;
    for (int r = 0; r < 3000; r++) {
        char *copy = g_strdup(MSG);
        size_t n = strlen(copy);
        for (int k = 0; k < 1 + r % 10; k++) {
            seed = seed * 1103515245 + 12345;
            copy[(seed >> 8) % n] = (char)(seed >> 20);
        }
        Message *x = mime_parse(copy, n);
        ok &= g_utf8_validate(x->text, -1, NULL);
        message_free(x);
        g_free(copy);
    }
    check(ok, "3000 damaged copies read safely");
    GString *deep = g_string_new("Content-Type: multipart/mixed; boundary=b\r\n\r\n");
    for (int i = 0; i < 50; i++) g_string_append(deep, "--b\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n");
    m = mime_parse(deep->str, deep->len);
    check(m != NULL, "50 nested parts: stopped at 10, no crash");
    message_free(m);
    g_string_free(deep, TRUE);

    check(mime_date("Fri, 06 Nov 2026 08:00 GMT") == 1793952000LL && mime_date("garbage") == 0 && mime_date("32 Jan 2026 10:00:00") == 0,
          "dates: without seconds, broken ones");
    check(address_valid("a.b+c@sub.example.co.uk") && !address_valid("a@b") && !address_valid("a b@c.com") && !address_valid("<a@c.com>") &&
              !address_valid("@c.com") && address_valid("me@localhost"),
          "which addresses are addresses");
    char *safe = mime_safe_filename("C:\\Users\\x\\..\\evil\nname.exe");
    check(!strcmp(safe, "evil_name.exe"), "a Windows path and a line break in a file name");
    g_free(safe);

    /* ---- writing ---- */
    GPtrArray *to = mime_addresses("Kofi <kofi@example.com>, \"Mensah, A.\" <a@example.org>");
    Attachment att = {"r\xc3\xa9sum\xc3\xa9.pdf", "application/pdf", g_bytes_new("PDFDATA", 7)};
    GPtrArray *atts = g_ptr_array_new();
    g_ptr_array_add(atts, &att);
    Draft d = {"Amá", "ama@example.com", to, NULL, "Café\r\nBcc: victim@evil.example", "Line one\nA line with = and é\n.\nend", atts,
               "<abc@example.com>", "<root@example.com>", 1793898245};
    char *id = NULL;
    gsize len = 0;
    char *raw = mime_build(&d, &id, &len);
    check(raw && id && g_str_has_suffix(id, "@example.com>") && strstr(raw, "Date: Thu, 05 Nov 2026 17:04:05 +0000\r\n"), "a message with a date and an id");
    check(!strstr(raw, "\r\nBcc:") && strstr(raw, "Subject: =?UTF-8?B?"), "a line break in the subject can't add a header");
    check(strstr(raw, "In-Reply-To: <abc@example.com>\r\nReferences: <root@example.com> <abc@example.com>\r\n") != NULL, "a reply's threading headers");
    check(strstr(raw, "\"Mensah, A.\" <a@example.org>") != NULL, "a name with a comma is quoted");
    Message *back = mime_parse(raw, len);
    check(!strcmp(back->subject, "Café  Bcc: victim@evil.example") && !strcmp(back->text, "Line one\nA line with = and é\n.\nend"),
          "it reads back: subject and text");
    check(back->attachments->len == 1 && !strcmp(((Attachment *)back->attachments->pdata[0])->filename, "résumé.pdf") &&
              g_bytes_get_size(((Attachment *)back->attachments->pdata[0])->data) == 7,
          "and its attachment, with its UTF-8 name");
    check(back->to->len == 2 && !strcmp(((Address *)back->to->pdata[1])->name, "Mensah, A."), "and its recipients");
    message_free(back);
    g_free(raw);
    g_free(id);
    g_bytes_unref(att.data);
    g_ptr_array_free(atts, TRUE);
    g_ptr_array_free(to, TRUE);
    char *r1 = mime_reply_subject("Re: lunch", FALSE), *r2 = mime_reply_subject("lunch", FALSE), *r3 = mime_reply_subject("Fwd: x", TRUE);
    check(!strcmp(r1, "Re: lunch") && !strcmp(r2, "Re: lunch") && !strcmp(r3, "Fwd: x"), "Re: and Fwd: not twice");
    g_free(r1); g_free(r2); g_free(r3);

    /* ---- IMAP ---- */
    const char *resp =
        "* 3 FETCH (UID 41 FLAGS (\\Seen \\Flagged) RFC822.SIZE 1234 BODY[HEADER.FIELDS (FROM SUBJECT)] {35}\r\n"
        "From: a@b.com\r\nSubject: hi )(\r\n\r\n)\r\n"
        "* LIST (\\HasNoChildren \\Sent) \"/\" \"Sent Items\"\r\n"
        "* LIST (\\Noselect) \".\" \"Entw&APw-rfe\"\r\n"
        "* SEARCH 4 7 9\r\n"
        "A003 OK done\r\n";
    GPtrArray *rs = imap_responses(resp, strlen(resp));
    ImapVal *f = rs->len ? rs->pdata[0] : NULL, *list = f && f->items->len > 2 ? f->items->pdata[2] : NULL;
    ImapVal *uid = imap_fetch_item(list, "UID"), *flags = imap_fetch_item(list, "FLAGS"), *h = imap_fetch_item(list, "BODY[");
    check(rs->len == 4 && uid && !strcmp(uid->s, "41") && flags && flags->items->len == 2, "a FETCH: UID and flags");
    check(h && h->n == 35 && strstr(h->s, "Subject: hi )("), "a literal with brackets in it, read whole");
    ImapVal *lst = rs->len > 1 ? rs->pdata[1] : NULL;
    check(lst && lst->items->len == 4 && !strcmp(((ImapVal *)lst->items->pdata[3])->s, "Sent Items"), "a LIST with a quoted name");
    char *u7 = imap_utf7_decode("Entw&APw-rfe"), *amp = imap_utf7_decode("Q&-A");
    check(!strcmp(u7, "Entwürfe") && !strcmp(amp, "Q&A"), "folder names in modified UTF-7");
    g_free(u7); g_free(amp);
    g_ptr_array_free(rs, TRUE);
    char *q = imap_quote("a \"b\" \\c"), *bad = imap_quote("x\r\nA1 DELETE INBOX");
    check(!strcmp(q, "\"a \\\"b\\\" \\\\c\"") && bad == NULL, "quoting: escapes, and a line break is refused (no injected command)");
    g_free(q);
    ok = 1;
    for (int r = 0; r < 3000; r++) {
        char *copy = g_strdup(resp);
        size_t n = strlen(copy);
        for (int k = 0; k < 1 + r % 6; k++) {
            seed = seed * 1103515245 + 12345;
            copy[(seed >> 8) % n] = "(){}\"\\* \r\n9["[(seed >> 20) % 13];
        }
        GPtrArray *x = imap_responses(copy, n);
        ok &= x != NULL;
        g_ptr_array_free(x, TRUE);
        g_free(copy);
    }
    check(ok, "3000 damaged server answers read safely");
    GPtrArray *huge = imap_responses("* 1 FETCH (BODY[] {999999999}\r\nshort", 34);
    check(huge->len == 1, "a literal longer than the data: cut, no overread");
    g_ptr_array_free(huge, TRUE);

    /* ---- passwords ---- */
    guint8 k1[32], k2[32];
    for (int i = 0; i < 32; i++) k1[i] = (guint8)i, k2[i] = (guint8)(i + 1);
    char *s = seal(k1, "a1", "hunter2 é"), *p = unseal(k1, "a1", s);
    check(s && !strstr(s, "hunter2") && p && !strcmp(p, "hunter2 é"), "a password sealed and opened again");
    char *wrongkey = unseal(k2, "a1", s), *wrongid = unseal(k1, "a2", s);
    check(!wrongkey && !wrongid, "the wrong key, or another account's: refused");
    char *t = g_strdup(s);
    t[10] = t[10] == 'A' ? 'B' : 'A';
    check(!unseal(k1, "a1", t) && !unseal(k1, "a1", "") && !unseal(k1, "a1", "!!notbase64"), "tampered or garbage: refused");
    g_free(s); g_free(p); g_free(t);

    /* ---- accounts ---- */
    Server in = {0}, out = {0};
    account_guess("ama@Gmail.com", &in, &out);
    check(!strcmp(in.host, "imap.gmail.com") && in.port == 993 && in.security == SEC_TLS && !strcmp(out.host, "smtp.gmail.com") && out.port == 465,
          "Gmail's servers");
    account_guess("kofi@school.edu.gh", &in, &out);
    check(!strcmp(in.host, "imap.school.edu.gh") && !strcmp(out.host, "smtp.school.edu.gh") && out.port == 587 && out.security == SEC_STARTTLS,
          "an unknown domain: imap. and smtp. with STARTTLS");
    g_free(in.host);
    g_free(out.host);

    char *tmp = g_dir_make_tmp("mail-test-XXXXXX", NULL);
    g_setenv("XDG_CACHE_HOME", tmp, TRUE);
    Box *b = box_load("a1", "INBOX");
    for (guint32 i = 1; i <= 3; i++) {
        Entry *e = g_new0(Entry, 1);
        e->uid = i;
        e->date = 1000 + i;
        e->from = g_strdup("A\tB");
        e->subject = g_strdup_printf("subject %u\nwith a break", i);
        e->seen = i == 2;
        g_ptr_array_add(b->entries, e);
    }
    b->uidvalidity = 77;
    b->uidnext = 4;
    check(box_save(b), "the offline copy saved");
    GBytes *body = g_bytes_new("raw message", 11);
    box_put_body(b, 2, body);
    box_free(b);
    b = box_load("a1", "INBOX");
    check(b->entries->len == 3 && b->uidvalidity == 77 && ((Entry *)b->entries->pdata[0])->uid == 3 && box_find(b, 2)->seen &&
              !strcmp(box_find(b, 1)->subject, "subject 1\nwith a break") && !strcmp(box_find(b, 1)->from, "A\tB"),
          "read back: newest first, flags, odd text");
    GBytes *got = box_body(b, 2);
    check(got && g_bytes_equal(got, body) && !box_body(b, 3), "a fetched message kept");
    box_remove(b, 2);
    check(!box_find(b, 2) && !box_body(b, 2), "and removed");
    g_bytes_unref(got);
    g_bytes_unref(body);
    box_free(b);

    /* ---- links from elsewhere */
    char *lto, *lcc, *subj, *text;
    check(mailto_parse("mailto:kofi@example.com?subject=Lunch%20on%20Friday&body=See%20you%0D%0AKofi&cc=yaw@example.com&bcc=spy@example.com",
                       &lto, &lcc, &subj, &text) &&
          !strcmp(lto, "kofi@example.com") && !strcmp(lcc, "yaw@example.com") && !strcmp(subj, "Lunch on Friday") && !strcmp(text, "See you\nKofi"),
          "mailto: address, subject, body (CRLF made \\n), cc; bcc ignored");
    g_free(lto); g_free(lcc); g_free(subj); g_free(text);
    check(mailto_parse("MAILTO:a@x.org,b@y.org?to=c@z.org", &lto, &lcc, &subj, &text) && !strcmp(lto, "a@x.org, b@y.org, c@z.org") && !*subj,
          "mailto: several addresses, any case");
    g_free(lto); g_free(lcc); g_free(subj); g_free(text);
    check(mailto_parse("mailto:not-an-address?subject=Hi%0D%0ABcc:%20spy@x.org", &lto, &lcc, &subj, &text) && !*lto &&
          !strchr(subj, '\n') && !strchr(subj, '\r'), "mailto: a bad address dropped; a line break can't add a header");
    g_free(lto); g_free(lcc); g_free(subj); g_free(text);
    check(mailto_parse("mailto:?subject=%E2%82%AC%zz%FF", &lto, &lcc, &subj, &text) && g_utf8_validate(subj, -1, NULL) && strstr(subj, "€"),
          "mailto: bad escapes and bad UTF-8 survive as text");
    g_free(lto); g_free(lcc); g_free(subj); g_free(text);
    check(!mailto_parse("https://example.com", &lto, &lcc, &subj, &text) && !lto, "not a mailto: link");

    GPtrArray *senders = g_ptr_array_new();
    g_ptr_array_add(senders, "Kofi Mensah <kofi@test.local>");
    g_ptr_array_add(senders, "Abena <abena@test.local>");
    g_ptr_array_add(senders, "Kofi Boateng <kboateng@work.example>");
    g_ptr_array_add(senders, "\"Mensah, Kofi\" <KOFI@test.local>");
    char *who = address_pick(senders, "abena");
    check(who && !strcmp(who, "Abena <abena@test.local>"), "write to Abena: her address from mail she sent");
    g_free(who);
    who = address_pick(senders, "kofi mensah");
    check(who && strstr(who, "kofi@test.local"), "two words; the same address written twice is one person");
    g_free(who);
    check(!address_pick(senders, "kofi"), "two different Kofis: no guess");
    check(!address_pick(senders, "yaw") && !address_pick(senders, "") && !address_pick(NULL, "x"), "nobody, or nothing asked: none");
    g_ptr_array_free(senders, TRUE);

    printf("%d of %d checks passed\n", checks - fails, checks);
    return fails != 0;
}
