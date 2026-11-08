/* The voice helix: Nexa Mind's face while it listens, thinks and speaks.
 * Two strands turning around each other, swelling with the voice; each
 * syllable makes the helix pop out where it lands and throws sparks; the
 * colours flow with the sound. No screen in here (Cairo only), so the
 * motion is unit tested.
 *
 * voiceprint: loudness, syllable beats and brightness of a voice, 100
 * times a second, from 16-bit samples. The same code reads Nexa's voice as
 * it plays and yours from the microphone. */
#ifndef NEXA_VOICE_HELIX_H
#define NEXA_VOICE_HELIX_H

#include <cairo.h>
#include <stddef.h>
#include <stdint.h>

/* ---------- voiceprint ---------- */

typedef struct {
    float level;    /* 0 quiet .. 1 loud */
    float onset;    /* > 0 on the frame a syllable starts (its strength) */
    float bright;   /* 0 dark .. 1 bright (hiss, high voices) */
} vp_frame;

typedef struct {
    int hop, fill, since, crossings;
    double sum2, slow, prev;
    int16_t last;
} vp_state;

void vp_init(vp_state *v, int rate);
/* Feed samples; writes the finished 10 ms frames (at most cap), returns
 * how many. */
int vp_feed(vp_state *v, const int16_t *s, size_t n, vp_frame *out, int cap);

/* ---------- the helix ---------- */

enum helix_mode { HELIX_IDLE, HELIX_LISTENING, HELIX_THINKING, HELIX_SPEAKING };
enum helix_palette { HELIX_VOICE, HELIX_CYCLE, HELIX_CALM, HELIX_EMBER };

#define HELIX_POPS 24
#define HELIX_SPARKS 160

typedef struct {
    enum helix_mode mode;
    enum helix_palette palette;
    int animations;          /* 0: still strands that brighten and swell, no sparks */
    double t, env, rot, hue, pitch, accent_hue;
    struct { double u, s, age; int used; } pop[HELIX_POPS];
    struct { double u, y, vx, vy, life, max, hue; int used; } spark[HELIX_SPARKS];
    unsigned rng;
    long pops_total;         /* for tests */
    double env_peak;
} helix;

/* accent_hue: the theme's accent, in degrees; the colours start there. */
void helix_init(helix *h, double accent_hue, unsigned seed);
/* Advance by dt seconds. level/onset/bright: the voice now (from
 * voiceprint), 0 when there is none. */
void helix_step(helix *h, double dt, double level, double onset, double bright);
/* A syllable now (strength 0..1): the helix pops out and throws sparks.
 * helix_step calls it for its onset; call it directly for more than one
 * syllable in a step (a late frame). */
void helix_pop(helix *h, double strength, double bright);
/* Draw into a w x hgt area (transparent around it). */
void helix_draw(const helix *h, cairo_t *cr, double w, double hgt);

/* Strand colour at u (0..1 along the helix), strand 0 or 1, as RGB. */
void helix_colour(const helix *h, double u, int strand, double *r, double *g, double *b);
#endif
