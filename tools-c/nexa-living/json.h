/* A small JSON reader for nexa-living: the settings file, Sway's tree,
 * Hyprland's client list. Parses into a flat array of nodes (no
 * allocation per node); strings point into the (modified) input. */
#ifndef NEXA_LIVING_JSON_H
#define NEXA_LIVING_JSON_H
#include <stddef.h>

enum { J_NULL, J_BOOL, J_NUM, J_STR, J_ARR, J_OBJ };

struct jnode {
    int type;
    const char *key;     /* in an object: this member's name */
    const char *str;     /* J_STR */
    double num;          /* J_NUM, J_BOOL (0/1) */
    int child, next;     /* indexes, -1 for none */
};

struct jdoc {
    struct jnode *n;
    int count, cap;
};

/* Parses text in place (it's modified). 0 on success; the root is node 0. */
int json_parse(struct jdoc *d, char *text);
void json_free(struct jdoc *d);
/* A member of object o, or -1. */
int json_get(const struct jdoc *d, int o, const char *key);
const char *json_str(const struct jdoc *d, int o, const char *key, const char *fallback);
double json_num(const struct jdoc *d, int o, const char *key, double fallback);
#endif
