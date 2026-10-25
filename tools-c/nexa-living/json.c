/* See json.h. Enough JSON for the compositor's and our own files: no
 * \u escapes beyond ASCII (they become '?'), depth limited. */
#include "json.h"

#include <ctype.h>
#include <stdlib.h>
#include <string.h>

struct st {
    char *p;
    struct jdoc *d;
    int depth;
};

static int add(struct jdoc *d, int type)
{
    if (d->count == d->cap) {
        int cap = d->cap ? d->cap * 2 : 256;
        struct jnode *n = realloc(d->n, (size_t)cap * sizeof *n);
        if (!n) return -1;
        d->n = n;
        d->cap = cap;
    }
    d->n[d->count] = (struct jnode){.type = type, .child = -1, .next = -1};
    return d->count++;
}

static void ws(struct st *s)
{
    while (*s->p && isspace((unsigned char)*s->p)) s->p++;
}

/* Reads a string at s->p (after the quote), NUL-terminates it in place. */
static char *string(struct st *s)
{
    char *out = s->p, *w = s->p;
    while (*s->p && *s->p != '"') {
        if (*s->p == '\\' && s->p[1]) {
            s->p++;
            char c = *s->p;
            if (c == 'n') *w++ = '\n';
            else if (c == 't') *w++ = '\t';
            else if (c == 'u') {
                *w++ = '?';
                for (int i = 0; i < 4 && s->p[1]; i++) s->p++;
            } else *w++ = c;
            s->p++;
        } else {
            *w++ = *s->p++;
        }
    }
    if (*s->p != '"') return NULL;
    s->p++;
    *w = 0;
    return out;
}

static int value(struct st *s);

static int container(struct st *s, int type, char close)
{
    int me = add(s->d, type), last = -1;
    if (me < 0 || ++s->depth > 64) return -1;
    s->p++;
    ws(s);
    if (*s->p == close) {
        s->p++;
        s->depth--;
        return me;
    }
    for (;;) {
        const char *key = NULL;
        ws(s);
        if (type == J_OBJ) {
            if (*s->p != '"') return -1;
            s->p++;
            key = string(s);
            ws(s);
            if (!key || *s->p != ':') return -1;
            s->p++;
        }
        int v = value(s);
        if (v < 0) return -1;
        s->d->n[v].key = key;
        if (last < 0) s->d->n[me].child = v;
        else s->d->n[last].next = v;
        last = v;
        ws(s);
        if (*s->p == ',') { s->p++; continue; }
        if (*s->p == close) { s->p++; s->depth--; return me; }
        return -1;
    }
}

static int value(struct st *s)
{
    ws(s);
    char c = *s->p;
    if (c == '{') return container(s, J_OBJ, '}');
    if (c == '[') return container(s, J_ARR, ']');
    if (c == '"') {
        s->p++;
        char *str = string(s);
        if (!str) return -1;
        int me = add(s->d, J_STR);
        if (me >= 0) s->d->n[me].str = str;
        return me;
    }
    if (!strncmp(s->p, "true", 4) || !strncmp(s->p, "false", 5)) {
        int me = add(s->d, J_BOOL);
        if (me >= 0) s->d->n[me].num = c == 't';
        s->p += c == 't' ? 4 : 5;
        return me;
    }
    if (!strncmp(s->p, "null", 4)) {
        s->p += 4;
        return add(s->d, J_NULL);
    }
    char *end;
    double v = strtod(s->p, &end);
    if (end == s->p) return -1;
    s->p = end;
    int me = add(s->d, J_NUM);
    if (me >= 0) s->d->n[me].num = v;
    return me;
}

int json_parse(struct jdoc *d, char *text)
{
    d->count = 0;
    struct st s = {.p = text, .d = d};
    return value(&s) == 0 ? 0 : -1;
}

void json_free(struct jdoc *d)
{
    free(d->n);
    d->n = NULL;
    d->count = d->cap = 0;
}

int json_get(const struct jdoc *d, int o, const char *key)
{
    if (o < 0 || o >= d->count || d->n[o].type != J_OBJ) return -1;
    for (int c = d->n[o].child; c >= 0; c = d->n[c].next)
        if (d->n[c].key && !strcmp(d->n[c].key, key)) return c;
    return -1;
}

const char *json_str(const struct jdoc *d, int o, const char *key, const char *fallback)
{
    int c = json_get(d, o, key);
    return c >= 0 && d->n[c].type == J_STR ? d->n[c].str : fallback;
}

double json_num(const struct jdoc *d, int o, const char *key, double fallback)
{
    int c = json_get(d, o, key);
    return c >= 0 && (d->n[c].type == J_NUM || d->n[c].type == J_BOOL) ? d->n[c].num : fallback;
}
