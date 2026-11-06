#include "chart.h"

#include <glib.h>
#include <math.h>
#include <string.h>

void chart_hex(int k, int *q, int *r)
{
    /* axial directions, going round: right, up-right, up-left, left, down-left, down-right */
    static const int D[6][2] = {{1, 0}, {1, -1}, {0, -1}, {-1, 0}, {-1, 1}, {0, 1}};
    *q = *r = 0;
    if (k <= 0) return;
    int ring = 1;
    while (k > 3 * ring * (ring + 1)) ring++;
    int i = k - 1 - 3 * ring * (ring - 1);  /* 0.. 6*ring-1 within this ring */
    /* start on the ring's right-hand cell, then walk its six sides */
    int x = ring, y = 0;
    for (int side = 0; side < 6; side++) {
        int dx = D[(side + 2) % 6][0], dy = D[(side + 2) % 6][1];
        for (int s = 0; s < ring; s++) {
            if (i-- == 0) {
                *q = x;
                *r = y;
                return;
            }
            x += dx;
            y += dy;
        }
    }
}

static double ring_of(int m) { return m <= 1 ? 0 : fmax(CHART_RING_MIN, m * CHART_RING_STEP / (2 * M_PI)); }

double chart_layout(const int *group, const int *order, int n, int ngroups, double w, double h,
                    struct chart_pt *star, struct chart_pt *centre)
{
    if (ngroups <= 0 || n <= 0) return 1;
    int size[CHART_MAX] = {0};
    if (ngroups > CHART_MAX) ngroups = CHART_MAX;
    for (int i = 0; i < n; i++)
        if (group[i] >= 0 && group[i] < ngroups) size[group[i]]++;
    double rmax = 0;
    for (int g = 0; g < ngroups; g++) rmax = fmax(rmax, ring_of(size[g]));
    double step = 2 * rmax + CHART_GAP;  /* centre to centre */
    /* cells, and the box around them (with room for a ring and its titles) */
    double minx = 1e9, maxx = -1e9, miny = 1e9, maxy = -1e9;
    for (int g = 0; g < ngroups; g++) {
        int q, r;
        chart_hex(g, &q, &r);
        centre[g].x = step * (q + r / 2.0);
        centre[g].y = step * (sqrt(3) / 2) * r;
        double rx = ring_of(size[g]) + 90, top = ring_of(size[g]) + 60, bottom = ring_of(size[g]) + 70;
        minx = fmin(minx, centre[g].x - rx);
        maxx = fmax(maxx, centre[g].x + rx);
        miny = fmin(miny, centre[g].y - top);
        maxy = fmax(maxy, centre[g].y + bottom);
    }
    double room_w = fmax(1, w - 2 * 40), room_h = fmax(1, h - 2 * 70);  /* the search above, the hints below */
    double scale = fmin(1, fmin(room_w / (maxx - minx), room_h / (maxy - miny)));
    if (scale < CHART_SCALE_MIN) scale = CHART_SCALE_MIN;
    double ox = w / 2 - scale * (minx + maxx) / 2, oy = h / 2 - scale * (miny + maxy) / 2;
    for (int g = 0; g < ngroups; g++) {
        centre[g].x = ox + scale * centre[g].x;
        centre[g].y = oy + scale * centre[g].y;
    }
    for (int i = 0; i < n; i++) {
        int g = group[i] >= 0 && group[i] < ngroups ? group[i] : 0;
        int m = size[g] ? size[g] : 1;
        double a = -M_PI / 2 + 2 * M_PI * (order[i] % m) / m;  /* the one used last on top */
        double r = scale * ring_of(m);
        star[i].x = centre[g].x + r * cos(a);
        star[i].y = centre[g].y + r * sin(a);
    }
    return scale;
}

static double dist2(struct chart_pt a, struct chart_pt b) { return (a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y); }

int chart_tree(const struct chart_pt *p, const int *members, int m, int (*edges)[2])
{
    if (m < 2) return 0;
    if (m > CHART_MAX) m = CHART_MAX;
    int in[CHART_MAX] = {0}, from[CHART_MAX];
    double best[CHART_MAX];
    in[0] = 1;
    for (int i = 1; i < m; i++) {
        best[i] = dist2(p[members[0]], p[members[i]]);
        from[i] = 0;
    }
    int ne = 0;
    for (int step = 1; step < m; step++) {
        int pick = -1;
        for (int i = 1; i < m; i++)
            if (!in[i] && (pick < 0 || best[i] < best[pick])) pick = i;
        in[pick] = 1;
        edges[ne][0] = members[from[pick]];
        edges[ne][1] = members[pick];
        ne++;
        for (int i = 1; i < m; i++) {
            double d = dist2(p[members[pick]], p[members[i]]);
            if (!in[i] && d < best[i]) {
                best[i] = d;
                from[i] = pick;
            }
        }
    }
    return ne;
}

int chart_step(const struct chart_pt *p, const int *on, int n, int from, int dx, int dy)
{
    if (from < 0 || from >= n) return -1;
    double len = hypot(dx, dy);
    if (len == 0) return -1;
    double ux = dx / len, uy = dy / len;
    int best = -1;
    double best_cost = 0;
    for (int i = 0; i < n; i++) {
        if (i == from || (on && !on[i])) continue;
        double vx = p[i].x - p[from].x, vy = p[i].y - p[from].y;
        double along = vx * ux + vy * uy, across = fabs(vx * uy - vy * ux);
        if (along <= 0.5 || across > along * 1.732) continue;  /* within 60 degrees */
        double cost = along + 2 * across;
        if (best < 0 || cost < best_cost) {
            best = i;
            best_cost = cost;
        }
    }
    return best;
}

int chart_match(const char *title, const char *app, const char *query)
{
    if (!query || !*query) return 1;
    char *hay = g_strconcat(title ? title : "", "\n", app ? app : "", NULL);
    char *h = g_utf8_casefold(hay, -1), *q = g_utf8_casefold(query, -1);
    char **words = g_strsplit_set(q, " \t", -1);
    int ok = 1;
    for (int i = 0; words[i] && ok; i++)
        if (*words[i] && !strstr(h, words[i])) ok = 0;
    g_strfreev(words);
    g_free(q);
    g_free(h);
    g_free(hay);
    return ok;
}
