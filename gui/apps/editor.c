#include "../libnexa.h"
#include <sys/stat.h>
#include <libgen.h>

static char edbuf[8192]="";
static int edcur=0;
static char edfile[256]="";
static int edld=0;

static void ed_load(void){
    if(edld)return;
    FILE *f=fopen(edfile,"r");
    if(f){size_t n=fread(edbuf,1,sizeof(edbuf)-1,f);edbuf[n]=0;fclose(f);}
    else strcpy(edbuf,"");
    edcur=(int)strlen(edbuf);edld=1;
}

static void ed_save(void){
    /* Ensure directory exists (simplified) */
    char tpath[256]; strncpy(tpath, edfile, 255);
    char *dname = dirname(tpath);
    mkdir(dname, 0755);

    FILE *f=fopen(edfile,"w");
    if(f){fputs(edbuf,f);fclose(f);}
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    nx_fill_rect(4, 4, nx_w-8, 18, C(17,24,39));
    nx_draw_text(8, 6, edfile, 1, NX_AMBER);
    
    char sv[]="[CTRL+S SAVE]";
    nx_draw_text(nx_w-8-nx_text_width(sv,1), 6, sv, 1, NX_TXT_DIM);
    
    int pad=6, ty=26, cw=6, ch=10, cols=(nx_w-2*pad)/cw, rows=(nx_h-30)/ch;
    if(cols<1) cols=1; if(rows<1) rows=1;
    
    int tl=1; for(int i=0; edbuf[i]; i++) if(edbuf[i]=='\n') tl++;
    int skip=tl-rows; if(skip<0) skip=0;
    
    const char *p = edbuf;
    while(skip>0 && *p){ if(*p=='\n') skip--; p++; }
    
    int cy = ty; char line[256];
    for(int row=0; row<rows && *p; row++){
        int i=0; while(*p && *p!='\n' && i<cols && i<255) line[i++] = *p++;
        line[i] = 0; if(*p=='\n') p++;
        nx_draw_text(pad, cy, line, 1, NX_TXT);
        cy += ch;
    }
    
    nx_fill_rect(pad + (edcur%cols)*cw, ty + (edcur/cols)*ch, 2, ch, NX_TXT_GREEN);
    nexa_flip();
}

int main(int argc, char **argv){
    if(argc > 1){
        strncpy(edfile, argv[1], 255);
    } else {
        strcpy(edfile, "/disk/data/notes.txt");
    }

    if(nexa_init("EDITOR", 420, 360, APP_EDITOR) < 0) return 1;

    ed_load();
    draw();

    NxEvent ev;
    int running = 1;

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
            else if(ev.type == NX_EV_KEY){
                char c = ev.key;
                if(c == 19) ed_save(); /* Ctrl+S */
                else if(c == '\n' || c == '\r'){
                    if(edcur < (int)sizeof(edbuf)-1){
                        memmove(edbuf+edcur+1, edbuf+edcur, strlen(edbuf+edcur)+1);
                        edbuf[edcur] = '\n'; edcur++; draw();
                    }
                } else if(c == 127 || c == '\b'){
                    if(edcur > 0){
                        memmove(edbuf+edcur-1, edbuf+edcur, strlen(edbuf+edcur)+1);
                        edcur--; draw();
                    }
                } else if(c >= 32 && c <= 126 && edcur < (int)sizeof(edbuf)-1){
                    memmove(edbuf+edcur+1, edbuf+edcur, strlen(edbuf+edcur)+1);
                    edbuf[edcur] = c; edcur++; draw();
                }
            }
        }
    }
    return 0;
}
