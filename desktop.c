#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/ioctl.h>
#include <linux/fb.h>

#include "font8x8_basic.h"

int fbfd = 0;
struct fb_var_screeninfo vinfo;
struct fb_fix_screeninfo finfo;
uint32_t *fbp = 0;
uint32_t *backbuf = 0;

int mouse_fd = 0;
int mouse_x = 512, mouse_y = 384;

void draw_rect(uint32_t *buf, int x, int y, int w, int h, uint32_t color) {
    for (int i = y; i < y + h; i++) {
        if (i < 0 || i >= vinfo.yres) continue;
        for (int j = x; j < x + w; j++) {
            if (j < 0 || j >= vinfo.xres) continue;
            buf[i * vinfo.xres + j] = color;
        }
    }
}

void draw_char(uint32_t *buf, char c, int x, int y, uint32_t color, int scale) {
    char *bitmap = font8x8_basic[(int)c];
    for (int r = 0; r < 8; r++) {
        for (int c_bit = 0; c_bit < 8; c_bit++) {
            if (bitmap[r] & (1 << c_bit)) {
                draw_rect(buf, x + c_bit * scale, y + r * scale, scale, scale, color);
            }
        }
    }
}

void draw_text(uint32_t *buf, const char *str, int x, int y, uint32_t color, int scale) {
    int cur_x = x;
    while (*str) {
        draw_char(buf, *str, cur_x, y, color, scale);
        cur_x += 8 * scale + 2;
        str++;
    }
}

void draw_cursor(uint32_t *buf, int x, int y) {
    uint32_t color = 0x00FFFFFF; // White cursor
    // Simple crosshair
    draw_rect(buf, x - 1, y - 8, 3, 17, color);
    draw_rect(buf, x - 8, y - 1, 17, 3, color);
    draw_rect(buf, x, y - 7, 1, 15, 0x00000000); // Black inner
    draw_rect(buf, x - 7, y, 15, 1, 0x00000000);
}

int main() {
    printf("Starting Nexa OS Desktop...\n");

    fbfd = open("/dev/fb0", O_RDWR);
    if (fbfd == -1) {
        perror("Error: cannot open framebuffer device");
        return 1;
    }

    if (ioctl(fbfd, FBIOGET_FSCREENINFO, &finfo) == -1) {
        perror("Error reading fixed information");
        return 1;
    }

    if (ioctl(fbfd, FBIOGET_VSCREENINFO, &vinfo) == -1) {
        perror("Error reading variable information");
        return 1;
    }

    printf("Resolution: %dx%d, %dbpp\n", vinfo.xres, vinfo.yres, vinfo.bits_per_pixel);

    // Ensure 32bpp
    if (vinfo.bits_per_pixel != 32) {
        printf("Warning: Framebuffer is not 32bpp. It is %dbpp. Colors may be wrong.\n", vinfo.bits_per_pixel);
    }

    long screensize = vinfo.yres_virtual * finfo.line_length;
    fbp = (uint32_t *)mmap(0, screensize, PROT_READ | PROT_WRITE, MAP_SHARED, fbfd, 0);
    if ((long)fbp == -1) {
        perror("Error: failed to map framebuffer device to memory");
        return 1;
    }

    backbuf = (uint32_t *)malloc(vinfo.xres * vinfo.yres * 4);
    if (!backbuf) {
        printf("Error: failed to allocate backbuffer\n");
        return 1;
    }

    // Set non-blocking read for mouse
    mouse_fd = open("/dev/input/mice", O_RDONLY | O_NONBLOCK);
    if (mouse_fd == -1) {
        printf("Warning: could not open /dev/input/mice\n");
    }

    mouse_x = vinfo.xres / 2;
    mouse_y = vinfo.yres / 2;

    uint32_t bg_color = 0x001E1E2E; // Dark Slate / Catppuccin Base
    uint32_t taskbar_color = 0x00181825; // Darker Taskbar
    uint32_t text_color = 0x00CDD6F4; // Light Text

    while (1) {
        // Read mouse events
        if (mouse_fd != -1) {
            signed char mbuf[3];
            int bytes = read(mouse_fd, mbuf, 3);
            if (bytes == 3) {
                mouse_x += mbuf[1];
                mouse_y -= mbuf[2]; // Y is inverted

                if (mouse_x < 0) mouse_x = 0;
                if (mouse_y < 0) mouse_y = 0;
                if (mouse_x > vinfo.xres - 1) mouse_x = vinfo.xres - 1;
                if (mouse_y > vinfo.yres - 1) mouse_y = vinfo.yres - 1;
            }
        }

        // Draw Background
        draw_rect(backbuf, 0, 0, vinfo.xres, vinfo.yres, bg_color);

        // Draw Taskbar (bottom 40 pixels)
        int taskbar_h = 40;
        int taskbar_y = vinfo.yres - taskbar_h;
        draw_rect(backbuf, 0, taskbar_y, vinfo.xres, taskbar_h, taskbar_color);

        // Draw Taskbar Accent Line
        draw_rect(backbuf, 0, taskbar_y, vinfo.xres, 2, 0x0089B4FA); // Blue accent

        // Draw Text
        draw_text(backbuf, "NEXA OS", 20, taskbar_y + 12, text_color, 2);
        
        // Draw centered welcome text
        draw_text(backbuf, "Welcome to Nexa OS GUI", vinfo.xres/2 - 200, vinfo.yres/2 - 20, 0x00F38BA8, 3);
        draw_text(backbuf, "Written in pure C. Zero flickering.", vinfo.xres/2 - 180, vinfo.yres/2 + 20, text_color, 2);

        // Draw Cursor
        draw_cursor(backbuf, mouse_x, mouse_y);

        // Copy backbuffer to real framebuffer (Double Buffering)
        memcpy(fbp, backbuf, vinfo.xres * vinfo.yres * 4);

        // Sleep to limit frame rate (~60 FPS)
        usleep(16000);
    }

    free(backbuf);
    munmap(fbp, screensize);
    close(fbfd);
    if (mouse_fd != -1) close(mouse_fd);
    return 0;
}
