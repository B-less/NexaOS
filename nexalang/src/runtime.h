/* NexaLang runtime v0.2. Included at the top of every generated program.
 *
 * Memory: strings and lists are reference counted. A value is freed the
 * moment its last owner lets go; there is no garbage collector and no
 * pause. Lists are values: copying one shares it until one copy is
 * changed, and then that copy is made private (copy-on-write).
 * Counts are not atomic: NexaLang 0.2 programs are single-threaded.
 *
 * Every check that can fail stops the program with a plain message
 * (file and line), never undefined behaviour. */
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>
#include <math.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <signal.h>
#include <sys/resource.h>

#define NX_RT static inline __attribute__((unused))

/* nexac refuses functions that always call themselves (with a message);
   the C compiler's own, different analysis of the same thing must never
   turn into "the C compiler failed". Running out of stack is reported
   by nx_on_segv below. All the hardened flags stay. */
#if defined(__clang__)
#pragma clang diagnostic ignored "-Winfinite-recursion"
#elif defined(__GNUC__) && __GNUC__ >= 12
#pragma GCC diagnostic ignored "-Winfinite-recursion"
#endif

typedef void (*nx_efn)(void *);
/* rc is NULL for text that lives forever (literals, program arguments). */
typedef struct { const char *p; int64_t len; int64_t *rc; } nx_str;
typedef struct {
    int64_t rc, len, cap, esz;
    unsigned char *data;
    nx_efn retain, release; /* per element; NULL when elements own nothing */
} nx_arr;
typedef struct { bool has; int64_t v; } nx_M_i;
typedef struct { bool has; double v; } nx_M_f;
typedef struct { bool has; nx_str v; } nx_M_s;
/* !T results: ok, the value (when ok), the error message (when not). */
typedef struct { bool ok; nx_str v; nx_str e; } nx_R_s;
typedef struct { bool ok; nx_str e; } nx_R_v;
typedef struct { bool ok; nx_arr *v; nx_str e; } nx_R_Ls;

static int nx_argc;
static char **nx_argv;
static int64_t nx_live; /* blocks currently allocated (for the leak check) */

/* Source files, by index; a location is index * 1000000 + line. */
static const char *const nx_files[] = { NX_FILES };
static const bool nx_std_files[] = { NX_STD_FILES };
/* Where the program last called into a standard module: a runtime error
   inside one is reported there, in the program's own file. */
__attribute__((unused)) static int nx_user_loc;

__attribute__((noreturn, format(printf, 2, 3), unused))
static void nx_trap(int loc, const char *fmt, ...) {
    va_list ap;
    if (nx_std_files[loc / 1000000] && nx_user_loc > 0) loc = nx_user_loc;
    int line = loc % 1000000;
    const char *file = nx_files[loc / 1000000];
    fflush(stdout);
    if (line > 0) fprintf(stderr, "nexa: runtime error in %s at line %d: ", file, line);
    else fprintf(stderr, "nexa: runtime error in %s: ", file);
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fputc('\n', stderr);
    exit(1);
}

/* Running out of stack (a function calling itself too deeply) is a
   runtime error with a message, not a silent crash. The handler runs on
   its own small stack and only uses async-signal-safe calls. Any other
   crash is left alone (it's a bug in the runtime, not the program). */
static uintptr_t nx_stack_top; /* roughly where main's stack starts */
static size_t nx_stack_size;
static void nx_on_segv(int sig, siginfo_t *si, void *ctx) {
    (void)ctx;
    uintptr_t a = (uintptr_t)si->si_addr;
    if (nx_stack_top && a < nx_stack_top && nx_stack_top - a <= nx_stack_size + ((size_t)1 << 20)) {
        static const char a1[] = "nexa: runtime error in ";
        static const char a2[] = ": the program ran out of stack space (a function called itself too many times)\n";
        ssize_t w = write(2, a1, sizeof a1 - 1);
        w = write(2, nx_files[0], strlen(nx_files[0])); /* the program's main file */
        w = write(2, a2, sizeof a2 - 1);
        (void)w;
        _exit(1);
    }
    signal(sig, SIG_DFL); /* fault again, the normal way */
}
static void nx_init(void) {
    /* `nexac run` asks for output line by line (0.8): someone is watching
       it, maybe through a pipe (a code editor's output view), not only
       when 4 KB have piled up or the program ends */
    const char *lines = getenv("NEXALANG_LINE_OUTPUT");
    if (lines && strcmp(lines, "1") == 0) setvbuf(stdout, NULL, _IOLBF, 0);
    static char alt[65536];
    stack_t ss;
    memset(&ss, 0, sizeof ss);
    ss.ss_sp = alt;
    ss.ss_size = sizeof alt;
    if (sigaltstack(&ss, NULL) != 0) return;
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = nx_on_segv;
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGSEGV, &sa, NULL);
    struct rlimit rl;
    if (getrlimit(RLIMIT_STACK, &rl) == 0 && rl.rlim_cur != RLIM_INFINITY) nx_stack_size = (size_t)rl.rlim_cur;
    else nx_stack_size = (size_t)1 << 32;
    nx_stack_top = (uintptr_t)__builtin_frame_address(0);
}

NX_RT void *nx_alloc(size_t n) {
    void *p = malloc(n ? n : 1);
    if (!p) nx_trap(0, "out of memory");
    nx_live++;
    return p;
}
NX_RT void nx_free(void *p) {
    if (p) { nx_live--; free(p); }
}
NX_RT int nx_finish(void) {
    fflush(stdout);
#ifdef NX_LEAK_CHECK
    if (nx_live != 0) {
        fprintf(stderr, "nexa: leak check: %" PRId64 " block(s) never freed\n", nx_live);
        return 3;
    }
#endif
    return 0;
}

