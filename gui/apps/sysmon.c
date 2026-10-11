#include "../libnexa.h"
#include <sys/statvfs.h>

static void rline(const char *path, char *buf, int sz){
    buf[0]=0; FILE *f=fopen(path,"r");
    if(f){fgets(buf,sz,f);fclose(f);}
    buf[strcspn(buf,"\n")]=0;
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    int ly = 10, pad = 8, lh = 22;
    nx_draw_text(pad, ly, "SYSTEM MONITOR", 2, NX_BLUE); ly += lh+8;
    nx_draw_hline(pad, ly-4, nx_w-2*pad, NX_BG_LIGHTER); ly += 4;
    
    char buf[256];
    rline("/proc/version", buf, sizeof(buf)); buf[40]=0;
    nx_draw_text(pad, ly, "KERNEL:", 1, NX_TXT_DIM); nx_draw_text(pad+50, ly, buf, 1, NX_TXT); ly += lh;
    
    FILE *f=fopen("/proc/uptime","r");
    double up=0; if(f){fscanf(f,"%lf",&up);fclose(f);}
    int um=(int)(up/60), us=(int)up%60;
    snprintf(buf, sizeof(buf), "%d MIN %d SEC", um, us);
    nx_draw_text(pad, ly, "UPTIME:", 1, NX_TXT_DIM); nx_draw_text(pad+50, ly, buf, 1, NX_TXT); ly += lh;
    
    f=fopen("/proc/meminfo","r");
    long total=0, avail=0;
    if(f){
        char l2[256];
        while(fgets(l2,sizeof(l2),f)){
            sscanf(l2,"MemTotal: %ld",&total); sscanf(l2,"MemAvailable: %ld",&avail);
        }
        fclose(f);
    }
    long used = total-avail;
    snprintf(buf, sizeof(buf), "%ld / %ld KB", used, total);
    nx_draw_text(pad, ly, "MEMORY:", 1, NX_TXT_DIM); nx_draw_text(pad+50, ly, buf, 1, NX_TXT); ly += lh;
    
    int bw2 = nx_w-2*pad-4, bx = pad+2;
    nx_fill_rect(bx, ly, bw2, 12, NX_BG_LIGHTER);
    if(total>0) nx_fill_rect(bx, ly, (int)((long long)bw2*used/total), 12, NX_BLUE); ly += lh;
    
    f=fopen("/proc/cpuinfo","r");
    int cpus=0; if(f){char l3[256]; while(fgets(l3,sizeof(l3),f)) if(strncmp(l3,"processor",9)==0) cpus++; fclose(f);}
    snprintf(buf, sizeof(buf), "%d", cpus);
    nx_draw_text(pad, ly, "CPUS:", 1, NX_TXT_DIM); nx_draw_text(pad+50, ly, buf, 1, NX_TXT); ly += lh;
    
    struct statvfs sv;
    if(statvfs("/disk",&sv)==0){
        long long dt=(long long)sv.f_blocks*sv.f_frsize/1024, df=(long long)sv.f_bfree*sv.f_frsize/1024, du=dt-df;
        snprintf(buf, sizeof(buf), "%lld / %lld KB", du, dt);
        nx_draw_text(pad, ly, "DISK:", 1, NX_TXT_DIM); nx_draw_text(pad+50, ly, buf, 1, NX_TXT); ly += lh;
        nx_fill_rect(bx, ly, bw2, 12, NX_BG_LIGHTER);
        if(dt>0) nx_fill_rect(bx, ly, (int)(bw2*du/dt), 12, NX_TEAL); ly += lh;
    }
    
    nexa_flip();
}

int main(void){
    if(nexa_init("SYSTEM", 340, 360, APP_SYSMON) < 0) return 1;

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
