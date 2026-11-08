/* Text to Kokoro tokens. See kokoro.h. */
#include "kokoro.h"

#include <ctype.h>
#include <stdlib.h>
#include <string.h>

/* ---------- small string helpers ---------- */

typedef struct { char *s; size_t n, cap; } buf;

static void put(buf *b, const char *s, size_t n)
{
    if (b->n + n + 1 > b->cap) {
        b->cap = (b->n + n + 1) * 2;
        b->s = realloc(b->s, b->cap);
        if (!b->s) abort();
    }
    memcpy(b->s + b->n, s, n);
    b->n += n;
    b->s[b->n] = 0;
}

static void puts_(buf *b, const char *s) { put(b, s, strlen(s)); }

static char *done(buf *b)
{
    if (!b->s) put(b, "", 0);
    return b->s;
}

/* Every `from` replaced by `to`, in a new string. */
static char *replace(char *s, const char *from, const char *to)
{
    size_t fl = strlen(from);
    if (!strstr(s, from)) return s;
    buf b = {0};
    for (const char *p = s;;) {
        const char *hit = strstr(p, from);
        if (!hit) { puts_(&b, p); break; }
        put(&b, p, hit - p);
        puts_(&b, to);
        p = hit + fl;
    }
    free(s);
    return done(&b);
}

/* One UTF-8 character: its code point and length (bad bytes: 0xFFFD, 1). */
static unsigned utf8(const unsigned char *s, int *len)
{
    unsigned c = s[0];
    int n = c < 0x80 ? 1 : (c >> 5) == 6 ? 2 : (c >> 4) == 14 ? 3 : (c >> 3) == 30 ? 4 : 0;
    if (!n) { *len = 1; return 0xFFFD; }
    if (n == 1) { *len = 1; return c; }
    unsigned v = c & (0x7F >> n);
    for (int i = 1; i < n; i++) {
        if ((s[i] & 0xC0) != 0x80) { *len = 1; return 0xFFFD; }
        v = (v << 6) | (s[i] & 0x3F);
    }
    *len = n;
    return v;
}

/* ---------- cleaning ---------- */

static int is_url_start(const char *p)
{
    return !strncmp(p, "http://", 7) || !strncmp(p, "https://", 8) || !strncmp(p, "www.", 4);
}

char *kokoro_clean(const char *in)
{
    buf out = {0};
    int fence = 0;
    const char *line = in;
    while (line && *line) {
        const char *end = strchr(line, '\n');
        size_t len = end ? (size_t)(end - line) : strlen(line);
        const char *p = line;
        while (p < line + len && (*p == ' ' || *p == '\t')) p++;
        if (!strncmp(p, "```", 3)) {          /* a code block isn't read out */
            fence = !fence;
            if (fence) puts_(&out, " There's some code here. ");
        } else if (!fence) {
            /* list marks and headings at the start of a line */
            if ((*p == '-' || *p == '*' || *p == '+') && p[1] == ' ') p += 2;
            while (*p == '#' || *p == '>') p++;
            buf l = {0};
            for (const char *q = p; q < line + len;) {
                if (is_url_start(q)) {
                    put(&l, "a link", 6);
                    while (q < line + len && !isspace((unsigned char)*q)) q++;
                    continue;
                }
                if (*q == '[') {               /* [text](url): the text */
                    const char *close = memchr(q, ']', line + len - q);
                    if (close && close + 1 < line + len && close[1] == '(') {
                        const char *rp = memchr(close, ')', line + len - close);
                        if (rp) { put(&l, q + 1, close - q - 1); q = rp + 1; continue; }
                    }
                }
                if (strchr("*_`~|#\\", *q)) { q++; continue; }
                if (*q == '\t' || *q == '\r') { put(&l, " ", 1); q++; continue; }
                put(&l, q, 1);
                q++;
            }
            char *t = done(&l);
            size_t tl = strlen(t);
            while (tl && t[tl - 1] == ' ') t[--tl] = 0;
            if (tl) {
                put(&out, " ", 1);
                put(&out, t, tl);
                /* a line without an ending reads as the end of a sentence */
                if (!strchr(".!?:;,", t[tl - 1])) put(&out, ".", 1);
            }
            free(t);
        }
        line = end ? end + 1 : NULL;
    }
    /* collapse spaces */
    char *s = done(&out);
    buf c = {0};
    int space = 1;
    for (char *p = s; *p; p++) {
        if (*p == ' ') { if (!space) put(&c, " ", 1); space = 1; continue; }
        put(&c, p, 1);
        space = 0;
    }
    free(s);
    char *r = done(&c);
    size_t rl = strlen(r);
    while (rl && r[rl - 1] == ' ') r[--rl] = 0;
    return replace(r, "...", "\xe2\x80\xa6");
}