/* ---- int: every operation is checked ---- */
NX_RT int64_t nx_add(int64_t a, int64_t b, int line) {
    int64_t r;
    if (__builtin_add_overflow(a, b, &r)) nx_trap(line, "the number got too big for an int (in +)");
    return r;
}
NX_RT int64_t nx_sub(int64_t a, int64_t b, int line) {
    int64_t r;
    if (__builtin_sub_overflow(a, b, &r)) nx_trap(line, "the number got too big for an int (in -)");
    return r;
}
NX_RT int64_t nx_mul(int64_t a, int64_t b, int line) {
    int64_t r;
    if (__builtin_mul_overflow(a, b, &r)) nx_trap(line, "the number got too big for an int (in *)");
    return r;
}
NX_RT int64_t nx_div(int64_t a, int64_t b, int line) {
    if (b == 0) nx_trap(line, "division by zero");
    if (a == INT64_MIN && b == -1) nx_trap(line, "the number got too big for an int (in /)");
    return a / b;
}
NX_RT int64_t nx_rem(int64_t a, int64_t b, int line) {
    if (b == 0) nx_trap(line, "division by zero (in %%)");
    if (a == INT64_MIN && b == -1) return 0;
    return a % b;
}
NX_RT int64_t nx_neg(int64_t a, int line) {
    if (a == INT64_MIN) nx_trap(line, "the number got too big for an int (in -)");
    return -a;
}
NX_RT int64_t nx_to_int(double d, int line) {
    if (!(d >= -9223372036854775808.0 && d < 9223372036854775808.0))
        nx_trap(line, "to_int: %g doesn't fit in an int", d);
    return (int64_t)d;
}

/* ---- str ---- */
NX_RT nx_str nx_str_lit(const char *p, int64_t len) { nx_str s = {p, len, NULL}; return s; }
NX_RT nx_str nx_str_retain(nx_str s) { if (s.rc) ++*s.rc; return s; }
NX_RT void nx_str_release(nx_str s) { if (s.rc && --*s.rc == 0) nx_free(s.rc); }
NX_RT void nx_str_eretain(void *p) { (void)nx_str_retain(*(nx_str *)p); }
NX_RT void nx_str_erelease(void *p) { nx_str_release(*(nx_str *)p); }

/* New text of n bytes with one owner; *out is where to write the bytes. */
NX_RT nx_str nx_str_new(size_t n, char **out) {
    if (n == 0) { *out = NULL; return nx_str_lit("", 0); }
    if (n > (size_t)INT64_MAX - sizeof(int64_t)) nx_trap(0, "the text is too big");
    int64_t *h = nx_alloc(sizeof(int64_t) + n);
    *h = 1;
    *out = (char *)(h + 1);
    nx_str s = {*out, (int64_t)n, h};
    return s;
}
NX_RT nx_str nx_str_heap(const char *src, size_t n) {
    char *w;
    nx_str s = nx_str_new(n, &w);
    if (n) memcpy(w, src, n);
    return s;
}
/* A piece of s that shares its bytes. */
NX_RT nx_str nx_str_sub(nx_str s, int64_t a, int64_t b) {
    if (a == b) return nx_str_lit("", 0);
    nx_str r = {s.p + a, b - a, s.rc};
    return nx_str_retain(r);
}
NX_RT nx_str nx_str_cat(nx_str a, nx_str b) {
    /* lengths are never negative; saying so also lets gcc's checks see it */
    if (a.len < 0 || b.len < 0) nx_trap(0, "internal error: text with a negative length");
    if (b.len == 0) return nx_str_retain(a);
    if (a.len == 0) return nx_str_retain(b);
    if (a.len > INT64_MAX - b.len) nx_trap(0, "the text is too big");
    char *w;
    nx_str s = nx_str_new((size_t)a.len + (size_t)b.len, &w);
    memcpy(w, a.p, (size_t)a.len);
    memcpy(w + a.len, b.p, (size_t)b.len);
    return s;
}
NX_RT bool nx_str_eq(nx_str a, nx_str b) {
    return a.len == b.len && (a.len == 0 || memcmp(a.p, b.p, (size_t)a.len) == 0);
}
NX_RT int nx_str_cmp(nx_str a, nx_str b) {
    size_t n = (size_t)(a.len < b.len ? a.len : b.len);
    int c = n ? memcmp(a.p, b.p, n) : 0;
    if (c != 0) return c < 0 ? -1 : 1;
    return a.len < b.len ? -1 : (a.len > b.len ? 1 : 0);
}
NX_RT nx_str nx_int_str(int64_t v) {
    char buf[32];
    int n = snprintf(buf, sizeof buf, "%" PRId64, v);
    return nx_str_heap(buf, (size_t)n);
}
NX_RT nx_str nx_float_str(double v) {
    /* Plain notation is only used below 1e17 (short), but the buffer fits
       any double written with %f (at most 332 bytes), so the C compiler
       can check it without knowing that (found by the fuzzer: print(1e308)
       failed -Werror=format-truncation). */
    char buf[384];
    if (isnan(v)) return nx_str_lit("nan", 3);
    if (isinf(v)) return v > 0 ? nx_str_lit("inf", 3) : nx_str_lit("-inf", 4);
    /* fewest significant digits that read back as the same number */
    int prec = 1;
    for (; prec < 17; prec++) {
        snprintf(buf, sizeof buf, "%.*e", prec - 1, v);
        if (strtod(buf, NULL) == v) break;
    }
    snprintf(buf, sizeof buf, "%.*e", prec - 1, v);
    int exp10 = atoi(strchr(buf, 'e') + 1);
    if (exp10 >= -5 && exp10 < 17) {
        /* plain notation: 1000.0, 0.001, 3.14159 */
        int decimals = prec - 1 - exp10;
        snprintf(buf, sizeof buf, "%.*f", decimals > 0 ? decimals : 0, v);
    }
    size_t n = strlen(buf);
    if (!strpbrk(buf, ".e")) { buf[n++] = '.'; buf[n++] = '0'; buf[n] = 0; }
    return nx_str_heap(buf, n);
}
/* Length of the valid UTF-8 letter at p[0..n), or 0 if it isn't one. */
NX_RT int nx_utf8_letter(const unsigned char *p, int64_t n) {
    unsigned c = p[0];
    if (c < 0x80) return 1;
    int k; unsigned min, v;
    if ((c & 0xE0) == 0xC0) { k = 1; v = c & 0x1F; min = 0x80; }
    else if ((c & 0xF0) == 0xE0) { k = 2; v = c & 0x0F; min = 0x800; }
    else if ((c & 0xF8) == 0xF0) { k = 3; v = c & 0x07; min = 0x10000; }
    else return 0;
    if (k >= n) return 0;
    for (int j = 1; j <= k; j++) {
        if ((p[j] & 0xC0) != 0x80) return 0;
        v = (v << 6) | (p[j] & 0x3F);
    }
    if (v < min || v > 0x10FFFF || (v >= 0xD800 && v <= 0xDFFF)) return 0;
    return k + 1;
}
/* Text from bytes that should be UTF-8: anything that isn't becomes U+FFFD,
   so every `str` in a NexaLang program is valid UTF-8. */
