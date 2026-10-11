/* NexaOS boot splash: draws logo + progress bar straight to /dev/fb0.
 * Runs during early boot, before Wayland/DRM exists, so it's plain C with
 * no GTK/Cairo — just linux/fb.h and the bitmap font already in this repo.
 * Usage: bootsplash <percent 0-100> [status text]
 *        bootsplash --clear
 */
#include <fcntl.h>
#include <linux/fb.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#include "font8x8.h"

typedef struct { uint8_t r, g, b; } Color;

typedef struct {
    int fd;
    uint8_t *mem, *back;
    size_t size;
    int w, h, stride, bpp;
    struct fb_var_screeninfo var;
} FB;

static const Color BG = {30, 30, 30};
static const Color BLUE = {122, 162, 247};
static const Color WHITE = {224, 224, 224};
static const Color DIM = {90, 90, 90};
static const Color TRACK = {58, 58, 58};

static int fb_open(FB *fb) {
    memset(fb, 0, sizeof(*fb));
    fb->fd = open("/dev/fb0", O_RDWR);
    if (fb->fd < 0) return -1;
    struct fb_fix_screeninfo fix;
    ioctl(fb->fd, FBIOGET_FSCREENINFO, &fix);
    ioctl(fb->fd, FBIOGET_VSCREENINFO, &fb->var);
    fb->w = fb->var.xres;
    fb->h = fb->var.yres;
    fb->stride = fix.line_length;
    fb->bpp = fb->var.bits_per_pixel;
    fb->size = fix.smem_len;
    fb->mem = mmap(NULL, fb->size, PROT_READ | PROT_WRITE, MAP_SHARED, fb->fd, 0);
    if (fb->mem == MAP_FAILED) { close(fb->fd); return -1; }
    fb->back = malloc(fb->size);
    if (!fb->back) { munmap(fb->mem, fb->size); close(fb->fd); return -1; }
    return 0;
}

static void fb_close(FB *fb) {
    free(fb->back);
    munmap(fb->mem, fb->size);
    close(fb->fd);
}

static void fb_flip(FB *fb) {
    memcpy(fb->mem, fb->back, fb->size);
}

static inline void px(FB *fb, int x, int y, Color c) {
    if (x < 0 || y < 0 || x >= fb->w || y >= fb->h) return;
    uint8_t *p = fb->back + (size_t)y * fb->stride + (size_t)x * (fb->bpp / 8);
    uint32_t v = ((uint32_t)c.r >> (8 - fb->var.red.length)) << fb->var.red.offset
               | ((uint32_t)c.g >> (8 - fb->var.green.length)) << fb->var.green.offset
               | ((uint32_t)c.b >> (8 - fb->var.blue.length)) << fb->var.blue.offset;
    if (fb->bpp == 32) *(uint32_t *)p = v;
    else if (fb->bpp == 16) *(uint16_t *)p = (uint16_t)v;
    else { p[0] = v & 0xff; p[1] = (v >> 8) & 0xff; p[2] = (v >> 16) & 0xff; }
}

static void fill_rect(FB *fb, int x, int y, int w, int h, Color c) {
    for (int j = 0; j < h; j++)
        for (int i = 0; i < w; i++)
            px(fb, x + i, y + j, c);
}

static void draw_char(FB *fb, int x, int y, char ch, int scale, Color c) {
    if ((unsigned char)ch >= 128) return;
    const char *glyph = font8x8_basic[(int)ch];
    for (int row = 0; row < 8; row++)
        for (int col = 0; col < 8; col++)
            if (glyph[row] & (1 << col))
                fill_rect(fb, x + col * scale, y + row * scale, scale, scale, c);
}

static void draw_text(FB *fb, int x, int y, const char *s, int scale, Color c) {
    for (int i = 0; s[i]; i++)
        draw_char(fb, x + i * 8 * scale, y, s[i], scale, c);
}

static int text_width(const char *s, int scale) {
    return (int)strlen(s) * 8 * scale;
}

/* A small hexagon outline with a filled dot, echoing the wallpaper's mark. */
static void draw_mark(FB *fb, int cx, int cy, int r, Color c) {
    for (double a = 0; a < 360; a += 0.5) {
        double rad = a * 3.14159265 / 180.0;
        int x = cx + (int)(r * 0.98 * cos(rad));
        int y = cy + (int)(r * 0.98 * sin(rad));
        px(fb, x, y, c);
        x = cx + (int)(r * 1.0 * cos(rad));
        y = cy + (int)(r * 1.0 * sin(rad));
        px(fb, x, y, c);
    }
    for (int dy = -3; dy <= 3; dy++)
        for (int dx = -3; dx <= 3; dx++)
            if (dx * dx + dy * dy <= 9) px(fb, cx + dx, cy + dy, c);
}

static void draw_frame(FB *fb, int percent, const char *status) {
    fill_rect(fb, 0, 0, fb->w, fb->h, BG);

    int cy = fb->h / 2 - 40;
    draw_mark(fb, fb->w / 2 - 90, cy, 22, BLUE);

    const char *title = "NexaOS";
    int scale = 4;
    draw_text(fb, fb->w / 2 - text_width(title, scale) / 2 + 20, cy - 16, title, scale, WHITE);

    int bar_w = 360, bar_h = 6;
    int bx = fb->w / 2 - bar_w / 2, by = cy + 70;
    fill_rect(fb, bx, by, bar_w, bar_h, TRACK);
    if (percent > 0) {
        int fill_w = bar_w * (percent > 100 ? 100 : percent) / 100;
        fill_rect(fb, bx, by, fill_w, bar_h, BLUE);
    }

    if (status && status[0]) {
        int sscale = 1;
        draw_text(fb, fb->w / 2 - text_width(status, sscale) / 2, by + 18, status, sscale, DIM);
    }
}

int main(int argc, char **argv) {
    FB fb;
    if (fb_open(&fb) != 0) return 1;

    if (argc > 1 && strcmp(argv[1], "--clear") == 0) {
        fill_rect(&fb, 0, 0, fb.w, fb.h, BG);
        fb_flip(&fb);
        fb_close(&fb);
        return 0;
    }

    int percent = argc > 1 ? atoi(argv[1]) : 0;
    const char *status = argc > 2 ? argv[2] : "";
    draw_frame(&fb, percent, status);
    fb_flip(&fb);
    fb_close(&fb);
    return 0;
}