/* ---------- pieces ---------- */

static const char *const ABBREV[] = {
    "mr", "mrs", "ms", "dr", "st", "vs", "etc", "e.g", "i.e", "eg", "ie", "jr", "sr", "prof", "no", "approx", "inc", "ltd", NULL,
};

static int abbreviation(const char *start, const char *dot)
{
    const char *w = dot;
    while (w > start && !isspace((unsigned char)w[-1])) w--;
    size_t n = dot - w;
    if (n == 1 && isalpha((unsigned char)*w)) return 1;    /* an initial: "J. Smith" */
    char low[16];
    if (n == 0 || n >= sizeof low) return 0;
    for (size_t i = 0; i < n; i++) low[i] = tolower((unsigned char)w[i]);
    low[n] = 0;
    for (int i = 0; ABBREV[i]; i++)
        if (!strcmp(low, ABBREV[i])) return 1;
    return 0;
}

/* The punctuation mark at p (one of ours), its length, or 0. */
static int mark_at(const char *p, char *out)
{
    static const char *const MARKS[] = {",", ".", "!", "?", ";", ":", "\xe2\x80\x94" /* — */, "\xe2\x80\xa6" /* … */, NULL};
    for (int i = 0; MARKS[i]; i++) {
        size_t n = strlen(MARKS[i]);
        if (!strncmp(p, MARKS[i], n)) {
            if (out) memcpy(out, MARKS[i], n + 1);
            return (int)n;
        }
    }
    return 0;
}

int kokoro_pieces(const char *clean, kokoro_piece **out)
{
    int n = 0, cap = 8;
    kokoro_piece *v = calloc(cap, sizeof *v);
    if (!v) abort();
    const char *start = clean;
    for (const char *p = clean;; p++) {
        char mark[8] = "";
        int ml = *p ? mark_at(p, mark) : 0;
        int at_end = !*p;
        if (ml) {
            const char *after = p + ml;
            /* a mark ends a piece only before a space or the end:
             * "6:30", "3.5" and "nexa.os" stay whole */
            int cut = !*after || *after == ' ' || ml > 1 || mark_at(after, NULL);
            if (cut && mark[0] == '.' && abbreviation(start, p)) cut = 0;
            if (!cut) continue;
        } else if (!at_end) {
            continue;
        }
        const char *e = p;
        while (e > start && e[-1] == ' ') e--;
        const char *s = start;
        while (s < e && *s == ' ') s++;
        if (e > s || (ml && n)) {
            if (e == s && n) {           /* "?!" or "..." : add to the last */
                size_t l = strlen(v[n - 1].punct);
                if (l + strlen(mark) < sizeof v[n - 1].punct) strcat(v[n - 1].punct, mark);
            } else {
                if (n == cap) {
                    cap *= 2;
                    v = realloc(v, cap * sizeof *v);
                    if (!v) abort();
                }
                v[n].words = strndup(s, e - s);
                strcpy(v[n].punct, mark);
                v[n].sentence_end = 0;
                n++;
            }
            if (at_end || strchr(".!?", mark[0]) || !strcmp(mark, "\xe2\x80\xa6")) v[n - 1].sentence_end = 1;
        }
        if (at_end) break;
        start = p + ml;
        p += ml - 1;
    }
    if (n) v[n - 1].sentence_end = 1;
    *out = v;
    return n;
}

void kokoro_pieces_free(kokoro_piece *p, int n)
{
    for (int i = 0; i < n; i++) free(p[i].words);
    free(p);
}

/* ---------- espeak IPA to misaki ---------- */