NX_RT nx_str nx_str_lossy(const char *src, size_t n) {
    const unsigned char *p = (const unsigned char *)src;
    size_t bad = 0;
    for (size_t i = 0; i < n;) {
        int k = nx_utf8_letter(p + i, (int64_t)(n - i));
        if (k == 0) { bad++; i++; } else i += (size_t)k;
    }
    if (bad == 0) return nx_str_heap(src, n);
    char *w;
    nx_str s = nx_str_new(n + bad * 2, &w);
    for (size_t i = 0; i < n;) {
        int k = nx_utf8_letter(p + i, (int64_t)(n - i));
        if (k == 0) { memcpy(w, "\357\277\275", 3); w += 3; i++; }
        else { memcpy(w, p + i, (size_t)k); w += k; i += (size_t)k; }
    }
    return s;
}
NX_RT nx_str nx_bool_str(bool b) { return b ? nx_str_lit("true", 4) : nx_str_lit("false", 5); }
NX_RT void nx_print(nx_str s) {
    if (s.len) fwrite(s.p, 1, (size_t)s.len, stdout);
    fputc('\n', stdout);
}
NX_RT void nx_assert(bool ok, int line) {
    if (!ok) nx_trap(line, "assert failed");
}

/* ---- lists ---- */
NX_RT nx_arr *nx_arr_new(int64_t esz, int64_t cap, nx_efn retain, nx_efn release) {
    nx_arr *a = nx_alloc(sizeof *a);
    a->rc = 1;
    a->len = 0;
    a->cap = cap;
    a->esz = esz;
    a->retain = retain;
    a->release = release;
    a->data = cap ? nx_alloc((size_t)(cap * esz)) : NULL;
    return a;
}
NX_RT nx_arr *nx_arr_retain(nx_arr *a) { a->rc++; return a; }
NX_RT void nx_arr_release(nx_arr *a) {
    if (--a->rc > 0) return;
    if (a->release)
        for (int64_t i = 0; i < a->len; i++) a->release(a->data + i * a->esz);
    nx_free(a->data);
    nx_free(a);
}
NX_RT void nx_arr_eretain(void *p) { (void)nx_arr_retain(*(nx_arr **)p); }
NX_RT void nx_arr_erelease(void *p) { nx_arr_release(*(nx_arr **)p); }
NX_RT int64_t nx_arr_len(const nx_arr *a) { return a->len; }
/* Make *pa safe to change: if it's shared, give it its own copy. */
NX_RT nx_arr *nx_arr_mut(nx_arr **pa, int line) {
    (void)line;
    nx_arr *a = *pa;
    if (a->rc == 1) return a;
    nx_arr *c = nx_arr_new(a->esz, a->len, a->retain, a->release);
    if (a->len) memcpy(c->data, a->data, (size_t)(a->len * a->esz));
    c->len = a->len;
    if (c->retain)
        for (int64_t i = 0; i < c->len; i++) c->retain(c->data + i * c->esz);
    a->rc--;
    *pa = c;
    return c;
}
NX_RT void *nx_arr_at(nx_arr *a, int64_t i, int line) {
    if (i < 0 || i >= a->len)
        nx_trap(line, "position %" PRId64 " is outside the list (length %" PRId64 ")", i, a->len);
    return a->data + i * a->esz;
}
/* Moves the value at v into the list. */
NX_RT void nx_arr_push(nx_arr *a, const void *v, int line) {
    if (a->len == a->cap) {
        int64_t cap = a->cap ? a->cap * 2 : 4;
        if (cap > INT64_MAX / a->esz / 2) nx_trap(line, "the list is too big");
        unsigned char *d = a->data ? realloc(a->data, (size_t)(cap * a->esz)) : nx_alloc((size_t)(cap * a->esz));
        if (!d) nx_trap(line, "out of memory");
        a->data = d;
        a->cap = cap;
    }
    memcpy(a->data + a->len * a->esz, v, (size_t)a->esz);
    a->len++;
}
/* Moves the last value out of the list into *out. */
NX_RT void nx_arr_pop(nx_arr *a, void *out, int line) {
    if (a->len == 0) nx_trap(line, "pop from an empty list");
    a->len--;
    memcpy(out, a->data + a->len * a->esz, (size_t)a->esz);
}

