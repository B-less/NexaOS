#include "../libnexa.h"

static char mdfile[256]="";
#define MAX_MD_LINES 100
static char mdlines[MAX_MD_LINES][256];
static int mdcnt=0;
static int mdld=0;

static void load_md(void){
    if(mdld) return;
    FILE *f = fopen(mdfile, "r");
    if(!f) return;
    char line[256];
    while(fgets(line, sizeof(line), f) && mdcnt < MAX_MD_LINES){
        line[strcspn(line, "\n")] = 0;
        strncpy(mdlines[mdcnt], line, 255);
        mdcnt++;
    }
    fclose(f);
    mdld = 1;
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    nx_fill_rect(4, 4, nx_w-8, 20, C(17,24,39));
    nx_draw_text(8, 7, mdfile, 1, NX_AMBER);
    
    int cy = 40;
    for(int i=0; i<mdcnt && cy < nx_h-10; i++){
        char *p = mdlines[i];
        if(p[0] == '#'){
            while(*p == '#' || *p == ' ') p++;
            nx_draw_text(10, cy, p, 2, NX_BLUE);
            cy += 20;
        } else if(p[0] == '-'){
            while(*p == '-' || *p == ' ') p++;
            nx_fill_rect(14, cy+5, 4, 4, NX_AMBER);
            nx_draw_text(24, cy, p, 1, NX_TXT);
            cy += 14;
        } else if(p[0] == 0){
            cy += 10;
        } else {
            nx_draw_text(10, cy, p, 1, NX_TXT_DIM);
            cy += 14;
        }
    }
    
    nexa_flip();
}

int main(int argc, char **argv){
    if(argc > 1) strncpy(mdfile, argv[1], 255);
    else strcpy(mdfile, "/disk/data/readme.md");

    if(nexa_init("MARKDOWN", 360, 400, APP_FILES) < 0) return 1;

    load_md();
    draw();

    NxEvent ev;
    int running = 1;

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
        }
    }
    return 0;
}
