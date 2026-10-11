#include "../libnexa.h"
#include <time.h>

int main(void){
    if(nexa_init("CLOCK", 280, 320, APP_CLOCK) < 0) return 1;

    NxEvent ev;
    int running = 1;
    time_t last_time = 0;

    while(running){
        time_t now = time(NULL);
        if(now != last_time){
            last_time = now;
            struct tm *t = localtime(&now);

            nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
            
            int cx = nx_w/2, cy = nx_h/2 - 20;
            int r = (nx_w < nx_h ? nx_w : nx_h)/2 - 35;
            if(r < 25) r = 25;

            nx_fill_circle(cx, cy, r, NX_BG_MID);
            nx_draw_circle(cx, cy, r, NX_TXT_DIM);
            nx_draw_circle(cx, cy, r-1, NX_TXT_DIM);

            for(int i=0; i<12; i++){
                double a = i*M_PI/6.0 - M_PI/2.0;
                int mx = cx + (int)((r-8)*cos(a)), my = cy + (int)((r-8)*sin(a));
                nx_fill_rect(mx-2, my-2, 5, 5, NX_TXT);
            }

            if(t){
                double ha = (t->tm_hour%12 + t->tm_min/60.0)*M_PI/6.0 - M_PI/2.0;
                int hl = r*50/100;
                for(int k=0; k<hl; k++) nx_fill_rect(cx+(int)(k*cos(ha))-1, cy+(int)(k*sin(ha))-1, 3, 3, NX_TXT);

                double ma = t->tm_min*M_PI/30.0 - M_PI/2.0;
                int ml = r*75/100;
                for(int k=0; k<ml; k++) nx_fill_rect(cx+(int)(k*cos(ma)), cy+(int)(k*sin(ma)), 2, 2, NX_TXT_DIM);

                double sa = t->tm_sec*M_PI/30.0 - M_PI/2.0;
                int sl = r*85/100;
                for(int k=0; k<sl; k++) nx_fill_rect(cx+(int)(k*cos(sa)), cy+(int)(k*sin(sa)), 1, 1, NX_TXT_RED);
                nx_fill_circle(cx, cy, 4, NX_TXT_RED);

                char ts[16]; snprintf(ts, sizeof(ts), "%02d:%02d:%02d", t->tm_hour, t->tm_min, t->tm_sec);
                nx_draw_text(cx - nx_text_width(ts, 2)/2, cy + r + 10, ts, 2, NX_TXT);
            }
            nexa_flip();
        }

        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) last_time = 0; 
        }
    }
    return 0;
}