/* ---- built-in text functions (nx_b_<name>) ---- */
NX_RT bool nx_is_cont(nx_str s, int64_t i) {
    return i > 0 && i < s.len && ((unsigned char)s.p[i] & 0xC0) == 0x80;
}
NX_RT nx_str nx_b_slice(nx_str s, int64_t a, int64_t b, int line) {
    if (a < 0 || b < a || b > s.len)
        nx_trap(line, "slice(%" PRId64 ", %" PRId64 ") is outside the text (length %" PRId64 ")", a, b, s.len);
    if (nx_is_cont(s, a) || nx_is_cont(s, b))
        nx_trap(line, "slice(%" PRId64 ", %" PRId64 ") would cut a letter in half", a, b);
    return nx_str_sub(s, a, b);
}
NX_RT int64_t nx_find_from(nx_str s, nx_str sub, int64_t from) {
    if (sub.len == 0) return from;
    for (int64_t i = from; i + sub.len <= s.len; i++)
        if (s.p[i] == sub.p[0] && memcmp(s.p + i, sub.p, (size_t)sub.len) == 0) return i;
    return -1;
}
NX_RT nx_M_i nx_b_find(nx_str s, nx_str sub, int line) {
    (void)line;
    int64_t i = nx_find_from(s, sub, 0);
    nx_M_i r = {i >= 0, i >= 0 ? i : 0};
    return r;
}
NX_RT bool nx_b_contains(nx_str s, nx_str sub, int line) { (void)line; return nx_find_from(s, sub, 0) >= 0; }
NX_RT bool nx_b_starts_with(nx_str s, nx_str p, int line) {
    (void)line;
    return p.len <= s.len && (p.len == 0 || memcmp(s.p, p.p, (size_t)p.len) == 0);
}
NX_RT bool nx_b_ends_with(nx_str s, nx_str p, int line) {
    (void)line;
    return p.len <= s.len && (p.len == 0 || memcmp(s.p + s.len - p.len, p.p, (size_t)p.len) == 0);
}
NX_RT nx_arr *nx_b_split(nx_str s, nx_str sep, int line) {
    if (sep.len == 0) nx_trap(line, "split needs a separator that isn't empty");
    nx_arr *a = nx_arr_new((int64_t)sizeof(nx_str), 0, nx_str_eretain, nx_str_erelease);
    int64_t start = 0;
    for (;;) {
        int64_t i = nx_find_from(s, sep, start);
        int64_t end = i < 0 ? s.len : i;
        nx_str part = nx_str_sub(s, start, end);
        nx_arr_push(a, &part, line);
        if (i < 0) break;
        start = i + sep.len;
    }
    return a;
}
NX_RT nx_str nx_b_join(nx_arr *a, nx_str sep, int line) {
    int64_t total = 0;
    for (int64_t i = 0; i < a->len; i++) {
        int64_t add = ((nx_str *)a->data)[i].len + (i ? sep.len : 0);
        if (total > INT64_MAX - add) nx_trap(line, "the text is too big");
        total += add;
    }
    char *w;
    nx_str r = nx_str_new((size_t)total, &w);
    for (int64_t i = 0; i < a->len; i++) {
        nx_str p = ((nx_str *)a->data)[i];
        if (i && sep.len) { memcpy(w, sep.p, (size_t)sep.len); w += sep.len; }
        if (p.len) { memcpy(w, p.p, (size_t)p.len); w += p.len; }
    }
    return r;
}
NX_RT bool nx_is_space(char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\v' || c == '\f'; }
NX_RT nx_str nx_b_trim(nx_str s, int line) {
    (void)line;
    int64_t a = 0, b = s.len;
    while (a < b && nx_is_space(s.p[a])) a++;
    while (b > a && nx_is_space(s.p[b - 1])) b--;
    return nx_str_sub(s, a, b);
}
NX_RT nx_str nx_case(nx_str s, bool upper) {
    char *w;
    nx_str r = nx_str_new((size_t)s.len, &w);
    for (int64_t i = 0; i < s.len; i++) {
        char c = s.p[i];
        if (upper && c >= 'a' && c <= 'z') c = (char)(c - 32);
        if (!upper && c >= 'A' && c <= 'Z') c = (char)(c + 32);
        w[i] = c;
    }
    return r;
}
NX_RT nx_str nx_b_to_upper(nx_str s, int line) { (void)line; return nx_case(s, true); }
NX_RT nx_str nx_b_to_lower(nx_str s, int line) { (void)line; return nx_case(s, false); }
NX_RT nx_str nx_b_replace(nx_str s, nx_str from, nx_str to, int line) {
    if (from.len == 0) nx_trap(line, "replace needs text to look for that isn't empty");
    int64_t count = 0;
    for (int64_t i = nx_find_from(s, from, 0); i >= 0; i = nx_find_from(s, from, i + from.len)) count++;
    if (count == 0) return nx_str_retain(s);
    int64_t grow = to.len - from.len;
    if (grow > 0 && count > (INT64_MAX - s.len) / grow) nx_trap(line, "the text is too big");
    char *w;
    nx_str r = nx_str_new((size_t)(s.len + count * grow), &w);
    int64_t at = 0;
    for (int64_t i = nx_find_from(s, from, 0); i >= 0; i = nx_find_from(s, from, at)) {
        if (i > at) { memcpy(w, s.p + at, (size_t)(i - at)); w += i - at; }
        if (to.len) { memcpy(w, to.p, (size_t)to.len); w += to.len; }
        at = i + from.len;
    }
    if (s.len > at) memcpy(w, s.p + at, (size_t)(s.len - at));
    return r;
}
NX_RT nx_str nx_b_repeat(nx_str s, int64_t n, int line) {
    if (n < 0) nx_trap(line, "repeat needs a count that isn't negative (got %" PRId64 ")", n);
    if (s.len && n > INT64_MAX / s.len) nx_trap(line, "the text is too big");
    char *w;
    nx_str r = nx_str_new((size_t)(s.len * n), &w);
    for (int64_t i = 0; i < n && s.len; i++) memcpy(w + i * s.len, s.p, (size_t)s.len);
    return r;
}
NX_RT nx_M_i nx_b_parse_int(nx_str s, int line) {
    (void)line;
    nx_M_i none = {false, 0};
    int64_t i = 0, v = 0;
    bool neg = false;
    if (i < s.len && (s.p[i] == '-' || s.p[i] == '+')) neg = s.p[i++] == '-';
    if (i == s.len) return none;
    for (; i < s.len; i++) {
        char c = s.p[i];
        if (c < '0' || c > '9') return none;
        /* build as a negative number so INT64_MIN fits */
        if (__builtin_mul_overflow(v, 10, &v) || __builtin_sub_overflow(v, c - '0', &v)) return none;
    }
    if (!neg) {
        if (v == INT64_MIN) return none;
        v = -v;
    }
    nx_M_i r = {true, v};
    return r;
}
NX_RT nx_M_f nx_b_parse_float(nx_str s, int line) {
    (void)line;
    nx_M_f none = {false, 0.0};
    if (s.len == 0 || s.len > 400) return none;
    char buf[401];
    for (int64_t i = 0; i < s.len; i++) {
        char c = s.p[i];
        /* digits, sign, point, exponent only: no spaces, hex, "nan", "inf" */
        if (!((c >= '0' && c <= '9') || c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E')) return none;
        buf[i] = c;
    }
    buf[s.len] = 0;
    char *end;
    double d = strtod(buf, &end);
    if (end != buf + s.len || !isfinite(d)) return none;
    nx_M_f r = {true, d};
    return r;
}
NX_RT nx_M_s nx_b_read_line(int line) {
    nx_M_s none = {false, {"", 0, NULL}};
    size_t cap = 128, n = 0;
    char *buf = malloc(cap);
    if (!buf) nx_trap(line, "out of memory");
    int c;
    while ((c = getchar()) != EOF && c != '\n') {
        if (n + 1 >= cap) {
            if (cap >= ((size_t)1 << 30)) { free(buf); nx_trap(line, "read_line: the line is longer than 1 GiB"); }
            char *nb = realloc(buf, cap * 2);
            if (!nb) { free(buf); nx_trap(line, "out of memory"); }
            buf = nb;
            cap *= 2;
        }
        buf[n++] = (char)c;
    }
    if (c == EOF && n == 0) { free(buf); return none; }
    if (n && buf[n - 1] == '\r') n--;
    nx_M_s r = {true, nx_str_lossy(buf, n)};
    free(buf);
    return r;
}
NX_RT bool nx_utf8_ok_raw(const char *s, size_t n) {
    for (size_t i = 0; i < n;) {
        int k = nx_utf8_letter((const unsigned char *)s + i, (int64_t)(n - i));
        if (k == 0) return false;
        i += (size_t)k;
    }
    return true;
}
NX_RT nx_arr *nx_b_args(int line) {
    nx_arr *a = nx_arr_new((int64_t)sizeof(nx_str), 0, nx_str_eretain, nx_str_erelease);
    for (int i = 1; i < nx_argc; i++) {
        size_t len = strlen(nx_argv[i]);
        nx_str s = nx_utf8_ok_raw(nx_argv[i], len) ? nx_str_lit(nx_argv[i], (int64_t)len) : nx_str_lossy(nx_argv[i], len);
        nx_arr_push(a, &s, line);
    }
    return a;
}

/* ---- bytes and letters ---- */
NX_RT int64_t nx_b_byte_at(nx_str s, int64_t i, int line) {
    if (i < 0 || i >= s.len)
        nx_trap(line, "byte_at: position %" PRId64 " is outside the text (length %" PRId64 ")", i, s.len);
    return (unsigned char)s.p[i];
}
/* The letter with Unicode number `c`, as UTF-8 text. */
NX_RT nx_str nx_b_char_from_code(int64_t c, int line) {
    char b[4];
    size_t n;
    if (c < 0 || c > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF))
        nx_trap(line, "char_from_code: %" PRId64 " isn't a Unicode letter number", c);
    if (c < 0x80) { b[0] = (char)c; n = 1; }
    else if (c < 0x800) { b[0] = (char)(0xC0 | (c >> 6)); b[1] = (char)(0x80 | (c & 0x3F)); n = 2; }
    else if (c < 0x10000) {
        b[0] = (char)(0xE0 | (c >> 12)); b[1] = (char)(0x80 | ((c >> 6) & 0x3F)); b[2] = (char)(0x80 | (c & 0x3F)); n = 3;
    } else {
        b[0] = (char)(0xF0 | (c >> 18)); b[1] = (char)(0x80 | ((c >> 12) & 0x3F));
        b[2] = (char)(0x80 | ((c >> 6) & 0x3F)); b[3] = (char)(0x80 | (c & 0x3F)); n = 4;
    }
    return nx_str_heap(b, n);
}
NX_RT void nx_b_exit(int64_t code, int line) {
    (void)line;
    fflush(stdout);
    exit(code < 0 || code > 255 ? 1 : (int)code);
}

/* ---- operating system primitives (only the std modules call these) ---- */
#define NX_MAX_FILE ((int64_t)256 << 20) /* files bigger than 256 MiB are refused */

/* A NUL-terminated copy of s for the C library (free it), or NULL if s is
   empty or contains a NUL byte. */
NX_RT char *nx_cstr(nx_str s) {
    if (s.len == 0 || memchr(s.p, 0, (size_t)s.len)) return NULL;
    char *c = malloc((size_t)s.len + 1);
    if (!c) nx_trap(0, "out of memory");
    memcpy(c, s.p, (size_t)s.len);
    c[s.len] = 0;
    return c;
}
NX_RT nx_str nx_fmt(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
NX_RT nx_str nx_fmt(const char *fmt, ...) {
    char buf[1024];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    if (n < 0) n = 0;
    if ((size_t)n >= sizeof buf) n = (int)sizeof buf - 1;
    return nx_str_heap(buf, (size_t)n);
}
/* "can't read notes.txt: it doesn't exist" */
NX_RT nx_str nx_os_err(const char *what, nx_str path, int e) {
    const char *why;
    switch (e) {
    case ENOENT: why = "it doesn't exist"; break;
    case EACCES: case EPERM: why = "permission denied (the app may not have access to that folder)"; break;
    case EISDIR: why = "it's a folder"; break;
    case ENOTDIR: why = "part of the path isn't a folder"; break;
    case ENOSPC: why = "the disk is full"; break;
    case EROFS: why = "the disk is read-only"; break;
    case EEXIST: why = "it already exists"; break;
    case ENOTEMPTY: why = "the folder isn't empty"; break;
    case ENAMETOOLONG: why = "the name is too long"; break;
    case ELOOP: why = "too many links in the path"; break;
    default: why = strerror(e); break;
    }
    int plen = path.len > 300 ? 300 : (int)path.len;
    return nx_fmt("can't %s %.*s: %s", what, plen, path.p, why);
}
#define NX_BAD_PATH(what, path) nx_fmt("can't %s: %s", what, (path).len == 0 ? "the path is empty" : "the path contains a zero byte")

/* Strict UTF-8 check: no overlong forms, surrogates or values past U+10FFFF. */
NX_RT bool nx_utf8_ok(const unsigned char *p, int64_t n) {
    for (int64_t i = 0; i < n;) {
        int k = nx_utf8_letter(p + i, n - i);
        if (k == 0) return false;
        i += k;
    }
    return true;
}

NX_RT nx_R_s nx_b___files_read(nx_str path, int line) {
    (void)line;
    nx_R_s r = {false, {"", 0, NULL}, {"", 0, NULL}};
    char *c = nx_cstr(path);
    if (!c) { r.e = NX_BAD_PATH("read", path); return r; }
    int fd = open(c, O_RDONLY | O_CLOEXEC);
    free(c);
    if (fd < 0) { r.e = nx_os_err("read", path, errno); return r; }
    struct stat st;
    if (fstat(fd, &st) == 0 && S_ISDIR(st.st_mode)) { close(fd); r.e = nx_os_err("read", path, EISDIR); return r; }
    size_t cap = 4096, n = 0;
    char *buf = malloc(cap);
    if (!buf) nx_trap(0, "out of memory");
    for (;;) {
        if (n == cap) {
            if ((int64_t)cap >= NX_MAX_FILE) {
                free(buf); close(fd);
                int plen = path.len > 300 ? 300 : (int)path.len;
                r.e = nx_fmt("can't read %.*s: it's bigger than 256 MiB", plen, path.p);
                return r;
            }
            char *nb = realloc(buf, cap * 2);
            if (!nb) { free(buf); nx_trap(0, "out of memory"); }
            buf = nb; cap *= 2;
        }
        ssize_t got = read(fd, buf + n, cap - n);
        if (got < 0) {
            if (errno == EINTR) continue;
            int e = errno; free(buf); close(fd);
            r.e = nx_os_err("read", path, e);
            return r;
        }
        if (got == 0) break;
        n += (size_t)got;
    }
    close(fd);
    if (!nx_utf8_ok((const unsigned char *)buf, (int64_t)n)) {
        free(buf);
        int plen = path.len > 300 ? 300 : (int)path.len;
        r.e = nx_fmt("can't read %.*s as text: it isn't UTF-8", plen, path.p);
        return r;
    }
    r.ok = true;
    r.v = nx_str_heap(buf, n);
    free(buf);
    return r;
}

NX_RT bool nx_write_all(int fd, nx_str s) {
    int64_t off = 0;
    while (off < s.len) {
        ssize_t w = write(fd, s.p + off, (size_t)(s.len - off));
        if (w < 0) { if (errno == EINTR) continue; return false; }
        off += w;
    }
    return true;
}

static int64_t nx_tmp_counter;

/* Write the whole file or nothing: write a temporary file next to it,
   flush it to disk, then rename it over the old one. */
NX_RT nx_R_v nx_b___files_write(nx_str path, nx_str text, int line) {
    (void)line;
    nx_R_v r = {false, {"", 0, NULL}};
    char *c = nx_cstr(path);
    if (!c) { r.e = NX_BAD_PATH("write", path); return r; }
    size_t tl = strlen(c) + 64;
    char *tmp = malloc(tl);
    if (!tmp) nx_trap(0, "out of memory");
    snprintf(tmp, tl, "%s.nexa-tmp-%ld-%" PRId64, c, (long)getpid(), nx_tmp_counter++);
    int fd = open(tmp, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0644);
    if (fd < 0) { int e = errno; free(c); free(tmp); r.e = nx_os_err("write", path, e); return r; }
    if (!nx_write_all(fd, text) || fsync(fd) != 0) {
        int e = errno; close(fd); unlink(tmp); free(c); free(tmp);
        r.e = nx_os_err("write", path, e);
        return r;
    }
    if (close(fd) != 0 || rename(tmp, c) != 0) {
        int e = errno; unlink(tmp); free(c); free(tmp);
        r.e = nx_os_err("write", path, e);
        return r;
    }
    free(c); free(tmp);
    r.ok = true;
    return r;
}

NX_RT nx_R_v nx_b___files_append(nx_str path, nx_str text, int line) {
    (void)line;
    nx_R_v r = {false, {"", 0, NULL}};
    char *c = nx_cstr(path);
    if (!c) { r.e = NX_BAD_PATH("add to", path); return r; }
    int fd = open(c, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0644);
    free(c);
    if (fd < 0) { r.e = nx_os_err("add to", path, errno); return r; }
    if (!nx_write_all(fd, text)) { int e = errno; close(fd); r.e = nx_os_err("add to", path, e); return r; }
    if (close(fd) != 0) { r.e = nx_os_err("add to", path, errno); return r; }
    r.ok = true;
    return r;
}

/* The size of a file in bytes, as decimal text (so it can use the runtime's
   existing `!str`; std/files.nx turns it into an int). Folders are refused. */
NX_RT nx_R_s nx_b___files_size(nx_str path, int line) {
    (void)line;
    nx_R_s r = {false, {"", 0, NULL}, {"", 0, NULL}};
    char *c = nx_cstr(path);
    if (!c) { r.e = NX_BAD_PATH("find the size of", path); return r; }
    struct stat st;
    int rc = stat(c, &st);
    int e = errno;
    free(c);
    if (rc != 0) { r.e = nx_os_err("find the size of", path, e); return r; }
    if (S_ISDIR(st.st_mode)) { r.e = nx_os_err("find the size of", path, EISDIR); return r; }
    r.v = nx_fmt("%lld", (long long)st.st_size);
    r.ok = true;
    return r;
}

NX_RT bool nx_b___files_exists(nx_str path, int line) {
    (void)line;
    char *c = nx_cstr(path);
    if (!c) return false;
    struct stat st;
    bool ok = stat(c, &st) == 0;
    free(c);
    return ok;
}
NX_RT bool nx_b___files_is_dir(nx_str path, int line) {
    (void)line;
    char *c = nx_cstr(path);
    if (!c) return false;
    struct stat st;
    bool ok = stat(c, &st) == 0 && S_ISDIR(st.st_mode);
    free(c);
    return ok;
}

NX_RT int nx_cmp_str_ptr(const void *a, const void *b) { return nx_str_cmp(*(const nx_str *)a, *(const nx_str *)b); }

/* Names in a folder, sorted; "." and ".." and names that aren't UTF-8 are left out. */
NX_RT nx_R_Ls nx_b___files_list(nx_str path, int line) {
    nx_R_Ls r = {false, NULL, {"", 0, NULL}};
    char *c = nx_cstr(path);
    if (!c) { r.e = NX_BAD_PATH("list", path); return r; }
    DIR *d = opendir(c);
    free(c);
    if (!d) { r.e = nx_os_err("list", path, errno); return r; }
    nx_arr *a = nx_arr_new((int64_t)sizeof(nx_str), 0, nx_str_eretain, nx_str_erelease);
    struct dirent *ent;
    while ((ent = readdir(d)) != NULL) {
        const char *n = ent->d_name;
        if (strcmp(n, ".") == 0 || strcmp(n, "..") == 0) continue;
        size_t len = strlen(n);
        if (!nx_utf8_ok((const unsigned char *)n, (int64_t)len)) continue;
        nx_str s = nx_str_heap(n, len);
        nx_arr_push(a, &s, line);
    }
    closedir(d);
    if (a->len > 1) qsort(a->data, (size_t)a->len, sizeof(nx_str), nx_cmp_str_ptr);
    r.ok = true;
    r.v = a;
    return r;
}

/* Make a folder and any missing folders above it. */
NX_RT nx_R_v nx_b___files_make_dir(nx_str path, int line) {
    (void)line;
    nx_R_v r = {false, {"", 0, NULL}};
    char *c = nx_cstr(path);
    if (!c) { r.e = NX_BAD_PATH("make the folder", path); return r; }
    size_t n = strlen(c);
    for (size_t i = 1; i <= n; i++) {
        if (c[i] != '/' && c[i] != 0) continue;
        char keep = c[i];
        c[i] = 0;
        if (mkdir(c, 0755) != 0 && errno != EEXIST) {
            int e = errno; free(c);
            r.e = nx_os_err("make the folder", path, e);
            return r;
        }
        c[i] = keep;
    }
    struct stat st;
    bool ok = stat(c, &st) == 0 && S_ISDIR(st.st_mode);
    free(c);
    if (!ok) { r.e = nx_os_err("make the folder", path, EEXIST); return r; }
    r.ok = true;
    return r;
}

/* Remove a file, or an empty folder. Never removes a folder's contents. */
NX_RT nx_R_v nx_b___files_remove(nx_str path, int line) {
    (void)line;
    nx_R_v r = {false, {"", 0, NULL}};
    char *c = nx_cstr(path);
    if (!c) { r.e = NX_BAD_PATH("remove", path); return r; }
    struct stat st;
    int rc;
    if (lstat(c, &st) == 0 && S_ISDIR(st.st_mode)) rc = rmdir(c);
    else rc = unlink(c);
    int e = errno;
    free(c);
    if (rc != 0) { r.e = nx_os_err("remove", path, e); return r; }
    r.ok = true;
    return r;
}

NX_RT nx_M_s nx_b___env(nx_str name, int line) {
    (void)line;
    nx_M_s r = {false, {"", 0, NULL}};
    char *c = nx_cstr(name);
    if (!c) return r;
    const char *v = getenv(c);
    free(c);
    if (!v) return r;
    size_t len = strlen(v);
    if (!nx_utf8_ok((const unsigned char *)v, (int64_t)len)) return r;
    r.has = true;
    r.v = nx_str_heap(v, len);
    return r;
}

/* ---- time ---- */
NX_RT int64_t nx_b___time_unix(int line) { (void)line; return (int64_t)time(NULL); }
NX_RT int64_t nx_b___time_millis(int line) {
    (void)line;
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}
NX_RT void nx_b___time_sleep_ms(int64_t ms, int line) {
    (void)line;
    if (ms <= 0) return;
    struct timespec ts = {(time_t)(ms / 1000), (long)(ms % 1000) * 1000000L};
    while (nanosleep(&ts, &ts) != 0 && errno == EINTR) {}
}
NX_RT nx_R_s nx_b___time_format(int64_t t, nx_str fmt, bool utc, int line) {
    (void)line;
    nx_R_s r = {false, {"", 0, NULL}, {"", 0, NULL}};
    char *f = nx_cstr(fmt);
    if (!f) { r.e = nx_str_lit("the time format is empty or has a zero byte", 43); return r; }
    time_t tt = (time_t)t;
    struct tm tm;
    if ((utc ? gmtime_r(&tt, &tm) : localtime_r(&tt, &tm)) == NULL) {
        free(f);
        r.e = nx_str_lit("that time can't be shown as a date", 34);
        return r;
    }
    char buf[512];
    size_t n = strftime(buf, sizeof buf, f, &tm);
    free(f);
    if (n == 0) { r.e = nx_str_lit("the formatted time would be empty or too long", 45); return r; }
    r.ok = true;
    r.v = nx_str_heap(buf, n);
    return r;
}

/* ---- one request over a unix socket: send a line, read one line back ---- */
/* One request over a Unix socket, done by plain C that touches no
   NexaLang memory (no nx_alloc, no counts, no traps), so a window can run
   it on a helper thread and keep drawing meanwhile (see nx_run_blocking).
   The message is borrowed: no NexaLang code runs until the job is done. */
typedef struct {
    struct sockaddr_un addr;
    const char *msg; size_t msglen;
    int64_t timeout_ms;
    char *buf; size_t n; /* the reply (malloc'd), when stage == 0 */
    int stage, err;      /* stage: 0 done, 1 socket, 2 connect, 3 send, 4 read, 5 too long, 6 no memory */
} nx_sock_job;

__attribute__((unused)) static void nx_sock_job_run(void *arg) {
    nx_sock_job *j = arg;
    j->buf = NULL; j->n = 0; j->err = 0;
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) { j->stage = 1; j->err = errno; return; }
    if (j->timeout_ms > 0) {
        struct timeval tv = {(time_t)(j->timeout_ms / 1000), (suseconds_t)((j->timeout_ms % 1000) * 1000)};
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    }
    if (connect(fd, (struct sockaddr *)&j->addr, sizeof j->addr) != 0) { j->stage = 2; j->err = errno; close(fd); return; }
    size_t off = 0;
    while (off < j->msglen) {
        ssize_t w = send(fd, j->msg + off, j->msglen - off, MSG_NOSIGNAL);
        if (w < 0) { if (errno == EINTR) continue; j->stage = 3; j->err = errno; close(fd); return; }
        off += (size_t)w;
    }
    size_t cap = 4096, n = 0;
    char *buf = malloc(cap);
    if (!buf) { j->stage = 6; close(fd); return; }
    for (;;) {
        if (n == cap) {
            if (cap >= ((size_t)1 << 20)) { free(buf); close(fd); j->stage = 5; return; }
            char *nb = realloc(buf, cap * 2);
            if (!nb) { free(buf); close(fd); j->stage = 6; return; }
            buf = nb; cap *= 2;
        }
        ssize_t got = recv(fd, buf + n, cap - n, 0);
        if (got < 0) {
            if (errno == EINTR) continue;
            j->stage = 4; j->err = errno; free(buf); close(fd); return;
        }
        if (got == 0) break;
        char *nl = memchr(buf + n, '\n', (size_t)got);
        n += (size_t)got;
        if (nl) { n = (size_t)(nl - buf); break; }
    }
    close(fd);
    j->stage = 0; j->buf = buf; j->n = n;
}

/* How a blocking job runs: directly (NULL), or, once a window is open, on
   a helper thread while the window keeps drawing (set by the ui runtime). */
__attribute__((unused)) static void (*nx_run_blocking)(void (*fn)(void *), void *arg);

NX_RT nx_R_s nx_b___unix_request(nx_str path, nx_str msg, int64_t timeout_ms, int line) {
    (void)line;
    nx_R_s r = {false, {"", 0, NULL}, {"", 0, NULL}};
    nx_sock_job j;
    memset(&j, 0, sizeof j);
    j.addr.sun_family = AF_UNIX;
    if (path.len <= 0 || path.len >= (int64_t)sizeof j.addr.sun_path || memchr(path.p, 0, (size_t)path.len)) {
        r.e = nx_str_lit("the socket path is empty, too long or has a zero byte", 53);
        return r;
    }
    memcpy(j.addr.sun_path, path.p, (size_t)path.len);
    j.msg = msg.p; j.msglen = (size_t)msg.len; j.timeout_ms = timeout_ms;
    if (nx_run_blocking) nx_run_blocking(nx_sock_job_run, &j);
    else nx_sock_job_run(&j);
    switch (j.stage) {
    case 0: break;
    case 1: r.e = nx_os_err("open a socket for", path, j.err); return r;
    case 2: r.e = nx_os_err("connect to", path, j.err); return r;
    case 3: r.e = nx_os_err("send to", path, j.err); return r;
    case 4:
        r.e = (j.err == EAGAIN || j.err == EWOULDBLOCK) ? nx_str_lit("no reply in time", 16) : nx_os_err("read the reply from", path, j.err);
        return r;
    case 5: r.e = nx_str_lit("the reply is longer than 1 MiB", 30); return r;
    default: nx_trap(0, "out of memory");
    }
    if (!nx_utf8_ok((const unsigned char *)j.buf, (int64_t)j.n)) { free(j.buf); r.e = nx_str_lit("the reply isn't UTF-8 text", 26); return r; }
    r.ok = true;
    r.v = nx_str_heap(j.buf, j.n);
    free(j.buf);
    return r;
}
