#include "../libnexa.h"

static char imgfile[256]="";
static int img_w=0, img_h=0;
static uint8_t *img_data=NULL;
static int imgld=0;

static void img_load(void){
    if(imgld) return;
    imgld=1;
    FILE *f=fopen(imgfile,"rb");
    if(!f) return;
    
    char header[3];
    if(fscanf(f, "%2s\n", header)!=1 || strcmp(header,"P6")!=0){ fclose(f); return; }
    
    // Skip comments
    int c = getc(f);
    while(c == '#'){ while(getc(f) != '\n'); c = getc(f); }
    ungetc(c, f);
    
    int maxval;
    if(fscanf(f, "%d %d\n%d\n", &img_w, &img_h, &maxval)!=3){ fclose(f); return; }
    
    img_data = malloc(img_w * img_h * 3);
    if(img_data){
        fread(img_data, 1, img_w * img_h * 3, f);
    }
    fclose(f);
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    nx_fill_rect(4, 4, nx_w-8, 18, C(17,24,39));
    nx_draw_text(8, 6, imgfile, 1, NX_AMBER);
    
    if(!img_data){
        nx_draw_text(10, 40, "IMAGE NOT FOUND OR INVALID FORMAT", 1, NX_TXT_RED);
    } else {
        int ox = (nx_w - img_w)/2; if(ox<0) ox=0;
        int oy = 30 + (nx_h - 30 - img_h)/2; if(oy<30) oy=30;
        
        for(int y=0; y<img_h && oy+y < nx_h; y++){
            for(int x=0; x<img_w && ox+x < nx_w; x++){
                uint8_t r = img_data[(y*img_w+x)*3 + 0];
                uint8_t g = img_data[(y*img_w+x)*3 + 1];
                uint8_t b = img_data[(y*img_w+x)*3 + 2];
                nx_px(ox+x, oy+y, C(r,g,b));
            }
        }
    }
    nexa_flip();
}

int main(int argc, char **argv){
    if(argc > 1){
        strncpy(imgfile, argv[1], 255);
    } else {
        strcpy(imgfile, "/disk/data/test.ppm");
    }

    if(nexa_init("IMAGE VIEWER", 400, 360, APP_FILES) < 0) return 1;

    img_load();
    draw();

    NxEvent ev;
    int running = 1;

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
        }
    }
    
    if(img_data) free(img_data);
    return 0;
}
