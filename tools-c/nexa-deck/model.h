/* nexa-deck: the parts with no screen in them, unit-tested by deck-test.c. */
#ifndef NEXA_DECK_MODEL_H
#define NEXA_DECK_MODEL_H
#include <stddef.h>

/* `wpctl get-volume @DEFAULT_AUDIO_SINK@`: "Volume: 0.45 [MUTED]".
 * 0 on success. */
int parse_volume(const char *out, int *percent, int *muted);

struct note {
    int id;
    char app[64], summary[160], body[320];
};
/* `makoctl list` / `makoctl history` (JSON) added to notes[] (skipping ids
 * already there and ids at or below cleared_up_to), newest first. Returns
 * the new count. */
int parse_mako(const char *json, int cleared_up_to, struct note *notes, int count, int max);

struct battery {
    int percent;
    int charging, full;   /* status Charging / Full */
    int minutes;          /* to empty, or to full while charging; -1 unknown */
};
/* The first BAT* under root (/sys/class/power_supply). 0, or -1 if none. */
int battery_read(const char *root, struct battery *b);
/* "2 h 10 min left", "Full in 40 min", "Charging", "Full", "" */
void battery_text(const struct battery *b, char *out, size_t cap);

/* The first backlight under root (/sys/class/backlight): percent. -1 if none. */
int backlight_read(const char *root);

/* iwd's signal (dBm * 100) as 0..100, and as 0..4 bars. */
int signal_percent(int raw);
int signal_bars(int percent);

/* After a byte-limited copy: drop a last character that was cut short. */
void utf8_cut(char *s);

/* The broker's tunnel names: 1..15 of [A-Za-z0-9_.-]. */
int tunnel_ok(const char *name);
/* One request line for /run/nexa-broker.sock. name may be NULL, percent
 * < 0 leaves it out. 0, or -1 if a name is unsafe or it doesn't fit. */
int broker_request(char *out, size_t cap, const char *action, const char *name, int percent);
/* The reply's "ok"; on failure its "error" in err. */
int broker_ok(const char *reply, char *err, size_t cap);

struct tunnel {
    char name[16];
    int active;
};
/* vpn_list's reply -> tunnels. Count, or -1. */
int parse_tunnels(const char *reply, struct tunnel *out, int max);
#endif
