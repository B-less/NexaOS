#include "../libnexa.h"
#include <dirent.h>
#include <sys/stat.h>

static char fmp[256] = "/disk";
static char fmn[64][128];
static int fmd[64];
static int fmc=0, fms=0;

/* File Associations */
#define MAX_ASSOC 16
static char assoc_ext[MAX_ASSOC][16];
static char assoc_app[MAX_ASSOC][64];
static int assoc_cnt = 0;

static void load_assoc(void){
    FILE *f = fopen("/disk/etc/associations.conf", "r");
    if(!f) return;
    char line[128];
    while(fgets(line, sizeof(line), f) && assoc_cnt < MAX_ASSOC){
        char *eq = strchr(line, '=');
        if(eq){
            *eq = 0;
            char *ext = line; char *app = eq+1;
            app[strcspn(app, "\n")] = 0;
            strncpy(assoc_ext[assoc_cnt], ext, 15);
            strncpy(assoc_app[assoc_cnt], app, 63);
            assoc_cnt++;
        }
    }
    fclose(f);
}

static const char* get_assoc(const char *ext){
    for(int i=0; i<assoc_cnt; i++){
        if(strcmp(assoc_ext[i], ext)==0) return assoc_app[i];
    }
    return NULL;
}

static const char* get_ext(const char *filename){
    const char *dot = strrchr(filename, '.');
    if(!dot || dot == filename) return "";
    return dot;
}

/* Context Menu State */
static int cmenu_active = 0;
static int cmenu_x = 0, cmenu_y = 0;
static int cmenu_idx = -1; /* which file is targeted */

static void fm_ref(void){
    fmc=0; fms=0; cmenu_active=0;
    DIR *d=opendir(fmp); 
    if(!d) return;
    struct dirent *e;
    while((e=readdir(d)) && fmc<64){
        if(e->d_name[0]=='.' && e->d_name[1]==0) continue;
        strncpy(fmn[fmc], e->d_name, 127);
        char full[512]; snprintf(full, sizeof(full), "%s/%s", fmp, e->d_name);
        struct stat st; fmd[fmc]=(stat(full,&st)==0 && S_ISDIR(st.st_mode)); 
        fmc++;
    }
    closedir(d);
}

/* 12x12 Pixel Icons */
static const char *ic_folder[]={
"  ........  ",
" .        . ",
".          .",
"............",
".          .",
".          .",
".          .",
".          .",
".          .",
".          .",
".          .",
"............"};

static const char *ic_doc[]={
" ........   ",
" .      ..  ",
" .      . . ",
" .      ... ",
" .        . ",
" .  ....  . ",
" .        . ",
" .  ....  . ",
" .        . ",
" .  ....  . ",
" .        . ",
" .......... "};

static const char *ic_img[]={
" .......... ",
" .        . ",
" .   O    . ",
" .        . ",
" .      /\\ .",
" .     /  \\.",
" .  /\\/    .",
" . /  \\    .",
" ./    \\   .",
" .        . ",
" .        . ",
" .......... "};

static const char *ic_gear[]={
"   ..  ..   ",
"  .  ..  .  ",
" ..      .. ",
" .   ..   . ",
" .  .  .  . ",
" .  .  .  . ",
" .   ..   . ",
" ..      .. ",
"  .  ..  .  ",
"   ..  ..   ",
"            ",
"            "};

