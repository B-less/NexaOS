#include <SDL.h>
#include <SDL_ttf.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

enum {
    SCREEN_W = 1366,
    SCREEN_H = 768,
    TASKBAR_H = 56
};

typedef struct {
    Uint8 r;
    Uint8 g;
    Uint8 b;
    Uint8 a;
} Color;

typedef struct {
    const char *title;
    int x;
    int y;
    int w;
    int h;
    Color accent;
} WindowBox;

static Color rgb(Uint8 r, Uint8 g, Uint8 b)
{
    Color c = { r, g, b, 255 };
    return c;
}

static void set_color(SDL_Renderer *r, Color c)
{
    SDL_SetRenderDrawColor(r, c.r, c.g, c.b, c.a);
}

static void fill_rect(SDL_Renderer *r, int x, int y, int w, int h, Color c)
{
    SDL_Rect rect = { x, y, w, h };
    set_color(r, c);
    SDL_RenderFillRect(r, &rect);
}

static void draw_rect(SDL_Renderer *r, int x, int y, int w, int h, Color c)
{
    SDL_Rect rect = { x, y, w, h };
    set_color(r, c);
    SDL_RenderDrawRect(r, &rect);
}

static void fill_circle(SDL_Renderer *r, int cx, int cy, int radius, Color c)
{
    set_color(r, c);
    for (int dy = -radius; dy <= radius; dy++) {
        for (int dx = -radius; dx <= radius; dx++) {
            if (dx * dx + dy * dy <= radius * radius) {
                SDL_RenderDrawPoint(r, cx + dx, cy + dy);
            }
        }
    }
}

static void fill_round_rect(SDL_Renderer *r, int x, int y, int w, int h, int radius, Color c)
{
    if (radius <= 0) {
        fill_rect(r, x, y, w, h, c);
        return;
    }

    fill_rect(r, x + radius, y, w - radius * 2, h, c);
    fill_rect(r, x, y + radius, radius, h - radius * 2, c);
    fill_rect(r, x + w - radius, y + radius, radius, h - radius * 2, c);
    fill_circle(r, x + radius, y + radius, radius, c);
    fill_circle(r, x + w - radius - 1, y + radius, radius, c);
    fill_circle(r, x + radius, y + h - radius - 1, radius, c);
    fill_circle(r, x + w - radius - 1, y + h - radius - 1, radius, c);
}

static void draw_text(SDL_Renderer *r, TTF_Font *font, const char *text, int x, int y, Color c)
{
    SDL_Color sdl_color = { c.r, c.g, c.b, c.a };
    SDL_Surface *surface = TTF_RenderUTF8_Blended(font, text, sdl_color);

    if (!surface) {
        return;
    }

    SDL_Texture *texture = SDL_CreateTextureFromSurface(r, surface);
    if (!texture) {
        SDL_FreeSurface(surface);
        return;
    }

    SDL_Rect dst = { x, y, surface->w, surface->h };
    SDL_RenderCopy(r, texture, NULL, &dst);
    SDL_DestroyTexture(texture);
    SDL_FreeSurface(surface);
}

static void draw_title_buttons(SDL_Renderer *r, int x, int y, int w)
{
    Color black = rgb(16, 20, 24);
    int bx = x + w - 112;
    int cy = y + 26;

    set_color(r, black);
    SDL_RenderDrawLine(r, bx, cy, bx + 16, cy);

    draw_rect(r, bx + 48, cy - 8, 14, 14, black);

    SDL_RenderDrawLine(r, bx + 92, cy - 8, bx + 106, cy + 6);
    SDL_RenderDrawLine(r, bx + 106, cy - 8, bx + 92, cy + 6);
}

static void draw_folder_icon(SDL_Renderer *r, int x, int y, int scale)
{
    Color folder = rgb(71, 177, 238);
    Color edge = rgb(45, 143, 208);

    fill_round_rect(r, x, y + scale, scale * 5, scale * 3, 4, folder);
    fill_round_rect(r, x, y, scale * 2, scale * 2, 4, folder);
    draw_rect(r, x, y + scale, scale * 5, scale * 3, edge);
}

static void draw_doc_icon(SDL_Renderer *r, int x, int y)
{
    fill_round_rect(r, x, y, 28, 36, 3, rgb(248, 250, 252));
    draw_rect(r, x, y, 28, 36, rgb(145, 152, 160));
    for (int i = 0; i < 5; i++) {
        set_color(r, rgb(145, 152, 160));
        SDL_RenderDrawLine(r, x + 6, y + 9 + i * 5, x + 22, y + 9 + i * 5);
    }
}

