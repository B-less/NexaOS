/* NexaLang runtime: HTTPS requests through libcurl (0.10). Included only
 * in programs that `use network` (they're linked with libcurl).
 *
 * The rules are kept here, not in std/network.nx, so no program can get
 * around them: https:// only (redirects too), the certificate always
 * checked, TLS 1.2 or newer, a time limit and a size limit on every
 * request, no cookies, headers that can't smuggle a second request.
 *
 * The request itself runs as plain C that touches no NexaLang memory (no
 * nx_alloc, no counts, no traps), so a window can run it on a helper
 * thread and keep drawing meanwhile (nx_run_blocking, as the broker's
 * requests do). Everything NexaLang sees is made afterwards, on the
 * program's own thread. */
#include <ctype.h>
#include <strings.h>
#include <curl/curl.h>

#define NX_NET_MAX_MEMORY ((int64_t)256 << 20)   /* an answer kept as text */
#define NX_NET_MAX_SAVE ((int64_t)1 << 40)       /* an answer saved to a file */
#define NX_NET_MAX_URL 8192
#define NX_NET_MAX_HEADERS 64                    /* sent */
#define NX_NET_MAX_HEADER_VALUE 8192
#define NX_NET_KEEP_HEADERS 100                  /* received and kept */
#define NX_NET_KEEP_HEADER_BYTES (64 << 10)

typedef struct {
    CURL *curl;
    int64_t max_bytes, got;
    int fd;                      /* saving to a file: the temporary file, else -1 */
    char *buf; size_t n, cap;    /* the answer, kept in memory (malloc'd) */
    char *hbuf; size_t hn, hcap; /* headers received: "name\0value\0" ... */
    int hcount;
    bool too_big, no_memory;
    int write_err;
    CURLcode rc;
} nx_net_job;

__attribute__((unused)) static bool nx_net_grow(char **p, size_t *cap, size_t need, size_t limit) {
    if (need <= *cap) return true;
    size_t c = *cap ? *cap : 4096;
    while (c < need) c = c > limit / 2 ? limit : c * 2;
    if (c < need) return false;
    char *np = realloc(*p, c);
    if (!np) return false;
    *p = np; *cap = c;
    return true;
}

__attribute__((unused)) static size_t nx_net_on_body(char *data, size_t size, size_t count, void *ud) {
    nx_net_job *j = ud;
    size_t n = size * count;
    if (n == 0) return 0;
    if ((int64_t)n > j->max_bytes - j->got) { j->too_big = true; return 0; }
    if (j->fd >= 0) {
        long status = 0;
        curl_easy_getinfo(j->curl, CURLINFO_RESPONSE_CODE, &status);
        if (status >= 200 && status <= 299) {
            size_t off = 0;
            while (off < n) {
                ssize_t w = write(j->fd, data + off, n - off);
                if (w < 0) { if (errno == EINTR) continue; j->write_err = errno; return 0; }
                off += (size_t)w;
            }
        } /* an answer that isn't a success isn't saved (only counted) */
    } else {
        if (!nx_net_grow(&j->buf, &j->cap, j->n + n, (size_t)j->max_bytes)) { j->no_memory = true; return 0; }
        memcpy(j->buf + j->n, data, n);
        j->n += n;
    }
    j->got += (int64_t)n;
    return n;
}

/* Headers: a status line ("HTTP/...") starts a new answer (after a
   redirect), so only the last answer's headers are kept. Folded lines and
   anything past the limits are left out; the request goes on. */
