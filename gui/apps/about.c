#include "../libnexa.h"

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    int cx=nx_w/2, cy2=30;
    nx_draw_text(cx-nx_text_width("NEXA",4)/2, cy2, "NEXA", 4, NX_BLUE); cy2+=38;
    nx_draw_text(cx-nx_text_width("OPERATING SYSTEM",2)/2, cy2, "OPERATING SYSTEM", 2, NX_TXT_DIM); cy2+=30;
    nx_draw_hline(20, cy2, nx_w-40, NX_BG_LIGHTER); cy2+=15;
    nx_draw_text(20, cy2, "VERSION:    1.0", 1, NX_TXT); cy2+=18;
    nx_draw_text(20, cy2, "KERNEL:     LINUX 6.18", 1, NX_TXT); cy2+=18;
    nx_draw_text(20, cy2, "SHELL:      NEXA SHELL", 1, NX_TXT); cy2+=18;
    nx_draw_text(20, cy2, "WM:         DISPLAY SERVER", 1, NX_TXT); cy2+=18;
    nx_draw_text(20, cy2, "DISPLAY:    FRAMEBUFFER", 1, NX_TXT); cy2+=18;
    nx_draw_text(20, cy2, "ARCH:       X86-64", 1, NX_TXT); cy2+=30;
    nx_draw_hline(20, cy2, nx_w-40, NX_BG_LIGHTER); cy2+=15;
    nx_draw_text(cx-nx_text_width("BUILT FROM SCRATCH",1)/2, cy2, "BUILT FROM SCRATCH", 1, NX_TXT_DIM);
    nexa_flip();
}

int main(void){
    if(nexa_init("ABOUT", 320, 340, APP_ABOUT) < 0) return 1;

    NxEvent ev;
    int running = 1;
    draw();

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
        }
    }
    return 0;
}