static void draw_window(SDL_Renderer *r, TTF_Font *title_font, WindowBox win)
{
    fill_round_rect(r, win.x + 8, win.y + 10, win.w, win.h, 12, rgb(190, 196, 202));
    fill_round_rect(r, win.x, win.y, win.w, win.h, 12, rgb(255, 255, 255));
    draw_rect(r, win.x, win.y, win.w, win.h, rgb(173, 180, 188));

    fill_rect(r, win.x, win.y, win.w, 48, rgb(246, 247, 249));
    draw_rect(r, win.x, win.y + 48, win.w, 1, rgb(226, 229, 232));
    fill_round_rect(r, win.x + 20, win.y + 17, 20, 20, 4, win.accent);
    draw_text(r, title_font, win.title, win.x + 52, win.y + 12, rgb(15, 23, 42));
    draw_title_buttons(r, win.x, win.y, win.w);
}

static void draw_notepad(SDL_Renderer *r, TTF_Font *title_font, TTF_Font *ui_font)
{
    WindowBox win = { "Untitled - Notepad", 140, 72, 940, 530, rgb(66, 174, 239) };
    draw_window(r, title_font, win);
    draw_text(r, ui_font, "File   Edit   Format   View   Help", win.x + 20, win.y + 62, rgb(17, 24, 39));
    draw_rect(r, win.x, win.y + 98, win.w, 1, rgb(229, 231, 235));
}

static void draw_app_window(SDL_Renderer *r, TTF_Font *title_font)
{
    WindowBox win = { "App", 360, 260, 920, 390, rgb(66, 174, 239) };
    draw_window(r, title_font, win);
}

static void draw_file_window(SDL_Renderer *r, TTF_Font *title_font, TTF_Font *ui_font)
{
    WindowBox win = { "Home", 100, 350, 660, 360, rgb(66, 174, 239) };
    const char *dirs[] = {
        "Desktop", "Documents", "Downloads",
        "Music", "Pictures", "Public",
        "Templates", "Videos"
    };
    const char *side[] = { "Home", "Documents", "Downloads", "Music", "Pictures", "Videos" };

    draw_window(r, title_font, win);
    fill_rect(r, win.x, win.y + 48, 250, win.h - 48, rgb(247, 248, 250));
    fill_rect(r, win.x, win.y + 94, 250, 48, rgb(233, 236, 239));
    draw_rect(r, win.x + 250, win.y + 48, 1, win.h - 48, rgb(225, 229, 234));

    for (int i = 0; i < 6; i++) {
        int y = win.y + 76 + i * 44;
        if (i == 0) {
            draw_folder_icon(r, win.x + 34, y - 4, 6);
        } else if (i == 1) {
            draw_doc_icon(r, win.x + 36, y - 4);
        } else {
            fill_circle(r, win.x + 50, y + 12, 10, rgb(117, 124, 132));
        }
        draw_text(r, ui_font, side[i], win.x + 70, y, rgb(17, 24, 39));
    }

    for (int i = 0; i < 8; i++) {
        int col = i % 3;
        int row = i / 3;
        int x = win.x + 300 + col * 120;
        int y = win.y + 120 + row * 104;
        draw_folder_icon(r, x, y, 13);
        draw_text(r, ui_font, dirs[i], x - 8, y + 70, rgb(17, 24, 39));
    }
}

static void draw_taskbar(SDL_Renderer *r, TTF_Font *font)
{
    int y = SCREEN_H - TASKBAR_H;
    fill_rect(r, 0, y, SCREEN_W, TASKBAR_H, rgb(247, 248, 249));
    draw_rect(r, 0, y, SCREEN_W, 1, rgb(210, 215, 220));

    fill_rect(r, 30, y + 17, 18, 18, rgb(66, 174, 239));
    fill_rect(r, 52, y + 17, 18, 18, rgb(66, 174, 239));
    fill_rect(r, 30, y + 39, 18, 18, rgb(66, 174, 239));
    fill_rect(r, 52, y + 39, 18, 18, rgb(66, 174, 239));

    draw_folder_icon(r, 125, y + 22, 9);
    fill_round_rect(r, 218, y + 20, 38, 34, 10, rgb(66, 174, 239));
    draw_doc_icon(r, 305, y + 18);
    draw_text(r, font, "10:30 AM", SCREEN_W - 175, y + 18, rgb(17, 24, 39));
}

