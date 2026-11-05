/* Unit tests for Strata's layer maths (make test). */
#include <math.h>
#include <stdio.h>

#include "layers.h"

static int fails, checks;

static void check(int ok, const char *what)
{
    checks++;
    if (!ok) fails++;
    printf("%s %s\n", ok ? "PASS" : "FAIL", what);
}

static int near(double a, double b) { return fabs(a - b) < 0.01; }

int main(void)
{
    check(strata_target(1, 1) == 2, "dive from layer 1 reaches layer 2");
    check(strata_target(5, 1) == 5, "no layer below 5 (no wrapping)");
    check(strata_target(1, -1) == 1, "nothing above the surface");
    check(strata_target(3, -2) == 1 && strata_target(2, 9) == 5, "longer steps stop at the ends");
    check(strata_target(0, 1) == 2 && strata_target(-99, 1) == 2 && strata_target(12, -1) == 1,
          "a named or special workspace counts as the surface");

    check(strata_step("+1") == 1 && strata_step("-1") == -1 && strata_step("2") == 2, "steps read");
    check(strata_step("") == 0 && strata_step("+") == 0 && strata_step("1x") == 0 && strata_step("999") == 0 &&
              strata_step(NULL) == 0,
          "bad steps refused");

    strata_rgb acc = {0x4d / 255.0, 0xa3 / 255.0, 1.0};
    strata_rgb t1 = strata_tint(acc, 1);
    check(near(t1.r, acc.r) && near(t1.g, acc.g) && near(t1.b, acc.b), "layer 1 is the accent itself");
    strata_rgb t2 = strata_tint(acc, 2), t5 = strata_tint(acc, 5);
    check(!near(t2.r, t1.r) || !near(t2.g, t1.g), "layer 2 has another colour");
    check(t2.r > t1.r && t2.b > 0.9, "layer 2 turns from blue towards violet");
    double l1 = fmax(t1.r, fmax(t1.g, t1.b)) + fmin(t1.r, fmin(t1.g, t1.b));
    double l5 = fmax(t5.r, fmax(t5.g, t5.b)) + fmin(t5.r, fmin(t5.g, t5.b));
    check(near(l1, l5), "the turn keeps the accent's lightness");
    strata_rgb grey = strata_tint((strata_rgb){0.5, 0.5, 0.5}, 3);
    check(near(grey.r, 0.5) && near(grey.b, 0.5), "a grey accent stays grey");
    int in_range = 1;
    for (int n = 1; n <= STRATA_LAYERS; n++) {
        strata_rgb c = strata_tint((strata_rgb){0.9, 0.2, 0.1}, n);
        in_range &= c.r >= 0 && c.r <= 1 && c.g >= 0 && c.g <= 1 && c.b >= 0 && c.b <= 1;
    }
    check(in_range, "tints stay within 0..1");

    struct strata_conf c;
    strata_conf_parse(&c, NULL);
    check(c.show == 1 && near(c.tint, STRATA_TINT), "defaults without a file");
    strata_conf_parse(&c, "# Strata\nshow=no\ntint=0.2\n");
    check(c.show == 0 && near(c.tint, 0.2), "show=no and a tint read");
    strata_conf_parse(&c, "tint=5\nshow=yes");
    check(c.show == 1 && near(c.tint, STRATA_TINT_MAX), "a tint too strong is capped");
    strata_conf_parse(&c, "tint=-1\n");
    check(near(c.tint, 0), "a negative tint is off");
    strata_conf_parse(&c, "tint=nan\ntint=abc\n");
    check(near(c.tint, STRATA_TINT), "a tint that isn't a number is ignored");

    struct strata_sheet cur = strata_sheet_at(2, 2), deep = strata_sheet_at(4, 2), past = strata_sheet_at(1, 2),
                        gone = strata_sheet_at(0, 2);
    check(near(cur.scale, 1) && near(cur.rise, 0) && near(cur.alpha, 1), "the current layer is full size, in place, solid");
    check(deep.scale < cur.scale && deep.rise > 0 && deep.alpha < 1 && deep.alpha >= 0.25, "deeper layers: higher, smaller, fainter");
    check(past.scale > 1 && past.rise < 0 && past.alpha == 0, "a layer dived past has fallen away");
    check(gone.alpha == 0, "two layers up: gone");
    struct strata_sheet mid = strata_sheet_at(1, 1.5);
    check(mid.alpha > 0 && mid.alpha < 1 && mid.rise < 0, "while gliding a passed layer fades out");
    int monotone = 1;
    for (int i = 0; i + 1 < STRATA_LAYERS; i++) {
        struct strata_sheet a = strata_sheet_at(i, 0), b = strata_sheet_at(i + 1, 0);
        monotone &= b.rise > a.rise && b.scale < a.scale;
    }
    check(monotone, "every deeper sheet higher and smaller");

    check(strata_ease(0) == 0 && strata_ease(1) == 1 && strata_ease(0.5) > 0.5 && strata_ease(-1) == 0 && strata_ease(2) == 1,
          "ease-out");
    printf("%d of %d checks passed\n", checks - fails, checks);
    return fails != 0;
}