__attribute__((unused)) static size_t nx_net_on_header(char *data, size_t size, size_t count, void *ud) {
    nx_net_job *j = ud;
    size_t n = size * count;
    if (n >= 5 && memcmp(data, "HTTP/", 5) == 0) { j->hn = 0; j->hcount = 0; return n; }
    if (n == 0 || data[0] == ' ' || data[0] == '\t') return n;
    const char *colon = memchr(data, ':', n);
    if (!colon || colon == data) return n;
    size_t nl = (size_t)(colon - data);
    const char *v = colon + 1, *end = data + n;
    while (v < end && (*v == ' ' || *v == '\t')) v++;
    while (end > v && (end[-1] == '\r' || end[-1] == '\n' || end[-1] == ' ' || end[-1] == '\t')) end--;
    size_t vl = (size_t)(end - v);
    if (j->hcount >= NX_NET_KEEP_HEADERS || j->hn + nl + vl + 2 > NX_NET_KEEP_HEADER_BYTES) return n;
    if (memchr(data, 0, nl) || memchr(v, 0, vl)) return n;
    if (!nx_net_grow(&j->hbuf, &j->hcap, j->hn + nl + vl + 2, NX_NET_KEEP_HEADER_BYTES)) return n;
    memcpy(j->hbuf + j->hn, data, nl); j->hn += nl; j->hbuf[j->hn++] = 0;
    memcpy(j->hbuf + j->hn, v, vl); j->hn += vl; j->hbuf[j->hn++] = 0;
    j->hcount++;
    return n;
}

__attribute__((unused)) static void nx_net_job_run(void *arg) {
    nx_net_job *j = arg;
    j->rc = curl_easy_perform(j->curl);
}

/* The host of an address, for messages ("api.example.org"). */
__attribute__((unused)) static void nx_net_host(const char *url, char *out, size_t size) {
    snprintf(out, size, "the server");
    if (!url) return;
    CURLU *u = curl_url();
    if (!u) return;
    char *h = NULL;
    if (curl_url_set(u, CURLUPART_URL, url, 0) == CURLUE_OK && curl_url_get(u, CURLUPART_HOST, &h, 0) == CURLUE_OK && h && *h)
        snprintf(out, size, "%s", h);
    curl_free(h);
    curl_url_cleanup(u);
}

/* Header names are HTTP tokens; these are set by the module itself. */
NX_RT bool nx_net_token(nx_str s) {
    if (s.len == 0 || s.len > 256) return false;
    for (int64_t i = 0; i < s.len; i++) {
        unsigned char c = (unsigned char)s.p[i];
        if (!(isalnum(c) || strchr("!#$%&'*+-.^_`|~", c)) || c == 0) return false;
    }
    return true;
}
NX_RT bool nx_net_name_is(nx_str s, const char *name) {
    return (size_t)s.len == strlen(name) && strncasecmp(s.p, name, (size_t)s.len) == 0;
}

NX_RT nx_str nx_net_seconds(int64_t ms) {
    if (ms % 1000 == 0) return nx_fmt("%" PRId64 " second%s", ms / 1000, ms == 1000 ? "" : "s");
    return nx_fmt("%.1f seconds", (double)ms / 1000.0);
}
NX_RT nx_str nx_net_size(int64_t b) {
    if (b >= ((int64_t)1 << 30) && b % ((int64_t)1 << 30) == 0) return nx_fmt("%" PRId64 " GiB", b >> 30);
    if (b >= ((int64_t)1 << 20) && b % ((int64_t)1 << 20) == 0) return nx_fmt("%" PRId64 " MiB", b >> 20);
    if (b >= 1024 && b % 1024 == 0) return nx_fmt("%" PRId64 " KiB", b >> 10);
    return nx_fmt("%" PRId64 " bytes", b);
}

static bool nx_net_ready;

/* method, url, [name, value, ...], body, time limit (ms), size limit
   (bytes), save to ("" keeps the answer as text) ->
   [status, url, content type, body (or the bytes saved), name, value, ...] */
