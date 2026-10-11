#include "../libnexa.h"

#define MAX_TODO 20
static char tdi[MAX_TODO][128];
static int tdd[MAX_TODO];
static int tdc=0;
static char tdinp[128]={0};
static int tdinp_len=0;

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    nx_draw_text(8, 8, "TODO LIST", 2, NX_PURPLE);
    nx_draw_hline(8, 26, nx_w-16, NX_BG_LIGHTER);
    
    int ty=30, rh=22;
    for(int i=0; i<tdc && ty < nx_h-30; i++){
        nx_draw_rect(8, ty+3, 14, 14, NX_TXT_DIM);
        if(tdd[i]){
            nx_fill_rect(10, ty+5, 10, 10, NX_TXT_GREEN);
            nx_draw_text(10, ty+5, "X", 1, NX_BG_DARK);
        }
        nx_draw_text(28, ty+4, tdi[i], 1, tdd[i] ? NX_TXT_DIM : NX_TXT);
        ty += rh;
    }
    
    nx_fill_rect(4, nx_h-24, nx_w-8, 20, C(17,24,39));
    nx_draw_rect(4, nx_h-24, nx_w-8, 20, NX_BG_LIGHTER);
    char inp[160]; snprintf(inp, sizeof(inp), "+ %s_", tdinp);
    nx_draw_text(8, nx_h-20, inp, 1, NX_TXT_GREEN);
    
    nexa_flip();
}

int main(void){
    if(nexa_init("TODO", 320, 340, APP_TODO) < 0) return 1;

    NxEvent ev;
    int running = 1;
    draw();

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
            else if(ev.type == NX_EV_MOUSE && ev.mbtn){
                int hdr = 30, rh = 22, idx = (ev.my - hdr) / rh;
                if(ev.my >= hdr && idx >= 0 && idx < tdc){
                    tdd[idx] = !tdd[idx];
                    draw();
                }
            }
            else if(ev.type == NX_EV_KEY){
                char c = ev.key;
                if(c == '\n' || c == '\r'){
                    if(tdinp_len > 0 && tdc < MAX_TODO){
                        strncpy(tdi[tdc], tdinp, 127);
                        tdd[tdc] = 0; tdc++;
                        tdinp[0] = 0; tdinp_len = 0;
                        draw();
                    }
                } else if(c == 127 || c == '\b'){
                    if(tdinp_len > 0){
                        tdinp[--tdinp_len] = 0;
                        draw();
                    }
                } else if(c >= 32 && c <= 126 && tdinp_len < (int)sizeof(tdinp)-1){
                    tdinp[tdinp_len++] = c;
                    tdinp[tdinp_len] = 0;
                    draw();
                }
            }
        }
    }
    return 0;
}