/* misaki's E2M table, applied in this order (longest first, as it does). */
static const char *const E2M[][2] = {
    {"\xca\x94\xcb\x8cn\xcc\xa9", "t\xe1\xb5\x8an"},  /* ʔˌn̩ → tᵊn */
    {"\xca\x94n\xcc\xa9", "t\xe1\xb5\x8an"},          /* ʔn̩ */
    {"a^\xc9\xaa", "I"},                              /* a^ɪ */
    {"a^\xca\x8a", "W"},                              /* a^ʊ */
    {"d^\xca\x92", "\xca\xa4"},                       /* d^ʒ → ʤ */
    {"e^\xc9\xaa", "A"},                              /* e^ɪ */
    {"t^\xca\x83", "\xca\xa7"},                       /* t^ʃ → ʧ */
    {"\xc9\x94^\xc9\xaa", "Y"},                       /* ɔ^ɪ */
    {"\xc9\x99^l", "\xe1\xb5\x8al"},                  /* ə^l → ᵊl */
    {"\xca\x94n", "t\xe1\xb5\x8an"},                  /* ʔn */
    {"\xca\xb2o", "jo"},                              /* ʲo */
    {"\xca\xb2\xc9\x99", "j\xc9\x99"},                /* ʲə */
    {"\xca\x94", "t"},                                /* ʔ */
    {"e", "A"},
    {"\xca\xb2", ""},                                 /* ʲ */
    {"\xc9\x9a", "\xc9\x99\xc9\xb9"},                 /* ɚ → əɹ */
    {"r", "\xc9\xb9"},                                /* r → ɹ */
    {"x", "k"},
    {"\xc3\xa7", "k"},                                /* ç */
    {"\xc9\x90", "\xc9\x99"},                         /* ɐ → ə */
    {"\xc9\xac", "l"},                                /* ɬ */
    {"\xcc\x83", ""},                                 /* nasal tilde */
};

char *kokoro_from_espeak(const char *ipa, int british)
{
    char *s = strdup(ipa);
    if (!s) abort();
    /* lines (clauses) joined by spaces */
    for (char *p = s; *p; p++)
        if (*p == '\n' || *p == '\r' || *p == '\t') *p = ' ';
    for (size_t i = 0; i < sizeof E2M / sizeof *E2M; i++) s = replace(s, E2M[i][0], E2M[i][1]);
    /* a syllabic consonant (X + U+0329) becomes ᵊX */
    buf b = {0};
    for (const char *p = s; *p;) {
        int l1, l2;
        utf8((const unsigned char *)p, &l1);
        if (p[l1] && (unsigned char)p[l1] == 0xCC && (unsigned char)p[l1 + 1] == 0xA9 && *p != ' ') {
            put(&b, "\xe1\xb5\x8a", 3);
            put(&b, p, l1);
            p += l1 + 2;
            continue;
        }
        unsigned c = utf8((const unsigned char *)p, &l2);
        if (c != 0x329) put(&b, p, l2);
        p += l2;
    }
    free(s);
    s = done(&b);
    if (british) {
        s = replace(s, "e^\xc9\x99", "\xc9\x9b\xcb\x90");  /* e^ə → ɛː */
        s = replace(s, "i\xc9\x99", "\xc9\xaa\xc9\x99");     /* iə → ɪə */
        s = replace(s, "\xc9\x99^\xca\x8a", "Q");            /* ə^ʊ → Q */
    } else {
        s = replace(s, "o^\xca\x8a", "O");                    /* o^ʊ → O */
        s = replace(s, "\xc9\x9c\xcb\x90\xc9\xb9", "\xc9\x9c\xc9\xb9"); /* ɜːɹ → ɜɹ */
        s = replace(s, "\xc9\x9c\xcb\x90", "\xc9\x9c\xc9\xb9");  /* ɜː → ɜɹ */
        s = replace(s, "\xc9\xaa\xc9\x99", "i\xc9\x99");     /* ɪə → iə */
        s = replace(s, "\xcb\x90", "");                       /* no length marks */
    }
    s = replace(s, "^", "");
    /* tidy spaces */
    buf c = {0};
    int space = 1;
    for (char *p = s; *p; p++) {
        if (*p == ' ') { if (!space) put(&c, " ", 1); space = 1; continue; }
        put(&c, p, 1);
        space = 0;
    }
    free(s);
    char *r = done(&c);
    size_t rl = strlen(r);
    while (rl && r[rl - 1] == ' ') r[--rl] = 0;
    return r;
}

