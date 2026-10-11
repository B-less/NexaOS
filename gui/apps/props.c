#include "../libnexa.h"
#include <sys/stat.h>

static char pfile[256]="";
static char pext[16]="";
static char psize[32]="";
static char papp[128]="";
static int papp_len=0;
static char status[64]="";

static void get_props(void){
    struct stat st;
    if(stat(pfile, &st)==0){
        snprintf(psize, sizeof(psize), "%ld BYTES", (long)st.st_size);
    } else {
        strcpy(psize, "UNKNOWN");
    }
    
    const char *dot = strrchr(pfile, '.');
    if(dot && dot != pfile) strncpy(pext, dot, 15);
    else strcpy(pext, "NONE");
    
    FILE *f = fopen("/disk/etc/associations.conf", "r");
    if(f){
        char line[256];
        while(fgets(line, sizeof(line), f)){
            char *eq = strchr(line, '=');
            if(eq){
                *eq = 0;
                if(strcmp(line, pext)==0){
                    char *a = eq+1; a[strcspn(a, "\n")]=0;
                    strncpy(papp, a, 127);
                    papp_len = strlen(papp);
                    break;
                }
            }
        }
        fclose(f);
    }
}

static void save_props(void){
    if(strcmp(pext, "NONE")==0){ strcpy(status, "NO EXTENSION TO ASSOCIATE"); return; }
    
    char lines[32][256]; int lcnt=0;
    FILE *f = fopen("/disk/etc/associations.conf", "r");
    if(f){
        while(fgets(lines[lcnt], 255, f) && lcnt<31){
            lines[lcnt][strcspn(lines[lcnt], "\n")]=0;
            lcnt++;
        }
        fclose(f);
    }
    
    int found=0;
    for(int i=0; i<lcnt; i++){
        char tmp[256]; strncpy(tmp, lines[i], 255);
        char *eq = strchr(tmp, '=');
        if(eq){
            *eq = 0;
            if(strcmp(tmp, pext)==0){
                snprintf(lines[i], 255, "%s=%s", pext, papp);
                found=1; break;
            }
        }
    }
    if(!found && lcnt<31) snprintf(lines[lcnt++], 255, "%s=%s", pext, papp);
    
    f = fopen("/disk/etc/associations.conf", "w");
    if(f){
        for(int i=0; i<lcnt; i++) fprintf(f, "%s\n", lines[i]);
        fclose(f);
        strcpy(status, "ASSOCIATION SAVED!");
    } else {
        strcpy(status, "ERROR SAVING");
    }
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    
    nx_draw_text(10, 10, "FILE PROPERTIES", 2, NX_BLUE);
    nx_draw_hline(10, 30, nx_w-20, NX_BG_LIGHTER);
    
    nx_draw_text(10, 45, "FILE:", 1, NX_TXT_DIM);
    nx_draw_text(80, 45, pfile, 1, NX_TXT);
    
    nx_draw_text(10, 65, "SIZE:", 1, NX_TXT_DIM);
    nx_draw_text(80, 65, psize, 1, NX_TXT);
    
    nx_draw_text(10, 85, "EXT:", 1, NX_TXT_DIM);
    nx_draw_text(80, 85, pext, 1, NX_AMBER);
    
    nx_draw_hline(10, 110, nx_w-20, NX_BG_LIGHTER);
    nx_draw_text(10, 125, "OPEN WITH APP:", 1, NX_TXT_DIM);
    
    nx_fill_rect(10, 145, nx_w-20, 24, C(17,24,39));
    nx_draw_rect(10, 145, nx_w-20, 24, NX_TXT_DIM);
    char buf[256]; snprintf(buf, sizeof(buf), "%s_", papp);
    nx_draw_text(15, 153, buf, 1, NX_TXT_GREEN);
    
    int bx = 10, by = 185, bw = 80, bh = 24;
    nx_fill_rect(bx, by, bw, bh, NX_TEAL);
    nx_draw_rect(bx, by, bw, bh, NX_WHITE);
    nx_draw_text(bx+24, by+8, "SAVE", 1, NX_WHITE);
    
    nx_draw_text(100, 193, status, 1, NX_TXT_GREEN);
    
    nexa_flip();
}

int main(int argc, char **argv){
    if(argc > 1) strncpy(pfile, argv[1], 255);
    else strcpy(pfile, "NONE");

    if(nexa_init("PROPERTIES", 320, 240, APP_FILES) < 0) return 1;

    get_props();
    draw();

    NxEvent ev;
    int running = 1;

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
            else if(ev.type == NX_EV_KEY){
                char c = ev.key;
                if(c == 127 || c == '\b'){
                    if(papp_len > 0){ papp[--papp_len]=0; strcpy(status,""); draw(); }
                } else if(c >= 32 && c <= 126 && papp_len < 127){
                    papp[papp_len++] = c; papp[papp_len]=0; strcpy(status,""); draw();
                }
            }
            else if(ev.type == NX_EV_MOUSE && ev.mbtn){
                int bx = 10, by = 185, bw = 80, bh = 24;
                if(ev.mx >= bx && ev.mx < bx+bw && ev.my >= by && ev.my < by+bh){
                    save_props(); draw();
                }
            }
        }
    }
    return 0;
}
