#include "../libnexa.h"

static char csvfile[256]="";
#define MAX_ROWS 100
#define MAX_COLS 10
static char cells[MAX_ROWS][MAX_COLS][64];
static int row_cnt=0;
static int col_cnt=0;

static void load_csv(void){
    FILE *f = fopen(csvfile, "r");
    if(!f) return;
    char line[512];
    while(fgets(line, sizeof(line), f) && row_cnt < MAX_ROWS){
        char *p = line;
        int c=0;
        while(*p && *p!='\n' && c < MAX_COLS){
            char *comma = strchr(p, ',');
            if(comma){
                *comma = 0;
                strncpy(cells[row_cnt][c], p, 63);
                p = comma+1;
            } else {
                strncpy(cells[row_cnt][c], p, 63);
                p += strlen(p);
            }
            c++;
        }
        if(c > col_cnt) col_cnt = c;
        row_cnt++;
    }
    fclose(f);
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    nx_fill_rect(4, 4, nx_w-8, 20, C(17,24,39));
    nx_draw_text(8, 7, csvfile, 1, NX_AMBER);
    
    int cw = 80, ch = 20;
    int ox = 10, oy = 40;
    
    for(int r=0; r<row_cnt && oy+r*ch < nx_h-10; r++){
        for(int c=0; c<col_cnt && ox+c*cw < nx_w-10; c++){
            int cx = ox + c*cw;
            int cy = oy + r*ch;
            nx_draw_rect(cx, cy, cw, ch, NX_BG_LIGHT);
            nx_draw_text(cx+4, cy+4, cells[r][c], 1, r==0 ? NX_AMBER : NX_TXT);
        }
    }
    
    nexa_flip();
}

int main(int argc, char **argv){
    if(argc > 1) strncpy(csvfile, argv[1], 255);
    else strcpy(csvfile, "/disk/data/test.csv");

    if(nexa_init("SPREADSHEET", 400, 300, APP_FILES) < 0) return 1;

    load_csv();
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
