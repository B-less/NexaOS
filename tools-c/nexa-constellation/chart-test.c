/* Unit tests for chart.c: make test. */
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

#include "chart.h"

static int checks, fails;

static void check(int ok, const char *what)
{
    checks++;
    if (!ok) {
        fails++;
        printf("FAIL %s\n", what);
    } else {
        printf("PASS %s\n", what);
    }
}

static int hexdist(int q, int r) { return (abs(q) + abs(r) + abs(q + r)) / 2; }

int main(void)
{
    /* ---- the spiral */
    int seen_dup = 0, ring_ok = 1, q, r, qs[37], rs[37];
    for (int k = 0; k < 37; k++) {
        chart_hex(k, &q, &r);
        qs[k] = q;
        rs[k] = r;
        int want = k == 0 ? 0 : k <= 6 ? 1 : k <= 18 ? 2 : 3;
        if (hexdist(q, r) != want) ring_ok = 0;
        for (int j = 0; j < k; j++)
            if (qs[j] == q && rs[j] == r) seen_dup = 1;
    }
    check(!seen_dup, "spiral: 37 cells, none twice");
    check(ring_ok, "spiral: the centre, then 6 around it, then 12, then 18");
    int steps_ok = 1;
    for (int k = 2; k < 37; k++)
        if (k != 7 && k != 19 && hexdist(qs[k] - qs[k - 1], rs[k] - rs[k - 1]) != 1) steps_ok = 0;
    check(steps_ok, "spiral: each cell beside the one before (within a ring)");

    /* ---- layout */
    struct chart_pt star[CHART_MAX], centre[CHART_MAX];
    int group[CHART_MAX], order[CHART_MAX];
    group[0] = 0;
    order[0] = 0;
    double s = chart_layout(group, order, 1, 1, 1280, 800, star, centre);
    check(s == 1 && fabs(star[0].x - 640) < 1 && fabs(star[0].y - 400) < 10, "one window: in the middle (room for its title below), full size");

    /* 12 apps with 3 windows each */
    int n = 0;
    for (int g = 0; g < 12; g++)
        for (int k = 0; k < 3; k++, n++) {
            group[n] = g;
            order[n] = k;
        }
    s = chart_layout(group, order, n, 12, 1280, 800, star, centre);
    int inside = 1, apart = 1, top = 1;
    for (int i = 0; i < n; i++) {
        if (star[i].x < 40 || star[i].x > 1240 || star[i].y < 70 || star[i].y > 730) inside = 0;
        for (int j = 0; j < n; j++)
            if (group[i] != group[j] && hypot(star[i].x - star[j].x, star[i].y - star[j].y) < 60 * s) apart = 0;
        if (order[i] == 0)
            for (int j = 0; j < n; j++)
                if (group[j] == group[i] && star[j].y < star[i].y - 0.01) top = 0;
    }
    check(s < 1 && s >= CHART_SCALE_MIN, "36 windows: made smaller to fit");
    check(inside, "36 windows: all on the screen, clear of the search and the hints");
    check(apart, "apps don't overlap each other");
    check(top, "an app's window used last sits on top of its ring");
    check(fabs(centre[0].x - centre[1].x) > 1 || fabs(centre[0].y - centre[1].y) > 1, "each app its own place");
    double spread = 0;
    for (int g = 1; g < 12; g++) spread = fmax(spread, hypot(centre[g].x - centre[0].x, centre[g].y - centre[0].y));
    check(spread > 100, "the apps spread out around the first");

    for (int i = 0; i < CHART_MAX; i++) {
        group[i] = i;
        order[i] = 0;
    }
    s = chart_layout(group, order, CHART_MAX, CHART_MAX, 1280, 800, star, centre);
    check(s == CHART_SCALE_MIN, "128 apps: never smaller than the smallest scale");

    /* ---- the tree of a project */
    struct chart_pt line[4] = {{0, 0}, {300, 0}, {100, 0}, {200, 0}};
    int mem[4] = {0, 1, 2, 3}, e[CHART_MAX][2];
    int ne = chart_tree(line, mem, 4, e);
    double total = 0;
    for (int i = 0; i < ne; i++) total += fabs(line[e[i][0]].x - line[e[i][1]].x);
    check(ne == 3 && fabs(total - 300) < 0.01, "a project's line: the shortest tree (3 edges, 300 px)");
    int touched[4] = {0};
    for (int i = 0; i < ne; i++) touched[e[i][0]] = touched[e[i][1]] = 1;
    check(touched[0] && touched[1] && touched[2] && touched[3], "every window of the project is on it");
    int one[1] = {2};
    check(chart_tree(line, one, 1, e) == 0, "one window: no line");

    /* ---- arrow keys on a 3 x 3 grid (index = row * 3 + column) */
    struct chart_pt g9[9];
    for (int i = 0; i < 9; i++) g9[i] = (struct chart_pt){100.0 * (i % 3), 100.0 * (i / 3)};
    int on[9] = {1, 1, 1, 1, 1, 1, 1, 1, 1};
    check(chart_step(g9, on, 9, 4, 1, 0) == 5, "Right from the middle");
    check(chart_step(g9, on, 9, 4, 0, -1) == 1, "Up from the middle");
    check(chart_step(g9, on, 9, 4, -1, 0) == 3, "Left from the middle");
    check(chart_step(g9, on, 9, 4, 0, 1) == 7, "Down from the middle");
    check(chart_step(g9, on, 9, 3, -1, 0) == -1, "nothing further left: stays");
    on[5] = 0;
    check(chart_step(g9, on, 9, 4, 1, 0) == 2 || chart_step(g9, on, 9, 4, 1, 0) == 8, "a faded star is passed over");
    on[2] = on[8] = 0;
    check(chart_step(g9, on, 9, 4, 1, 0) == -1, "nothing that way: stays");
    check(chart_step(g9, on, 9, 99, 1, 0) == -1, "from no star: nothing");

    /* ---- finding */
    check(chart_match("notes.txt — Editor", "Editor", "notes"), "a word of the title");
    check(chart_match("notes.txt — Editor", "Editor", "NOTES edi"), "two words, any case");
    check(!chart_match("notes.txt — Editor", "Editor", "notes mail"), "every word must be there");
    check(chart_match("Inbox", "Mail", "mail"), "the app's name");
    check(chart_match("ÉCOLE — Écrits", "Editor", "école"), "case in any script");
    check(chart_match("x", "y", ""), "nothing typed: everything");
    check(chart_match(NULL, NULL, NULL), "no title");

    printf("%d of %d checks passed\n", checks - fails, checks);
    return fails != 0;
}