/* ---------- tokens ---------- */

/* Kokoro-82M v1.0's vocabulary (tokenizer.json), by code point. */
static const struct { unsigned cp; int id; } VOCAB[] = {
    {';', 1}, {':', 2}, {',', 3}, {'.', 4}, {'!', 5}, {'?', 6}, {0x2014, 9}, {0x2026, 10}, {'"', 11},
    {'(', 12}, {')', 13}, {0x201c, 14}, {0x201d, 15}, {' ', 16}, {0x303, 17}, {0x2a3, 18}, {0x2a5, 19},
    {0x2a6, 20}, {0x2a8, 21}, {0x1d5d, 22}, {0xab67, 23}, {'A', 24}, {'I', 25}, {'O', 31}, {'Q', 33},
    {'S', 35}, {'T', 36}, {'W', 39}, {'Y', 41}, {0x1d4a, 42}, {'a', 43}, {'b', 44}, {'c', 45}, {'d', 46},
    {'e', 47}, {'f', 48}, {'h', 50}, {'i', 51}, {'j', 52}, {'k', 53}, {'l', 54}, {'m', 55}, {'n', 56},
    {'o', 57}, {'p', 58}, {'q', 59}, {'r', 60}, {'s', 61}, {'t', 62}, {'u', 63}, {'v', 64}, {'w', 65},
    {'x', 66}, {'y', 67}, {'z', 68}, {0x251, 69}, {0x250, 70}, {0x252, 71}, {0xe6, 72}, {0x3b2, 75},
    {0x254, 76}, {0x255, 77}, {0xe7, 78}, {0x256, 80}, {0xf0, 81}, {0x2a4, 82}, {0x259, 83}, {0x25a, 85},
    {0x25b, 86}, {0x25c, 87}, {0x25f, 90}, {0x261, 92}, {0x265, 99}, {0x268, 101}, {0x26a, 102},
    {0x29d, 103}, {0x26f, 110}, {0x270, 111}, {0x14b, 112}, {0x273, 113}, {0x272, 114}, {0x274, 115},
    {0xf8, 116}, {0x278, 118}, {0x3b8, 119}, {0x153, 120}, {0x279, 123}, {0x27e, 125}, {0x27b, 126},
    {0x281, 128}, {0x27d, 129}, {0x282, 130}, {0x283, 131}, {0x288, 132}, {0x2a7, 133}, {0x28a, 135},
    {0x28b, 136}, {0x28c, 138}, {0x263, 139}, {0x264, 140}, {0x3c7, 142}, {0x28e, 143}, {0x292, 147},
    {0x294, 148}, {0x2c8, 156}, {0x2cc, 157}, {0x2d0, 158}, {0x2b0, 162}, {0x2b2, 164}, {0x2193, 169},
    {0x2192, 171}, {0x2197, 172}, {0x2198, 173}, {0x1d7b, 177},
};

static int token_of(unsigned cp)
{
    for (size_t i = 0; i < sizeof VOCAB / sizeof *VOCAB; i++)
        if (VOCAB[i].cp == cp) return VOCAB[i].id;
    return -1;
}

int kokoro_ids(const char *ph, int64_t *ids, int cap)
{
    int n = 0;
    for (const unsigned char *p = (const unsigned char *)ph; *p;) {
        int l;
        int id = token_of(utf8(p, &l));
        p += l;
        if (id < 0) continue;
        if (n < cap && ids) ids[n] = id;
        n++;
    }
    return n;
}

size_t kokoro_cut(const char *s, int max_tokens)
{
    int n = 0;
    size_t last_space = 0;
    for (const unsigned char *p = (const unsigned char *)s; *p;) {
        int l;
        unsigned cp = utf8(p, &l);
        if (token_of(cp) >= 0) {
            if (n == max_tokens) return last_space ? last_space : (size_t)((const char *)p - s);
            n++;
        }
        if (cp == ' ') last_space = (const char *)p - s;
        p += l;
    }
    return strlen(s);
}
