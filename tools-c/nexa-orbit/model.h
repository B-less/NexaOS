/* nexa-orbit: the parts with no screen in them (config, /proc, Exec
 * lines, app ids), so they can be unit-tested (orbit-test.c). */
#ifndef NEXA_ORBIT_MODEL_H
#define NEXA_ORBIT_MODEL_H
#include <stddef.h>

#define ORBIT_MAX_PINNED 32
#define ORBIT_ID_LEN 128

struct orbit_conf {
    char pinned[ORBIT_MAX_PINNED][ORBIT_ID_LEN];
    int npinned;
    int fold;  /* 1: fold into a line on the edge, keep no space */
};

/* The pinned apps of a new account. */
void conf_defaults(struct orbit_conf *c);
/* key=value lines; unknown keys and bad ids are skipped. No "pinned" line
 * keeps the defaults; an empty one means nothing pinned. */
void conf_parse(struct orbit_conf *c, const char *text);
/* The file's text; 0, or -1 if it doesn't fit. */
int conf_format(const struct orbit_conf *c, char *out, size_t cap);
int conf_find(const struct orbit_conf *c, const char *id);
/* 0 on success, -1 if full / already there / not found / bad id. */
int conf_pin(struct orbit_conf *c, const char *id);
int conf_unpin(struct orbit_conf *c, const char *id);

/* An app id or desktop id safe to put in a compositor command and a file
 * name: 1..127 characters of [A-Za-z0-9._-], not starting with '.' or '-'. */
int id_ok(const char *id);

/* One /proc/<pid>/stat line: parent pid and CPU ticks (utime + stime).
 * The name in parentheses may hold spaces and ')'. 0 on success. */
int parse_stat(const char *line, int *ppid, unsigned long long *ticks);

/* Percent of one core from two tick counts dt seconds apart. */
double cpu_percent(unsigned long long before, unsigned long long after, double dt, long clk_tck);

/* Exec= of a desktop file without its field codes (%f, %U, ...), "%%"
 * becoming "%". 0, or -1 if it doesn't fit. */
int exec_strip(const char *exec, char *out, size_t cap);

/* Window app_id -> desktop id for apps whose names differ. */
const char *alias_of(const char *app_id);
#endif
