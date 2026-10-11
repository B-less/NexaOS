#include "../libnexa.h"

/* POSIX ustar header format */
typedef struct {
    char name[100];
    char mode[8];
    char uid[8];
    char gid[8];
    char size[12];
    char mtime[12];
    char chksum[8];
    char typeflag;
    char linkname[100];
    char magic[6];
    char version[2];
    char uname[32];
    char gname[32];
    char devmajor[8];
    char devminor[8];
    char prefix[155];
    char padding[12];
} TarHeader;

static char arcfile[256]="";
static char arcent[64][128];
static int arccnt=0;
static int arcld=0;
static char status[128]="READY";

static int oct2int(const char *oct, int len){
    int res = 0;
    for(int i=0; i<len; i++){
        if(oct[i] >= '0' && oct[i] <= '7') res = (res<<3) + (oct[i]-'0');
    }
    return res;
}

static void arc_load(void){
    if(arcld) return;
    FILE *f = fopen(arcfile, "rb");
    if(!f){ strcpy(status, "FILE NOT FOUND"); return; }
    
    TarHeader hdr;
    while(fread(&hdr, 1, 512, f) == 512){
        if(hdr.name[0] == 0) break;
        if(arccnt < 64){
            strncpy(arcent[arccnt], hdr.name, 127);
            arccnt++;
        }
        int sz = oct2int(hdr.size, 11);
        int blocks = (sz + 511) / 512;
        fseek(f, blocks * 512, SEEK_CUR);
    }
    fclose(f);
    arcld = 1;
}

static void arc_extract(void){
    FILE *f = fopen(arcfile, "rb");
    if(!f) return;
    
    system("mkdir -p /disk/data/extracted");
    
    TarHeader hdr;
    while(fread(&hdr, 1, 512, f) == 512){
        if(hdr.name[0] == 0) break;
        int sz = oct2int(hdr.size, 11);
        int blocks = (sz + 511) / 512;
        
        if(hdr.typeflag == '0' || hdr.typeflag == 0){
            char path[512]; snprintf(path, sizeof(path), "/disk/data/extracted/%s", hdr.name);
            FILE *out = fopen(path, "wb");
            if(out){
                int rem = sz;
                char buf[512];
                for(int b=0; b<blocks; b++){
                    fread(buf, 1, 512, f);
                    int write_sz = rem > 512 ? 512 : rem;
                    fwrite(buf, 1, write_sz, out);
                    rem -= write_sz;
                }
                fclose(out);
            } else {
                fseek(f, blocks * 512, SEEK_CUR);
            }
        } else {
            fseek(f, blocks * 512, SEEK_CUR);
        }
    }
    fclose(f);
    strcpy(status, "EXTRACTED TO /disk/data/extracted");
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    
    nx_fill_rect(4, 4, nx_w-8, 20, C(17,24,39));
    nx_draw_text(8, 7, arcfile, 1, NX_AMBER);
    
    nx_draw_text(8, 30, "CONTENTS:", 1, NX_TXT_DIM);
    
    int rh=18, cy=50;
    for(int i=0; i<arccnt && cy < nx_h-60; i++){
        nx_draw_text(16, cy, arcent[i], 1, NX_TXT);
        cy += rh;
    }
    
    int by = nx_h-40, bw = 100, bh = 24, bx = (nx_w-bw)/2;
    nx_fill_rect(bx, by, bw, bh, NX_BLUE);
    nx_draw_rect(bx, by, bw, bh, NX_WHITE);
    nx_draw_text(bx+14, by+8, "EXTRACT", 1, NX_WHITE);
    
    nx_draw_text(8, nx_h-12, status, 1, NX_TXT_GREEN);
    
    nexa_flip();
}

int main(int argc, char **argv){
    if(argc > 1){
        strncpy(arcfile, argv[1], 255);
    } else {
        strcpy(arcfile, "/disk/data/test.tar");
    }

    if(nexa_init("ARCHIVE", 340, 360, APP_FILES) < 0) return 1;

    arc_load();
    draw();

    NxEvent ev;
    int running = 1;

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
            else if(ev.type == NX_EV_MOUSE && ev.mbtn){
                int by = nx_h-40, bw = 100, bh = 24, bx = (nx_w-bw)/2;
                if(ev.mx >= bx && ev.mx < bx+bw && ev.my >= by && ev.my < by+bh){
                    strcpy(status, "EXTRACTING..."); draw();
                    arc_extract(); draw();
                }
            }
        }
    }
    return 0;
}
