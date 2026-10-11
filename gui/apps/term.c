#include "../libnexa.h"

static char tinp[256]={0};
static int tinp_len=0;
static char tout[8192]="Welcome to Nexa Shell!\nType commands here.\n";

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    int pad=6, cw=12, ch=18, cols=(nx_w-2*pad)/cw, rows=(nx_h-2*pad)/ch;
    if(cols<1) cols=1; if(rows<2) rows=2;
    
    int tl=1; for(const char *p=tout; *p; p++) if(*p=='\n') tl++;
    int skip = tl-(rows-1); if(skip<0) skip=0;
    
    const char *p=tout;
    while(skip>0 && *p){ if(*p=='\n') skip--; p++; }
    
    int cy = pad, my = nx_h-pad-ch;
    char line[256];
    while(*p && cy<my){
        int i=0;
        while(*p && *p!='\n' && i<cols && i<255) line[i++] = *p++;
        line[i]=0; while(*p && *p!='\n') p++; if(*p=='\n') p++;
        nx_draw_text(pad, cy, line, 2, NX_TXT);
        cy += ch;
    }
    char inp[300]; snprintf(inp, sizeof(inp), "NEXA> %s_", tinp);
    nx_draw_text(pad, nx_h-pad-ch, inp, 2, NX_TXT_GREEN);
    nexa_flip();
}

static void term_exec(void){
    if(!tinp_len) return;
    char cmd[512];
    snprintf(cmd, sizeof(cmd), "/disk/apps/desktop-gui/nexa_wrapper.sh %s 2>&1", tinp);
    
    size_t cur = strlen(tout);
    snprintf(tout+cur, sizeof(tout)-cur, "> %s\n", tinp);
    
    FILE *fp = popen(cmd, "r");
    if(fp){
        char ck[256];
        while(fgets(ck, sizeof(ck), fp)){
            size_t cl = strlen(tout), kl = strlen(ck);
            if(cl+kl >= sizeof(tout)-1){
                size_t keep = sizeof(tout)/2;
                memmove(tout, tout+cl-keep, keep+1);
            }
            strcat(tout, ck);
        }
        pclose(fp);
    }
    tinp[0]=0; tinp_len=0;
    draw();
}

int main(void){
    if(nexa_init("TERMINAL", 520, 380, APP_TERM) < 0) return 1;

    NxEvent ev;
    int running = 1;
    draw();

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
            else if(ev.type == NX_EV_KEY){
                char c = ev.key;
                if(c == '\n' || c == '\r'){
                    if(strcmp(tinp, "exit")==0) running = 0;
                    else term_exec();
                } else if(c == 127 || c == '\b'){
                    if(tinp_len > 0){ tinp[--tinp_len] = 0; draw(); }
                } else if(c >= 32 && c <= 126 && tinp_len < (int)sizeof(tinp)-1){
                    tinp[tinp_len++] = c; tinp[tinp_len] = 0; draw();
                }
            }
        }
    }
    return 0;
}
