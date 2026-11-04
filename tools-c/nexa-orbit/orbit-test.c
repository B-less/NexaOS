/* Unit tests for nexa-orbit's model (make test). */
#include <stdio.h>
#include <string.h>

#include "model.h"

static int fails;
#define CHECK(c, what) do { if (c) printf("PASS %s\n", what); else { printf("FAIL %s\n", what); fails++; } } while (0)

int main(void)
{
    struct orbit_conf c;
    char buf[8192];

    conf_parse(&c, NULL);
    CHECK(c.npinned == 8 && !strcmp(c.pinned[0], "nexa-files") && !c.fold, "no file: the default apps, not folded");

    conf_parse(&c, "# comment\npinned = nexa-files, foot ,nexa-files,bad id,../x,-x\nfold=yes\nother=1\n");
    CHECK(c.npinned == 2 && !strcmp(c.pinned[1], "foot"), "pinned: trimmed, repeats and unsafe ids dropped");
    CHECK(c.fold == 1, "fold=yes");

    conf_parse(&c, "pinned=\n");
    CHECK(c.npinned == 0, "an empty pinned line means nothing pinned");

    conf_parse(&c, "fold=no\n");
    CHECK(c.npinned == 8 && c.fold == 0, "no pinned line keeps the defaults");

    conf_parse(&c, "pinned=a,b\nfold=yes\n");
    CHECK(conf_format(&c, buf, sizeof buf) == 0 && strstr(buf, "\npinned=a,b\nfold=yes\n"), "format");
    struct orbit_conf back;
    conf_parse(&back, buf);
    CHECK(back.npinned == 2 && back.fold == 1 && !strcmp(back.pinned[0], "a"), "format then parse gives the same");
    CHECK(conf_format(&c, buf, 20) == -1, "format refuses a buffer too small");

    CHECK(conf_pin(&c, "c") == 0 && c.npinned == 3 && !strcmp(c.pinned[2], "c"), "pin adds at the end");
    CHECK(conf_pin(&c, "a") == -1, "pin refuses a repeat");
    CHECK(conf_pin(&c, "x\"y") == -1, "pin refuses an unsafe id");
    CHECK(conf_unpin(&c, "a") == 0 && c.npinned == 2 && !strcmp(c.pinned[0], "b") && !strcmp(c.pinned[1], "c"), "unpin keeps the order");
    CHECK(conf_unpin(&c, "zz") == -1, "unpin of a missing id");
    conf_parse(&c, "pinned=\n");
    for (int i = 0; i < ORBIT_MAX_PINNED; i++) { char id[16]; snprintf(id, sizeof id, "app%d", i); conf_pin(&c, id); }
    CHECK(c.npinned == ORBIT_MAX_PINNED && conf_pin(&c, "one-more") == -1, "pin stops when full");

    CHECK(id_ok("org.codeberg.dnkl.foot") && id_ok("nexa-files") && id_ok("A_b.9"), "id_ok: normal ids");
    CHECK(!id_ok("") && !id_ok(NULL) && !id_ok(".hidden") && !id_ok("-x") && !id_ok("a b") && !id_ok("a\"]; exec x") &&
          !id_ok("a/b") && !id_ok("a$b"), "id_ok: refuses what could break a command or a path");
    char longid[200];
    memset(longid, 'a', sizeof longid - 1);
    longid[sizeof longid - 1] = 0;
    CHECK(!id_ok(longid), "id_ok: too long");

    int ppid;
    unsigned long long ticks;
    CHECK(parse_stat("1234 (foot) S 1000 1234 1234 0 -1 4194304 100 0 0 0 250 50 0 0 20 0 1 0 100 0 0", &ppid, &ticks) == 0 &&
          ppid == 1000 && ticks == 300, "stat: parent and ticks");
    CHECK(parse_stat("77 (Web Content) (x)) R 12 77 77 0 -1 0 0 0 0 0 7 3 0 0", &ppid, &ticks) == 0 && ppid == 12 && ticks == 10,
          "stat: a name with spaces and parentheses");
    CHECK(parse_stat("garbage", &ppid, &ticks) == -1 && parse_stat(NULL, &ppid, &ticks) == -1 && parse_stat("1 (x) S", &ppid, &ticks) == -1,
          "stat: refuses broken lines");

    CHECK(cpu_percent(100, 300, 2.0, 100) == 100.0, "cpu: one core busy for 2 s");
    CHECK(cpu_percent(100, 150, 2.0, 100) == 25.0, "cpu: a quarter");
    CHECK(cpu_percent(300, 100, 2.0, 100) == 0 && cpu_percent(1, 2, 0, 100) == 0, "cpu: going backwards or no time is 0");

    CHECK(exec_strip("nexa-files %U", buf, sizeof buf) == 0 && !strcmp(buf, "nexa-files "), "exec: field code dropped");
    CHECK(exec_strip("sh -c 'echo 100%%' %f", buf, sizeof buf) == 0 && !strcmp(buf, "sh -c 'echo 100%' "), "exec: %% is a percent sign");
    CHECK(exec_strip("x %", buf, sizeof buf) == 0 && !strcmp(buf, "x "), "exec: a lone % at the end");
    CHECK(exec_strip("abcdef", buf, 4) == -1, "exec: too long");

    CHECK(!strcmp(alias_of("foot"), "org.codeberg.dnkl.foot") && !strcmp(alias_of("org.codeberg.dnkl.foot"), "foot") && !alias_of("x"),
          "aliases");

    printf("%s: %d failed\n", fails ? "FAIL" : "PASS", fails);
    return fails != 0;
}
