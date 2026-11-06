/* Constellation: the parts with no screen in them (TODOnexa_identity.md,
 * section "Constellation"): where the stars go, the lines of a project,
 * the arrow keys and finding. Used by the sky in Orbit (sky.c); unit tests
 * in chart-test.c. */
#ifndef NEXA_CONSTELLATION_CHART_H
#define NEXA_CONSTELLATION_CHART_H

#define CHART_MAX 128          /* windows (WM_MAX_WIN) */
#define CHART_RING_MIN 70.0    /* px: an app's ring of windows, at least */
#define CHART_RING_STEP 125.0  /* px of ring per window */
#define CHART_GAP 170.0        /* px between neighbouring rings */
#define CHART_SCALE_MIN 0.45

struct chart_pt { double x, y; };

/* The k-th cell of a hexagon spiral (0 = the centre, then the six around
 * it, then the twelve around those...), in axial coordinates. */
void chart_hex(int k, int *q, int *r);

/* Where everything goes on a w x h screen. Window i belongs to group
 * group[i] (0 = the app used last, then by last use) and has place
 * order[i] in its group (0 = used last). Fills star[n] and centre[ngroups]
 * and returns the scale (1 = full size; smaller when crowded, never below
 * CHART_SCALE_MIN). */
double chart_layout(const int *group, const int *order, int n, int ngroups, double w, double h,
                    struct chart_pt *star, struct chart_pt *centre);

/* The shortest tree through the m stars members[] (Prim): m-1 edges into
 * edges[][2] (indices into p). Returns the number of edges. */
int chart_tree(const struct chart_pt *p, const int *members, int m, int (*edges)[2]);

/* From star `from`, the nearest star in direction (dx, dy) (one of the
 * four arrow keys) among those with on[i] set, within 60 degrees of it;
 * -1 if there is none. */
int chart_step(const struct chart_pt *p, const int *on, int n, int from, int dx, int dy);

/* Does the search match? Every word of `query` in the title or the app's
 * name, ignoring case (any script). An empty query matches everything. */
int chart_match(const char *title, const char *app, const char *query);
#endif