static void draw_icon(int x, int y, const char **ic, Color c){
    for(int r=0; r<12; r++){
        for(int c2=0; c2<12; c2++){
            if(ic[r][c2] != ' ') nx_px(x+c2, y+r, c);
        }
    }
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    nx_fill_rect(4, 4, nx_w-8, 20, C(17,24,39));
    nx_draw_text(8, 7, fmp, 1, NX_AMBER);
    
    int rh=20, hdr=28, rows=(nx_h-hdr)/rh;
    for(int i=0; i<rows && i+fms<fmc; i++){
        int idx = i+fms, ey = hdr + i*rh;
        nx_fill_rect(2, ey, nx_w-4, rh, i%2 ? C(20,30,50) : NX_BG_DARK);
        if(fmd[idx]){
            draw_icon(6, ey+4, ic_folder, NX_AMBER);
            nx_draw_text(24, ey+6, fmn[idx], 1, NX_AMBER);
        } else {
            const char *ext = get_ext(fmn[idx]);
            const char **ic = ic_doc; Color cc = NX_TXT;
            if(strcmp(ext, ".ppm")==0 || strcmp(ext, ".bmp")==0){ ic = ic_img; cc = NX_PURPLE; }
            else if(strcmp(ext, ".sh")==0){ ic = ic_gear; cc = NX_TXT_GREEN; }
            else if(strcmp(ext, ".c")==0 || strcmp(ext, ".h")==0){ cc = NX_BLUE; }
            draw_icon(6, ey+4, ic, cc);
            nx_draw_text(24, ey+6, fmn[idx], 1, cc);
        }
    }

    if(cmenu_active){
        nx_fill_rect(cmenu_x, cmenu_y, 100, 60, NX_BG_LIGHT);
        nx_draw_rect(cmenu_x, cmenu_y, 100, 60, NX_WHITE);
        nx_draw_text(cmenu_x+10, cmenu_y+8, "OPEN", 1, NX_WHITE);
        nx_draw_text(cmenu_x+10, cmenu_y+28, "PROPS", 1, NX_WHITE);
        nx_draw_text(cmenu_x+10, cmenu_y+48, "DELETE", 1, NX_TXT_RED);
    }
    
    nexa_flip();
}

static void exec_file(int idx, int action){
    char full[512]; snprintf(full, sizeof(full), "%s/%s", fmp, fmn[idx]);
    if(fmd[idx]){
        /* Directory Navigation */
        if(strcmp(fmn[idx], "..")==0){
            char *sl = strrchr(fmp, '/');
            if(sl && sl!=fmp) *sl=0; else strcpy(fmp, "/");
        } else {
            strncpy(fmp, full, 255);
        }
        fm_ref();
    } else {
        /* File Execution */
        char cmd[1024];
        if(action == 1){
            snprintf(cmd, sizeof(cmd), "/disk/apps/gui/props %s &", full);
        } else {
            const char *ext = get_ext(fmn[idx]);
            const char *app = get_assoc(ext);
            if(app){
                snprintf(cmd, sizeof(cmd), "%s %s &", app, full);
            } else if(strcmp(ext, ".sh")==0){
                snprintf(cmd, sizeof(cmd), "%s &", full);
            } else {
                snprintf(cmd, sizeof(cmd), "/disk/apps/gui/editor %s &", full);
            }
        }
        system(cmd);
    }
}

static void del_file(int idx){
    char full[512]; snprintf(full, sizeof(full), "%s/%s", fmp, fmn[idx]);
    if(!fmd[idx]){
        unlink(full);
        fm_ref();
    }
}

int main(void){
    if(nexa_init("FILES", 360, 380, APP_FILES) < 0) return 1;

    load_assoc();
    fm_ref();
    draw();

    NxEvent ev;
    int running = 1;

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE){ cmenu_active=0; draw(); }
            else if(ev.type == NX_EV_MOUSE && ev.mbtn){
                if(cmenu_active){
                    if(ev.mx >= cmenu_x && ev.mx < cmenu_x+100 && ev.my >= cmenu_y && ev.my < cmenu_y+60){
                        int my = ev.my - cmenu_y;
                        if(my < 20) exec_file(cmenu_idx, 0);       // OPEN
                        else if(my < 40) exec_file(cmenu_idx, 1);  // PROPS
                        else del_file(cmenu_idx);                  // DELETE
                    }
                    cmenu_active = 0;
                    draw();
                } else {
                    int rh=20, hdr=28, idx=fms+(ev.my-hdr)/rh;
                    if(ev.my >= hdr && idx >= 0 && idx < fmc){
                        if(ev.mbtn == 1){ /* Left click -> Execute */
                            exec_file(idx, 0);
                            draw();
                        } else if(ev.mbtn == 2){ /* Right click -> Context Menu */
                            cmenu_active = 1;
                            cmenu_x = ev.mx; cmenu_y = ev.my;
                            cmenu_idx = idx;
                            if(cmenu_x + 100 > nx_w) cmenu_x = nx_w - 100;
                            if(cmenu_y + 60 > nx_h) cmenu_y = nx_h - 60;
                            draw();
                        }
                    }
                }
            }
        }
    }
    return 0;
}