NX_RT nx_R_Ls nx_b___net_request(nx_str method, nx_str url, nx_arr *headers, nx_str body,
                                 int64_t timeout_ms, int64_t max_bytes, nx_str save, int line) {
    (void)line;
    nx_R_Ls r = {false, NULL, {"", 0, NULL}};
#define NX_NET_FAIL(...) do { r.e = nx_fmt(__VA_ARGS__); goto done; } while (0)
    CURL *c = NULL;
    struct curl_slist *hl = NULL;
    char *curl_s = NULL, *path = NULL, *tmp = NULL, *method_c = NULL;
    nx_net_job j;
    memset(&j, 0, sizeof j);
    j.fd = -1;
    bool saving = save.len > 0;

    /* ---- what's asked: checked before anything is sent ---- */
    static const char *const methods[] = {"GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"};
    int mi = -1;
    for (int i = 0; i < 6; i++)
        if ((size_t)method.len == strlen(methods[i]) && memcmp(method.p, methods[i], (size_t)method.len) == 0) mi = i;
    if (mi < 0)
        NX_NET_FAIL("the method must be GET, POST, PUT, PATCH, DELETE or HEAD (not \"%.*s\")", method.len > 40 ? 40 : (int)method.len, method.p);
    if (body.len > 0 && (mi == 0 || mi == 5)) NX_NET_FAIL("a %s request can't have a body", methods[mi]);
    if (url.len == 0) NX_NET_FAIL("the address is empty");
    if (url.len > NX_NET_MAX_URL) NX_NET_FAIL("the address is longer than %d bytes", NX_NET_MAX_URL);
    for (int64_t i = 0; i < url.len; i++) {
        unsigned char ch = (unsigned char)url.p[i];
        if (ch <= 0x20 || ch == 0x7f)
            NX_NET_FAIL("the address has a space or a control character in it (use network.encode for the parts of an address)");
    }
    if (url.len < 8 || strncasecmp(url.p, "https://", 8) != 0) {
        if (url.len >= 7 && strncasecmp(url.p, "http://", 7) == 0)
            NX_NET_FAIL("only https:// addresses are allowed, so nothing can be read or changed on the way (this one starts with http://)");
        NX_NET_FAIL("the address must start with https:// (\"%.*s\")", url.len > 60 ? 60 : (int)url.len, url.p);
    }
    curl_s = nx_cstr(url);
    {
        CURLU *u = curl_url();
        if (!u) nx_trap(0, "out of memory");
        char *host = NULL, *user = NULL;
        CURLUcode uc = curl_url_set(u, CURLUPART_URL, curl_s, 0);
        bool has_host = uc == CURLUE_OK && curl_url_get(u, CURLUPART_HOST, &host, 0) == CURLUE_OK && host && *host;
        bool has_user = uc == CURLUE_OK && curl_url_get(u, CURLUPART_USER, &user, 0) == CURLUE_OK && user;
        curl_free(host); curl_free(user);
        curl_url_cleanup(u);
        if (!has_host) NX_NET_FAIL("\"%.*s\" isn't a web address that can be used", url.len > 60 ? 60 : (int)url.len, url.p);
        if (has_user) NX_NET_FAIL("the address has a name or password in it; send those in a header instead, where they aren't logged with the address");
    }
    if (headers->len % 2 != 0) nx_trap(0, "network: headers must come in name, value pairs");
    if (headers->len / 2 > NX_NET_MAX_HEADERS) NX_NET_FAIL("too many headers (at most %d)", NX_NET_MAX_HEADERS);
    if (timeout_ms < 1 || timeout_ms > 600000) NX_NET_FAIL("the time limit must be 1 to 600000 ms (10 minutes), not %" PRId64, timeout_ms);
    int64_t max_allowed = saving ? NX_NET_MAX_SAVE : NX_NET_MAX_MEMORY;
    if (max_bytes < 1 || max_bytes > max_allowed) {
        nx_str most = nx_net_size(max_allowed);
        r.e = nx_fmt("the size limit must be 1 byte to %.*s, not %" PRId64, (int)most.len, most.p, max_bytes);
        nx_str_release(most);
        goto done;
    }
    for (int64_t i = 0; i + 1 < headers->len; i += 2) {
        nx_str name = *(nx_str *)(headers->data + i * headers->esz);
        nx_str value = *(nx_str *)(headers->data + (i + 1) * headers->esz);
        if (!nx_net_token(name))
            NX_NET_FAIL("\"%.*s\" can't be a header's name (letters, digits and !#$%%&'*+-.^_`|~ only)", name.len > 60 ? 60 : (int)name.len, name.p);
        static const char *const own[] = {"host", "content-length", "transfer-encoding", "connection", "expect", "upgrade", "te", "trailer", "keep-alive", "proxy-connection"};
        for (size_t k = 0; k < sizeof own / sizeof *own; k++)
            if (nx_net_name_is(name, own[k])) NX_NET_FAIL("the %.*s header is set by the network module itself", (int)name.len, name.p);
        if (value.len > NX_NET_MAX_HEADER_VALUE) NX_NET_FAIL("the %.*s header is longer than %d bytes", (int)name.len, name.p, NX_NET_MAX_HEADER_VALUE);
        if (memchr(value.p, '\r', (size_t)value.len) || memchr(value.p, '\n', (size_t)value.len) || memchr(value.p, 0, (size_t)value.len))
            NX_NET_FAIL("the %.*s header has a line break or a zero byte in it", (int)name.len, name.p);
        /* "Name;" sends an empty header ("Name:" would remove it) */
        char *line_c = malloc((size_t)name.len + (size_t)value.len + 3);
        if (!line_c) nx_trap(0, "out of memory");
        if (value.len == 0) sprintf(line_c, "%.*s;", (int)name.len, name.p);
        else sprintf(line_c, "%.*s: %.*s", (int)name.len, name.p, (int)value.len, value.p);
        struct curl_slist *nl2 = curl_slist_append(hl, line_c);
        free(line_c);
        if (!nl2) nx_trap(0, "out of memory");
        hl = nl2;
    }
    {
        /* no 100-continue wait; and no Content-Type unless the program
           says (curl would call every body a form) */
        bool typed = false;
        for (int64_t i = 0; i + 1 < headers->len; i += 2)
            if (nx_net_name_is(*(nx_str *)(headers->data + i * headers->esz), "content-type")) typed = true;
        struct curl_slist *nl2 = curl_slist_append(hl, "Expect:");
        if (!nl2) nx_trap(0, "out of memory");
        hl = nl2;
        if (!typed) {
            nl2 = curl_slist_append(hl, "Content-Type:");
            if (!nl2) nx_trap(0, "out of memory");
            hl = nl2;
        }
    }
    if (saving) {
        path = nx_cstr(save);
        if (!path) { r.e = NX_BAD_PATH("save the download", save); goto done; }
        size_t tl = strlen(path) + 64;
        tmp = malloc(tl);
        if (!tmp) nx_trap(0, "out of memory");
        snprintf(tmp, tl, "%s.nexa-tmp-%ld-%" PRId64, path, (long)getpid(), nx_tmp_counter++);
        j.fd = open(tmp, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0644);
        if (j.fd < 0) { r.e = nx_os_err("save the download as", save, errno); goto done; }
    }

    /* ---- the request ---- */
    if (!nx_net_ready) {
        if (curl_global_init(CURL_GLOBAL_DEFAULT) != CURLE_OK) NX_NET_FAIL("the network library couldn't start");
        nx_net_ready = true;
    }
    c = curl_easy_init();
    if (!c) NX_NET_FAIL("the network library couldn't start");
    j.curl = c;
    j.max_bytes = max_bytes;
    curl_easy_setopt(c, CURLOPT_URL, curl_s);
    curl_easy_setopt(c, CURLOPT_PROTOCOLS_STR, "https");
    curl_easy_setopt(c, CURLOPT_REDIR_PROTOCOLS_STR, "https");
    curl_easy_setopt(c, CURLOPT_FOLLOWLOCATION, 1L);
    curl_easy_setopt(c, CURLOPT_MAXREDIRS, 5L);
    curl_easy_setopt(c, CURLOPT_SSL_VERIFYPEER, 1L);
    curl_easy_setopt(c, CURLOPT_SSL_VERIFYHOST, 2L);
    curl_easy_setopt(c, CURLOPT_SSLVERSION, (long)CURL_SSLVERSION_TLSv1_2);
    curl_easy_setopt(c, CURLOPT_NOSIGNAL, 1L);
    curl_easy_setopt(c, CURLOPT_NOPROGRESS, 1L);
    curl_easy_setopt(c, CURLOPT_TIMEOUT_MS, (long)timeout_ms);
    curl_easy_setopt(c, CURLOPT_CONNECTTIMEOUT_MS, (long)(timeout_ms < 15000 ? timeout_ms : 15000));
    curl_easy_setopt(c, CURLOPT_ACCEPT_ENCODING, "");  /* gzip etc.; the size limit counts what's unpacked */
    curl_easy_setopt(c, CURLOPT_MAXFILESIZE_LARGE, (curl_off_t)max_bytes);
    curl_easy_setopt(c, CURLOPT_USERAGENT, "NexaLang/0.10");
    curl_easy_setopt(c, CURLOPT_HTTPHEADER, hl);
    curl_easy_setopt(c, CURLOPT_WRITEFUNCTION, nx_net_on_body);
    curl_easy_setopt(c, CURLOPT_WRITEDATA, &j);
    curl_easy_setopt(c, CURLOPT_HEADERFUNCTION, nx_net_on_header);
    curl_easy_setopt(c, CURLOPT_HEADERDATA, &j);
    {
        /* Tests only: trust this file's certificates instead of the system's
           (a local test server). Like NEXA_BROKER_SOCKET it changes only
           this program's own requests; checking stays on. */
        const char *ca = getenv("NEXALANG_TEST_CA");
        if (ca && *ca) curl_easy_setopt(c, CURLOPT_CAINFO, ca);
    }
    if (mi == 5) curl_easy_setopt(c, CURLOPT_NOBODY, 1L);
    else if (mi != 0) {
        method_c = strdup(methods[mi]);
        if (!method_c) nx_trap(0, "out of memory");
        if (mi != 1) curl_easy_setopt(c, CURLOPT_CUSTOMREQUEST, method_c);
        curl_easy_setopt(c, CURLOPT_POSTFIELDSIZE_LARGE, (curl_off_t)body.len);
        curl_easy_setopt(c, CURLOPT_POSTFIELDS, body.len ? body.p : "");
    }
    if (nx_run_blocking) nx_run_blocking(nx_net_job_run, &j);
    else nx_net_job_run(&j);

    /* ---- what came back ---- */
    char host[300];
    {
        char *eff = NULL;
        curl_easy_getinfo(c, CURLINFO_EFFECTIVE_URL, &eff);
        nx_net_host(eff ? eff : curl_s, host, sizeof host);
    }
    if (j.no_memory) nx_trap(0, "out of memory");
    if (j.rc != CURLE_OK) {
        long os = 0;
        curl_easy_getinfo(c, CURLINFO_OS_ERRNO, &os);
        switch (j.rc) {
        case CURLE_COULDNT_RESOLVE_HOST:
            NX_NET_FAIL("can't find %s: check the address, and that this computer is online", host);
        case CURLE_COULDNT_RESOLVE_PROXY:
            NX_NET_FAIL("can't find the proxy server this computer is set to use");
        case CURLE_COULDNT_CONNECT:
            if (os == EACCES || os == EPERM)
                NX_NET_FAIL("NexaOS didn't let this program use the internet: an app needs the `network` permission in its nexa.toml, and you can switch it on in Settings");
            if (os == ECONNREFUSED) NX_NET_FAIL("%s refused the connection", host);
            if (os == ENETUNREACH || os == EHOSTUNREACH) NX_NET_FAIL("can't reach %s: this computer may not be online", host);
            NX_NET_FAIL("can't connect to %s", host);
        case CURLE_OPERATION_TIMEDOUT: {
            nx_str t = nx_net_seconds(timeout_ms);
            r.e = nx_fmt("no answer from %s within %.*s", host, (int)t.len, t.p);
            nx_str_release(t);
            goto done;
        }
        case CURLE_PEER_FAILED_VERIFICATION:
        case CURLE_SSL_CACERT_BADFILE:
        case CURLE_SSL_ISSUER_ERROR:
        case CURLE_SSL_PINNEDPUBKEYNOTMATCH:
            NX_NET_FAIL("%s's certificate couldn't be checked, so nothing was sent: it may not be the real %s (or this computer's clock is wrong)", host, host);
        case CURLE_SSL_CONNECT_ERROR:
            NX_NET_FAIL("a secure connection to %s couldn't be made", host);
        case CURLE_TOO_MANY_REDIRECTS:
            NX_NET_FAIL("%s sent the request on too many times (more than 5 redirects)", host);
        case CURLE_UNSUPPORTED_PROTOCOL:
            NX_NET_FAIL("%s sent the request on to an address that isn't https://, so it was stopped", host);
        case CURLE_FILESIZE_EXCEEDED:
        case CURLE_WRITE_ERROR:
            if (j.write_err) { r.e = nx_os_err("save the download as", save, j.write_err); goto done; }
            if (j.too_big || j.rc == CURLE_FILESIZE_EXCEEDED) {
                nx_str most = nx_net_size(max_bytes);
                r.e = nx_fmt("the answer from %s is bigger than %.*s, so it was stopped", host, (int)most.len, most.p);
                nx_str_release(most);
                goto done;
            }
            NX_NET_FAIL("the answer from %s couldn't be kept", host);
        case CURLE_GOT_NOTHING:
            NX_NET_FAIL("%s closed the connection without answering", host);
        case CURLE_SEND_ERROR:
        case CURLE_RECV_ERROR:
        case CURLE_PARTIAL_FILE:
            NX_NET_FAIL("the connection to %s broke before the answer was complete", host);
        case CURLE_BAD_CONTENT_ENCODING:
            NX_NET_FAIL("%s's answer is packed in a way that couldn't be unpacked", host);
        default:
            NX_NET_FAIL("the request to %s failed: %s", host, curl_easy_strerror(j.rc));
        }
    }
    long status = 0;
    curl_easy_getinfo(c, CURLINFO_RESPONSE_CODE, &status);
    nx_str text = nx_str_lit("", 0);
    if (saving) {
        if (status >= 200 && status <= 299) {
            if (fsync(j.fd) != 0 || close(j.fd) != 0) { int e = errno; j.fd = -1; r.e = nx_os_err("save the download as", save, e); goto done; }
            j.fd = -1;
            if (rename(tmp, path) != 0) { r.e = nx_os_err("save the download as", save, errno); goto done; }
            free(tmp); tmp = NULL; /* in place: nothing to remove */
        }
        text = nx_fmt("%" PRId64, j.got);
    } else if (j.n > 0) {
        if (!nx_utf8_ok((const unsigned char *)j.buf, (int64_t)j.n)) {
            char *ct = NULL;
            curl_easy_getinfo(c, CURLINFO_CONTENT_TYPE, &ct);
            NX_NET_FAIL("the answer from %s isn't text (%s): save it to a file with network.download instead", host, ct && *ct ? ct : "no type given");
        }
        text = nx_str_heap(j.buf, j.n);
    }
    nx_arr *a = nx_arr_new((int64_t)sizeof(nx_str), 4 + 2 * (int64_t)j.hcount, nx_str_eretain, nx_str_erelease);
    nx_str s = nx_fmt("%ld", status);
    nx_arr_push(a, &s, line);
    char *eff = NULL, *ct = NULL;
    curl_easy_getinfo(c, CURLINFO_EFFECTIVE_URL, &eff);
    curl_easy_getinfo(c, CURLINFO_CONTENT_TYPE, &ct);
    s = eff ? nx_str_lossy(eff, strlen(eff)) : nx_str_retain(url);
    nx_arr_push(a, &s, line);
    s = ct ? nx_str_lossy(ct, strlen(ct)) : nx_str_lit("", 0);
    nx_arr_push(a, &s, line);
    nx_arr_push(a, &text, line);
    for (size_t off = 0; off < j.hn;) {
        size_t l = strlen(j.hbuf + off);
        s = nx_str_lossy(j.hbuf + off, l);
        nx_arr_push(a, &s, line);
        off += l + 1;
    }
    r.ok = true;
    r.v = a;
done:
#undef NX_NET_FAIL
    if (j.fd >= 0) close(j.fd);
    if (tmp) unlink(tmp);
    if (c) curl_easy_cleanup(c);
    curl_slist_free_all(hl);
    free(j.buf); free(j.hbuf); free(curl_s); free(path); free(tmp); free(method_c);
    return r;
}
