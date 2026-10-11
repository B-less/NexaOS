#include "../libnexa.h"
#include <unistd.h>
#include <fcntl.h>
#include <signal.h>
#include <sys/wait.h>

static char wavfile[256]="";
static int playing=0;
static pid_t child_pid=0;

static void start_playback(void){
    if(playing) return;
    child_pid = fork();
    if(child_pid == 0){
        int dsp = open("/dev/dsp", O_WRONLY);
        if(dsp >= 0){
            int f = open(wavfile, O_RDONLY);
            if(f >= 0){
                lseek(f, 44, SEEK_SET);
                char buf[4096];
                int n;
                while((n = read(f, buf, sizeof(buf))) > 0){
                    write(dsp, buf, n);
                }
                close(f);
            }
            close(dsp);
        }
        exit(0);
    }
    playing = 1;
}

static void stop_playback(void){
    if(!playing) return;
    if(child_pid > 0){
        kill(child_pid, SIGKILL);
        waitpid(child_pid, NULL, 0);
    }
    playing = 0;
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    
    nx_fill_rect(4, 4, nx_w-8, 20, C(17,24,39));
    nx_draw_text(8, 7, "MEDIA PLAYER", 1, NX_BLUE);
    
    nx_draw_text(10, 40, "FILE:", 1, NX_TXT_DIM);
    nx_draw_text(50, 40, wavfile, 1, NX_AMBER);
    
    int bw=80, bh=30, by=80;
    
    nx_fill_rect(20, by, bw, bh, NX_TEAL);
    nx_draw_rect(20, by, bw, bh, NX_WHITE);
    nx_draw_text(40, by+10, "PLAY", 1, NX_WHITE);
    
    nx_fill_rect(120, by, bw, bh, NX_TXT_RED);
    nx_draw_rect(120, by, bw, bh, NX_WHITE);
    nx_draw_text(140, by+10, "STOP", 1, NX_WHITE);
    
    if(playing) nx_draw_text(20, 130, "STATUS: PLAYING...", 1, NX_TXT_GREEN);
    else nx_draw_text(20, 130, "STATUS: STOPPED", 1, NX_TXT_DIM);
    
    nexa_flip();
}

int main(int argc, char **argv){
    if(argc > 1) strncpy(wavfile, argv[1], 255);
    else strcpy(wavfile, "/disk/data/test.wav");

    if(nexa_init("AUDIO", 260, 180, APP_FILES) < 0) return 1;

    draw();

    NxEvent ev;
    int running = 1;

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
            else if(ev.type == NX_EV_MOUSE && ev.mbtn){
                if(ev.my >= 80 && ev.my < 110){
                    if(ev.mx >= 20 && ev.mx < 100){ start_playback(); draw(); }
                    else if(ev.mx >= 120 && ev.mx < 200){ stop_playback(); draw(); }
                }
            }
        }
        /* Check if child died naturally to update status */
        if(playing && child_pid > 0){
            int st;
            if(waitpid(child_pid, &st, WNOHANG) > 0){
                playing = 0;
                draw();
            }
        }
    }
    stop_playback();
    return 0;
}
