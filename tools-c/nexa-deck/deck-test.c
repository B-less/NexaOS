/* Unit tests for nexa-deck's model (make test). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "model.h"

static int fails;
#define CHECK(c, what) do { if (c) printf("PASS %s\n", what); else { printf("FAIL %s\n", what); fails++; } } while (0)

static void put(const char *dir, const char *name, const char *text)
{
    char p[600];
    snprintf(p, sizeof p, "%s/%s", dir, name);
    FILE *f = fopen(p, "w");
    fputs(text, f);
    fclose(f);
}

int main(void)
{
    int pct, muted;
    CHECK(parse_volume("Volume: 0.45\n", &pct, &muted) == 0 && pct == 45 && !muted, "volume");
    CHECK(parse_volume("Volume: 1.20 [MUTED]\n", &pct, &muted) == 0 && pct == 120 && muted, "volume muted, above 100 %");
    CHECK(parse_volume("", &pct, &muted) == -1 && parse_volume(NULL, &pct, &muted) == -1 && parse_volume("Volume: x", &pct, &muted) == -1,
          "volume: nothing usable");

    struct note n[8];
    const char *hist = "{\"type\": \"aa{sv}\", \"data\": [[{\"app-name\": {\"type\": \"s\", \"data\": \"Music\"}, \"summary\": {\"type\": \"s\", "
                       "\"data\": \"Caf\\u00e9 \\ud83c\\udfb5\"}, \"body\": {\"type\": \"s\", \"data\": \"Next: Song\"}, \"id\": {\"type\": \"u\", \"data\": 7}},"
                       "{\"app-name\": {\"type\": \"s\", \"data\": \"Files\"}, \"summary\": {\"type\": \"s\", \"data\": \"Copied\"}, \"id\": {\"type\": \"u\", \"data\": 3}}]]}";
    int c = parse_mako(hist, 0, n, 0, 8);
    CHECK(c == 2 && n[0].id == 7 && !strcmp(n[0].app, "Music") && !strcmp(n[1].summary, "Copied") && n[1].body[0] == 0,
          "mako history: newest first, missing body empty");
    CHECK(!strcmp(n[0].summary, "Caf\xc3\xa9 \xf0\x9f\x8e\xb5"), "mako: \\u escapes and surrogate pairs become UTF-8");
    const char *list = "{\"data\": [{\"id\": {\"data\": 9}, \"summary\": {\"data\": \"Now\"}, \"app-name\": {\"data\": \"Store\"}},"
                       "{\"id\": {\"data\": 7}, \"summary\": {\"data\": \"dup\"}}]}";
    c = parse_mako(list, 0, n, c, 8);
    CHECK(c == 3 && n[0].id == 9 && n[1].id == 7 && !strcmp(n[1].app, "Music"), "mako list merged, a repeated id kept once");
    c = parse_mako(hist, 5, n, 0, 8);
    CHECK(c == 1 && n[0].id == 7, "mako: cleared ids skipped");
    CHECK(parse_mako("not json", 0, n, 2, 8) == 2 && parse_mako(NULL, 0, n, 1, 8) == 1, "mako: broken output changes nothing");
    char cut[8] = "ab\xc3\xa9";
    utf8_cut(cut);
    CHECK(!strcmp(cut, "ab\xc3\xa9"), "utf8_cut keeps a whole character");
    cut[3] = 0;
    utf8_cut(cut);
    CHECK(!strcmp(cut, "ab"), "utf8_cut drops half a character");
    strcpy(cut, "x\xf0\x9f\x8e");
    utf8_cut(cut);
    CHECK(!strcmp(cut, "x"), "utf8_cut drops three quarters of an emoji");
    c = parse_mako(hist, 0, n, 0, 1);
    CHECK(c == 1, "mako: stops at max");

    char root[] = "/tmp/deck-test-XXXXXX", dir[600], text[64];
    CHECK(mkdtemp(root) != NULL, "temp folder");
    struct battery b;
    CHECK(battery_read(root, &b) == -1, "no battery (a desktop)");
    snprintf(dir, sizeof dir, "%s/BAT0", root);
    mkdir(dir, 0700);
    put(dir, "capacity", "64\n");
    put(dir, "status", "Discharging\n");
    put(dir, "energy_now", "30000000\n");
    put(dir, "energy_full", "50000000\n");
    put(dir, "power_now", "12000000\n");
    CHECK(battery_read(root, &b) == 0 && b.percent == 64 && !b.charging && b.minutes == 150, "battery: 30 Wh at 12 W is 2 h 30 min");
    battery_text(&b, text, sizeof text);
    CHECK(!strcmp(text, "2 h 30 min left"), text);
    put(dir, "status", "Charging\n");
    CHECK(battery_read(root, &b) == 0 && b.charging && b.minutes == 100, "charging: 20 Wh to go at 12 W");
    battery_text(&b, text, sizeof text);
    CHECK(!strcmp(text, "Full in 1 h 40 min"), text);
    put(dir, "power_now", "0\n");
    battery_read(root, &b);
    battery_text(&b, text, sizeof text);
    CHECK(b.minutes == -1 && !strcmp(text, "Charging"), "no rate: no forecast");
    put(dir, "status", "Full\n");
    battery_read(root, &b);
    battery_text(&b, text, sizeof text);
    CHECK(b.full && !strcmp(text, "Full"), "full");
    put(dir, "status", "Discharging\n");
    put(dir, "power_now", "10\n");
    battery_read(root, &b);
    CHECK(b.minutes == -1, "a tiny rate gives no silly forecast");

    CHECK(backlight_read(root) == -1, "no backlight (a desktop)");
    snprintf(dir, sizeof dir, "%s/intel_backlight", root);
    mkdir(dir, 0700);
    put(dir, "brightness", "960\n");
    put(dir, "max_brightness", "1920\n");
    CHECK(backlight_read(root) == 50, "backlight 50 %");

    CHECK(signal_percent(-4500) == 100 && signal_percent(-7000) == 60 && signal_percent(-10000) == 0 && signal_percent(-12000) == 0,
          "iwd signal to percent");
    CHECK(signal_bars(100) == 4 && signal_bars(60) == 3 && signal_bars(30) == 2 && signal_bars(5) == 1 && signal_bars(0) == 0, "bars");

    CHECK(tunnel_ok("Sweden") && tunnel_ok("wg-0.home_1") && !tunnel_ok("") && !tunnel_ok("a\"b") && !tunnel_ok("sixteen-chars-xx") &&
          !tunnel_ok("a b") && !tunnel_ok(NULL), "tunnel names");
    char req[256];
    CHECK(broker_request(req, sizeof req, "vpn_up", "Sweden", -1) == 0 && !strcmp(req, "{\"action\": \"vpn_up\", \"args\": {\"name\": \"Sweden\"}}\n"),
          "request with a name");
    CHECK(broker_request(req, sizeof req, "set_brightness", NULL, 140) == 0 && strstr(req, "\"percent\": 100}"), "brightness capped");
    CHECK(broker_request(req, sizeof req, "vpn_list", NULL, -1) == 0 && strstr(req, "\"args\": {}}"), "request without arguments");
    CHECK(broker_request(req, sizeof req, "vpn_up", "x\"}, \"action\": \"reboot", -1) == -1, "an unsafe name is never sent");
    CHECK(broker_request(req, sizeof req, "Reboot!", NULL, -1) == -1 && broker_request(req, 10, "vpn_list", NULL, -1) == -1,
          "odd action, small buffer");
    char err[128];
    CHECK(broker_ok("{\"ok\": true, \"result\": {}}", err, sizeof err) == 1, "reply ok");
    CHECK(broker_ok("{\"ok\": false, \"error\": \"No such VPN.\"}", err, sizeof err) == 0 && !strcmp(err, "No such VPN."), "reply error");
    CHECK(broker_ok("", err, sizeof err) == 0 && err[0], "no reply");
    struct tunnel t[4];
    CHECK(parse_tunnels("{\"ok\": true, \"result\": {\"tunnels\": [{\"name\": \"Sweden\", \"active\": false}, {\"name\": \"bad name\"},"
                        " {\"name\": \"Home\", \"active\": true}]}}", t, 4) == 2 && !strcmp(t[1].name, "Home") && t[1].active && !t[0].active,
          "tunnels: unsafe names skipped");
    CHECK(parse_tunnels("{\"ok\": false}", t, 4) == -1, "tunnels: refused");

    printf("%s: %d failed\n", fails ? "FAIL" : "PASS", fails);
    char cmd[700];
    snprintf(cmd, sizeof cmd, "rm -rf %s", root);
    if (system(cmd)) {}
    return fails != 0;
}