static void draw_mic(SDL_Renderer *r, TTF_Font *big_font)
{
    int cx = 1225;
    int cy = 120;

    fill_circle(r, cx, cy, 54, rgb(243, 245, 247));
    draw_rect(r, cx - 16, cy - 34, 32, 58, rgb(45, 52, 60));
    fill_round_rect(r, cx - 13, cy - 34, 26, 58, 13, rgb(45, 52, 60));

    set_color(r, rgb(45, 52, 60));
    SDL_RenderDrawLine(r, cx - 28, cy + 4, cx - 28, cy + 8);
    SDL_RenderDrawLine(r, cx + 28, cy + 4, cx + 28, cy + 8);
    SDL_RenderDrawLine(r, cx - 28, cy + 8, cx - 20, cy + 25);
    SDL_RenderDrawLine(r, cx + 28, cy + 8, cx + 20, cy + 25);
    SDL_RenderDrawLine(r, cx - 20, cy + 25, cx + 20, cy + 25);
    SDL_RenderDrawLine(r, cx, cy + 25, cx, cy + 45);
    SDL_RenderDrawLine(r, cx - 18, cy + 45, cx + 18, cy + 45);

    draw_text(r, big_font, "Nexa", cx - 43, cy + 70, rgb(17, 24, 39));
}

static void draw_desktop(SDL_Renderer *r, TTF_Font *title_font, TTF_Font *ui_font, TTF_Font *big_font)
{
    for (int y = 0; y < SCREEN_H; y++) {
        Uint8 shade = (Uint8)(232 + (y * 16 / SCREEN_H));
        set_color(r, rgb(shade, shade + 2, shade + 3));
        SDL_RenderDrawLine(r, 0, y, SCREEN_W, y);
    }

    draw_notepad(r, title_font, ui_font);
    draw_app_window(r, title_font);
    draw_file_window(r, title_font, ui_font);
    draw_mic(r, big_font);
    draw_taskbar(r, ui_font);
}

static TTF_Font *open_font(int size)
{
    const char *paths[] = {
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
        NULL
    };

    for (int i = 0; paths[i] != NULL; i++) {
        TTF_Font *font = TTF_OpenFont(paths[i], size);
        if (font) {
            return font;
        }
    }

    return NULL;
}

int main(int argc, char **argv)
{
    bool render_once = false;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--once") == 0) {
            render_once = true;
        }
    }

    if (SDL_Init(SDL_INIT_VIDEO) != 0) {
        fprintf(stderr, "SDL_Init failed: %s\n", SDL_GetError());
        return 1;
    }

    if (TTF_Init() != 0) {
        fprintf(stderr, "TTF_Init failed: %s\n", TTF_GetError());
        SDL_Quit();
        return 1;
    }

    SDL_Window *window = SDL_CreateWindow(
        "Nexa Desktop Prototype",
        SDL_WINDOWPOS_CENTERED,
        SDL_WINDOWPOS_CENTERED,
        SCREEN_W,
        SCREEN_H,
        SDL_WINDOW_SHOWN
    );

    if (!window) {
        fprintf(stderr, "SDL_CreateWindow failed: %s\n", SDL_GetError());
        TTF_Quit();
        SDL_Quit();
        return 1;
    }

    SDL_Renderer *renderer = SDL_CreateRenderer(window, -1, SDL_RENDERER_ACCELERATED | SDL_RENDERER_PRESENTVSYNC);
    if (!renderer) {
        renderer = SDL_CreateRenderer(window, -1, SDL_RENDERER_SOFTWARE);
    }

    if (!renderer) {
        fprintf(stderr, "SDL_CreateRenderer failed: %s\n", SDL_GetError());
        SDL_DestroyWindow(window);
        TTF_Quit();
        SDL_Quit();
        return 1;
    }

    TTF_Font *title_font = open_font(24);
    TTF_Font *ui_font = open_font(22);
    TTF_Font *big_font = open_font(32);

    if (!title_font || !ui_font || !big_font) {
        fprintf(stderr, "Could not open a system TrueType font.\n");
        if (title_font) TTF_CloseFont(title_font);
        if (ui_font) TTF_CloseFont(ui_font);
        if (big_font) TTF_CloseFont(big_font);
        SDL_DestroyRenderer(renderer);
        SDL_DestroyWindow(window);
        TTF_Quit();
        SDL_Quit();
        return 1;
    }

    bool running = true;
    while (running) {
        SDL_Event event;
        while (SDL_PollEvent(&event)) {
            if (event.type == SDL_QUIT) {
                running = false;
            }
            if (event.type == SDL_KEYDOWN && event.key.keysym.sym == SDLK_ESCAPE) {
                running = false;
            }
        }

        draw_desktop(renderer, title_font, ui_font, big_font);
        SDL_RenderPresent(renderer);

        if (render_once) {
            running = false;
        }
    }

    TTF_CloseFont(title_font);
    TTF_CloseFont(ui_font);
    TTF_CloseFont(big_font);
    SDL_DestroyRenderer(renderer);
    SDL_DestroyWindow(window);
    TTF_Quit();
    SDL_Quit();
    return 0;
}
