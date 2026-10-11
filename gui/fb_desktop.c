/* ================================================================
 *  NEXA OS - Display Server (Window Manager v3)
 *  True display server architecture using Unix sockets and IPC
 *  Shared Memory for client application compositing.
 * ================================================================ */
#include <errno.h>
#include <fcntl.h>
#include <linux/fb.h>
#include <linux/kd.h>
#include <math.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/time.h>
#include <time.h>
#include <termios.h>
#include <sys/stat.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/shm.h>
#include <dirent.h>

#include "libnexa.h" /* For colors and msg defs */

/* ================================================================
 *  FRAMEBUFFER
 * ================================================================ */
typedef struct {
    int fd; uint8_t *mem,*back; size_t size;
    int w,h,stride,bpp;
    struct fb_var_screeninfo var;
    struct fb_fix_screeninfo fix;
} FB;

static int fb_open(FB *fb){
    memset(fb,0,sizeof(*fb));
    fb->fd=open("/dev/fb0",O_RDWR);
    if(fb->fd<0)return -1;
    ioctl(fb->fd,FBIOGET_FSCREENINFO,&fb->fix);
    ioctl(fb->fd,FBIOGET_VSCREENINFO,&fb->var);
    fb->w=fb->var.xres;fb->h=fb->var.yres;
    fb->stride=fb->fix.line_length;fb->bpp=fb->var.bits_per_pixel;
    fb->size=fb->fix.smem_len;
    fb->mem=mmap(NULL,fb->size,PROT_READ|PROT_WRITE,MAP_SHARED,fb->fd,0);
    if(fb->mem==MAP_FAILED){close(fb->fd);return -1;}
    fb->back=malloc(fb->size);
    if(!fb->back){munmap(fb->mem,fb->size);close(fb->fd);return -1;}
    return 0;
}
static void fb_close(FB *fb){
    if(fb->back)free(fb->back);
    if(fb->mem&&fb->mem!=MAP_FAILED)munmap(fb->mem,fb->size);
    if(fb->fd>=0)close(fb->fd);
}
static void fb_flip(FB *fb){
    memcpy(fb->mem,fb->back,fb->size);
    fb->var.yoffset=0;fb->var.xoffset=0;
    ioctl(fb->fd,FBIOPAN_DISPLAY,&fb->var);
}
static inline void px(FB *fb,int x,int y,Color c){
    if(x<0||y<0||x>=fb->w||y>=fb->h)return;
    uint8_t *p=fb->back+y*fb->stride+x*(fb->bpp/8);
    uint32_t v=((uint32_t)c.r>>(8-fb->var.red.length))<<fb->var.red.offset
              |((uint32_t)c.g>>(8-fb->var.green.length))<<fb->var.green.offset
              |((uint32_t)c.b>>(8-fb->var.blue.length))<<fb->var.blue.offset;
    if(fb->var.transp.length)v|=((uint32_t)0xff>>(8-fb->var.transp.length))<<fb->var.transp.offset;
    if(fb->bpp==32)*(uint32_t*)p=v;else if(fb->bpp==16)*(uint16_t*)p=(uint16_t)v;
}
static void fillr(FB *fb,int x,int y,int w,int h,Color c){
    for(int j=y;j<y+h;j++)for(int i=x;i<x+w;i++)px(fb,i,j,c);
}
static void drawr(FB *fb,int x,int y,int w,int h,Color c){
    for(int i=x;i<x+w;i++){px(fb,i,y,c);px(fb,i,y+h-1,c);}
    for(int j=y;j<y+h;j++){px(fb,x,j,c);px(fb,x+w-1,j,c);}
}
static void hline(FB *fb,int x,int y,int w,Color c){for(int i=x;i<x+w;i++)px(fb,i,y,c);}
static void fcircle(FB *fb,int cx,int cy,int r,Color c){
    for(int j=-r;j<=r;j++)for(int i=-r;i<=r;i++)if(i*i+j*j<=r*r)px(fb,cx+i,cy+j,c);
}
static void dcircle(FB *fb,int cx,int cy,int r,Color c){
    for(int j=-r;j<=r;j++)for(int i=-r;i<=r;i++){int d=i*i+j*j;if(d>=(r-1)*(r-1)&&d<=r*r)px(fb,cx+i,cy+j,c);}
}

static void dchar(FB *fb,int x,int y,char ch,int s,Color c){
    if(ch>='a'&&ch<='z')ch-=32;
    if((unsigned char)ch>=128)return;
    const uint8_t *g=nx_font[(int)ch];
    for(int r=0;r<7;r++)for(int co=0;co<5;co++)
        if(g[r]&(1<<(4-co)))fillr(fb,x+co*s,y+r*s,s,s,c);
}
static void dtext(FB *fb,int x,int y,const char *t,int s,Color c){
    for(int i=0;t[i];i++)dchar(fb,x+i*6*s,y,t[i],s,c);
}
static int tw(const char *t,int s){return(int)strlen(t)*6*s;}

static long ms_now(void){
    struct timeval tv; gettimeofday(&tv,NULL);
    return tv.tv_sec*1000L+tv.tv_usec/1000L;
}

/* ================================================================
 *  MOUSE
 * ================================================================ */
typedef struct { int x,y,btn; } Mouse;
static int mouse_fd=-1;
static void mouse_open(void){
    mouse_fd=open("/dev/input/mice",O_RDONLY|O_NONBLOCK);
    if(mouse_fd<0)mouse_fd=open("/dev/input/mouse0",O_RDONLY|O_NONBLOCK);
}
static void mouse_read(Mouse *m,int sw,int sh){
    unsigned char buf[3];
    while(read(mouse_fd,buf,3)==3){
        m->btn=buf[0]&1; m->x+=(int)(signed char)buf[1]; m->y-=(int)(signed char)buf[2];
        if(m->x<0)m->x=0; if(m->x>=sw)m->x=sw-1;
        if(m->y<0)m->y=0; if(m->y>=sh)m->y=sh-1;
    }
}
static void draw_cursor(FB *fb,int cx,int cy){
    static const char *s[]={
        "X","XX","X.X","X..X","X...X","X....X","X.....X",
        "X......X","X.......X","X........X","X.....XXXXX",
        "X..X..X","X.X.X..X","XX..X..X","X....X..X",
        ".....X..X","......XX",NULL};
    for(int r=0;s[r];r++)for(int c2=0;s[r][c2];c2++){
        if(s[r][c2]=='X')px(fb,cx+c2,cy+r,NX_WHITE);
        else if(s[r][c2]=='.')px(fb,cx+c2,cy+r,NX_BLACK);
    }
}

/* ================================================================
 *  CLIENT PROTOCOL & WINDOW MANAGER
 * ================================================================ */
#define MAX_CLIENTS 16
#define TB_H 26
#define CBTN 14
#define TASKBAR_H 34
#define RESIZE_EDGE 6
#define MIN_WIN_W 160
#define MIN_WIN_H 120

typedef struct {
    int fd;
    int active;
    int x, y, w, h;
    int shm_id;
    uint32_t *pixels;
    char title[32];
    int app_id;
    int visible;
} Client;

static Client clients[MAX_CLIENTS];
static int wo[MAX_CLIENTS]; /* draw order */
static int wc=0, wf=-1; /* count and focus */

static int svr_sock = -1;

static void svr_init(void){
    unlink(NEXA_SOCKET_PATH);
    svr_sock = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, NEXA_SOCKET_PATH, sizeof(addr.sun_path)-1);
    bind(svr_sock, (struct sockaddr*)&addr, sizeof(addr));
    listen(svr_sock, MAX_CLIENTS);
    memset(clients, 0, sizeof(clients));
}

static void client_close(int idx){
    Client *c = &clients[idx];
    if(c->fd > 0) close(c->fd);
    if(c->pixels && c->pixels != (void*)-1) shmdt(c->pixels);
    if(c->shm_id > 0) shmctl(c->shm_id, IPC_RMID, NULL);
    c->active = 0;
    
    /* remove from draw order */
    int p=-1;
    for(int i=0;i<wc;i++) if(wo[i]==idx){p=i;break;}
    if(p>=0){
        for(int i=p;i<wc-1;i++) wo[i] = wo[i+1];
        wc--;
    }
    if(wf == idx) wf = (wc > 0) ? wo[wc-1] : -1;
}

static void wfront(int idx){
    int p=-1;
    for(int i=0;i<wc;i++)if(wo[i]==idx){p=i;break;}
    if(p<0)return;
    int v=wo[p];
    for(int i=p;i<wc-1;i++)wo[i]=wo[i+1];
    wo[wc-1]=v; wf=idx;
}

static int hit(int px,int py,int rx,int ry,int rw,int rh){
    return px>=rx&&px<rx+rw&&py>=ry&&py<ry+rh;
}

static int resize_edge(int mx,int my,Client *w){
    int e=RESIZE_EDGE, t=0,b=0,l=0,r=0;
    if(my>=w->y&&my<w->y+e)t=1;
    if(my>=w->y+w->h-e&&my<w->y+w->h)b=1;
    if(mx>=w->x&&mx<w->x+e)l=1;
    if(mx>=w->x+w->w-e&&mx<w->x+w->w)r=1;
    if(t&&l)return 5; if(t&&r)return 6;
    if(b&&l)return 7; if(b&&r)return 8;
    if(t)return 1; if(b)return 2;
    if(l)return 3; if(r)return 4;
    return 0;
}

/* ================================================================
 *  DESKTOP ICONS
 * ================================================================ */
#define MAX_ICON 12
#define ICON_W 72
#define ICON_H 72
#define ICON_GRID_X 80
#define ICON_GRID_Y 82

typedef struct { int x,y,app_id; char label[24]; int selected; char cmd[64]; } DIcon;
static DIcon icons[MAX_ICON];
static int icnt=0;

static void icon_add(int gx,int gy,int app,const char *lbl, const char *cmd){
    if(icnt>=MAX_ICON)return;
    icons[icnt].x=10+gx*ICON_GRID_X;
    icons[icnt].y=10+gy*ICON_GRID_Y;
    icons[icnt].app_id=app;
    icons[icnt].selected=0;
    strncpy(icons[icnt].label,lbl,23);
    strncpy(icons[icnt].cmd,cmd,63);
    icnt++;
}

static void load_desktop_icons(void){
    DIR *d = opendir("/disk/Desktop");
    if(!d) return;
    struct dirent *e;
    int col=0, row=0;
    while((e=readdir(d))){
        if(strstr(e->d_name, ".desktop")){
            char path[512]; snprintf(path, sizeof(path), "/disk/Desktop/%s", e->d_name);
            FILE *f = fopen(path, "r");
            if(f){
                char name[32]="App", cmd[64]="", line[128];
                int app_id=APP_ABOUT;
                while(fgets(line, sizeof(line), f)){
                    if(strncmp(line, "Name=", 5)==0){ strncpy(name, line+5, 31); name[strcspn(name, "\n")]=0; }
                    if(strncmp(line, "Exec=", 5)==0){ strncpy(cmd, line+5, 63); cmd[strcspn(cmd, "\n")]=0; }
                    if(strncmp(line, "Icon=", 5)==0){ app_id = atoi(line+5); }
                }
                fclose(f);
                icon_add(col, row, app_id, name, cmd);
                col++; if(col>3){ col=0; row++; }
            }
        }
    }
    closedir(d);
}

static void draw_icon_glyph(FB *fb,int cx,int cy,int app){
    switch(app){
    case APP_TERM:
        fillr(fb,cx,cy,44,34,NX_BG_MID);drawr(fb,cx,cy,44,34,NX_BG_LIGHTER);
        dtext(fb,cx+8,cy+10,">_",2,NX_TXT_GREEN);break;
    case APP_CLOCK:
        fcircle(fb,cx+22,cy+17,17,NX_BLUE);dcircle(fb,cx+22,cy+17,17,NX_WHITE);
        for(int k=0;k<9;k++)px(fb,cx+22,cy+17-k,NX_WHITE);
        for(int k=0;k<12;k++)px(fb,cx+22+k,cy+17,NX_WHITE);
        fcircle(fb,cx+22,cy+17,2,NX_WHITE);break;
    case APP_CALC:
        fillr(fb,cx+2,cy,40,34,NX_BG_MID);drawr(fb,cx+2,cy,40,34,NX_BG_LIGHTER);
        dtext(fb,cx+6,cy+3,"123",1,NX_TXT_GREEN);
        fillr(fb,cx+6,cy+14,8,7,NX_BLUE);fillr(fb,cx+16,cy+14,8,7,NX_BLUE);
        fillr(fb,cx+26,cy+14,8,7,NX_BLUE);fillr(fb,cx+6,cy+23,8,7,NX_BLUE);
        fillr(fb,cx+16,cy+23,8,7,NX_BLUE);fillr(fb,cx+26,cy+23,8,7,NX_BLUE);break;
    case APP_FILES:
        fillr(fb,cx+4,cy+4,14,6,NX_AMBER);
        fillr(fb,cx,cy+10,40,22,NX_AMBER);
        drawr(fb,cx,cy+10,40,22,C(200,130,0));break;
    case APP_SYSMON:
        fillr(fb,cx,cy,44,34,NX_BG_MID);drawr(fb,cx,cy,44,34,NX_BG_LIGHTER);
        fillr(fb,cx+4,cy+18,6,14,NX_TXT_GREEN);fillr(fb,cx+12,cy+10,6,22,NX_BLUE);
        fillr(fb,cx+20,cy+14,6,18,NX_TEAL);fillr(fb,cx+28,cy+6,6,26,NX_PURPLE);break;
    case APP_EDITOR:
        fillr(fb,cx+2,cy,40,34,NX_BG_MID);drawr(fb,cx+2,cy,40,34,NX_BG_LIGHTER);
        dtext(fb,cx+6,cy+4,"ABC",1,NX_TXT);dtext(fb,cx+6,cy+14,"DEF",1,NX_TXT_DIM);
        dtext(fb,cx+6,cy+22,"___",1,NX_TXT_GREEN);break;
    case APP_TODO:
        fillr(fb,cx+2,cy,40,34,NX_BG_MID);drawr(fb,cx+2,cy,40,34,NX_BG_LIGHTER);
        drawr(fb,cx+6,cy+5,8,8,NX_TXT_GREEN);fillr(fb,cx+8,cy+7,4,4,NX_TXT_GREEN);
        hline(fb,cx+16,cy+9,18,NX_TXT);drawr(fb,cx+6,cy+16,8,8,NX_TXT_DIM);
        hline(fb,cx+16,cy+20,18,NX_TXT_DIM);break;
    case APP_ABOUT:
        fcircle(fb,cx+22,cy+17,17,NX_BLUE_DP);dcircle(fb,cx+22,cy+17,17,NX_BLUE);
        dtext(fb,cx+16,cy+10,"N",2,NX_WHITE);break;
    }
}

static void draw_icons(FB *fb){
    for(int i=0;i<icnt;i++){
        int ix=icons[i].x,iy=icons[i].y;
        if(icons[i].selected){
            fillr(fb,ix,iy,ICON_W,ICON_H,C(59,130,246));
            for(int j=iy;j<iy+ICON_H;j++)for(int k=ix;k<ix+ICON_W;k++){
                if((k+j)%2==0)px(fb,k,j,C(30,60,120));
            }
        }
        draw_icon_glyph(fb,ix+14,iy+6,icons[i].app_id);
        int t2=tw(icons[i].label,1);
        dtext(fb,ix+(ICON_W-t2)/2,iy+48,icons[i].label,1,NX_TXT);
    }
}

/* ================================================================
 *  DRAW WINDOW
 * ================================================================ */
static void draw_win(FB *fb,Client *w,int foc){
    if(!w->visible)return;
    int wx=w->x, wy=w->y, ww=w->w, wh=w->h;
    
    fillr(fb,wx+5,wy+5,ww+4,wh+TB_H+4,C(5,10,20)); /* Shadow */
    fillr(fb,wx,wy,ww+4,wh+TB_H+4,NX_BG_MID); /* Body */
    drawr(fb,wx,wy,ww+4,wh+TB_H+4,foc?NX_BLUE:NX_BG_LIGHTER); /* Border */
    if(foc)drawr(fb,wx+1,wy+1,ww+2,wh+TB_H+2,NX_BLUE_DK);

    for(int j=0;j<TB_H;j++){
        Color tc=foc?C((uint8_t)(37-(j>37?37:j)/3),(uint8_t)(99-(j>99?99:j)),(uint8_t)(235-(j>117?117:j)*2)):C((uint8_t)(51+j/2),(uint8_t)(65+j/2),(uint8_t)(85+j/2));
        hline(fb,wx+2,wy+2+j,ww,tc);
    }
    dtext(fb,wx+10,wy+7,w->title,2,NX_WHITE);
    
    int cbx=wx+ww+4-CBTN-8,cby=wy+6;
    fillr(fb,cbx,cby,CBTN,CBTN,NX_TXT_RED);dtext(fb,cbx+2,cby+0,"X",2,NX_WHITE);

    for(int k=0;k<3;k++){
        int gx=wx+ww+4-6-k*4,gy=wy+wh+TB_H+4-6-k*4;
        fillr(fb,gx,gy,3,3,foc?NX_BLUE:NX_BG_LIGHTER);
    }

    /* Copy client shared memory pixels to framebuffer */
    if(w->pixels && w->pixels != (void*)-1){
        int cx = wx+2, cy = wy+TB_H+2;
        for(int j=0; j<w->h; j++){
            for(int i=0; i<w->w; i++){
                uint32_t cv = w->pixels[j * w->w + i];
                Color pxc = C((cv>>16)&0xff, (cv>>8)&0xff, cv&0xff);
                px(fb, cx+i, cy+j, pxc);
            }
        }
    }
}

/* ================================================================
 *  TASKBAR & BACKGROUND
 * ================================================================ */
static void draw_taskbar(FB *fb){
    int ty=fb->h-TASKBAR_H;
    for(int j=0;j<TASKBAR_H;j++)hline(fb,0,ty+j,fb->w,C((uint8_t)(20+j/4),(uint8_t)(30+j/4),(uint8_t)(48+j/4)));
    hline(fb,0,ty,fb->w,NX_BLUE);
    fillr(fb,4,ty+3,72,TASKBAR_H-6,NX_BLUE_DK);drawr(fb,4,ty+3,72,TASKBAR_H-6,NX_BLUE);
    dtext(fb,12,ty+9,"NEXA",2,NX_WHITE);
    int bx=82;
    for(int i=0;i<wc;i++){
        int idx = wo[i]; Client *c = &clients[idx];
        Color bg=(wf==idx)?NX_BG_LIGHT:NX_BG_MID;
        Color brd=(wf==idx)?NX_BLUE:NX_BG_LIGHTER;
        fillr(fb,bx,ty+3,88,TASKBAR_H-6,bg);
        drawr(fb,bx,ty+3,88,TASKBAR_H-6,brd);
        if(c->visible)fillr(fb,bx+2,ty+TASKBAR_H-8,84,2,NX_BLUE);
        char lbl[12];strncpy(lbl,c->title,8);lbl[8]=0;
        dtext(fb,bx+4,ty+9,lbl,2,c->visible?NX_TXT:NX_TXT_DIM);
        bx+=94;
    }
    time_t now=time(NULL);struct tm *t=localtime(&now);
    if(t){
        char ts[16];snprintf(ts,sizeof(ts),"%02d:%02d",t->tm_hour,t->tm_min);
        dtext(fb,fb->w-tw(ts,2)-10,ty+9,ts,2,NX_TXT);
    }
}

static void draw_bg(FB *fb){
    int tbh=TASKBAR_H;
    for(int y=0;y<fb->h-tbh;y++){
        float t2=(float)y/(float)(fb->h-tbh);
        uint8_t r=(uint8_t)(12+t2*18),g=(uint8_t)(18+t2*25),b=(uint8_t)(38+t2*40);
        for(int x=0;x<fb->w;x++)px(fb,x,y,C(r,g,b));
    }
    for(int y=0;y<fb->h-tbh;y+=40) for(int x=0;x<fb->w;x++)px(fb,x,y,C(18,26,48));
    for(int x=0;x<fb->w;x+=40) for(int y=0;y<fb->h-tbh;y++)px(fb,x,y,C(18,26,48));
}

/* ================================================================
 *  MAIN
 * ================================================================ */
int main(void){
    FB fb;
    if(fb_open(&fb)!=0)return 1;
    int tty_fd=open("/dev/tty0",O_RDWR);
    if(tty_fd>=0)ioctl(tty_fd,KDSETMODE,KD_GRAPHICS);
    struct termios ot,nt;
    tcgetattr(STDIN_FILENO,&ot);nt=ot;
    nt.c_lflag&=~(ICANON|ECHO);
    tcsetattr(STDIN_FILENO,TCSANOW,&nt);
    mouse_open();
    svr_init();

    Mouse ms;ms.x=fb.w/2;ms.y=fb.h/2;ms.btn=0;

    load_desktop_icons();

    int dragging_win=-1, drag_ox=0, drag_oy=0;
    int dragging_icon=-1, idrag_ox=0, idrag_oy=0;
    int resizing_win=-1, resize_dir=0, rz_ox=0, rz_oy=0, rz_ow=0, rz_oh=0;
    int prev_btn=0;
    long last_click_time=0;
    int last_click_icon=-1;
    int running=1;

    struct pollfd pfd[MAX_CLIENTS + 3];

    while(running){
        draw_bg(&fb);
        draw_icons(&fb);
        for(int i=0;i<wc;i++){int idx=wo[i];draw_win(&fb,&clients[idx],idx==wf);}
        draw_taskbar(&fb);
        draw_cursor(&fb,ms.x,ms.y);
        fb_flip(&fb);

        int np = 0;
        pfd[np].fd = STDIN_FILENO; pfd[np].events = POLLIN; np++;
        if(mouse_fd>=0){ pfd[np].fd = mouse_fd; pfd[np].events = POLLIN; np++; }
        pfd[np].fd = svr_sock; pfd[np].events = POLLIN; np++;

        for(int i=0; i<MAX_CLIENTS; i++){
            if(clients[i].active){
                pfd[np].fd = clients[i].fd; pfd[np].events = POLLIN; np++;
            }
        }

        poll(pfd, np, 20);

        /* Accept new clients */
        if(pfd[2].revents & POLLIN){
            int cfd = accept(svr_sock, NULL, NULL);
            if(cfd >= 0){
                int c_idx = -1;
                for(int i=0; i<MAX_CLIENTS; i++) if(!clients[i].active){ c_idx = i; break; }
                if(c_idx >= 0){
                    clients[c_idx].active = 1;
                    clients[c_idx].fd = cfd;
                    clients[c_idx].x = 100 + (wc*20);
                    clients[c_idx].y = 80 + (wc*20);
                    clients[c_idx].visible = 1;
                    wo[wc++] = c_idx;
                    wf = c_idx;
                } else close(cfd);
            }
        }

        /* Read client messages */
        int p_idx = 3;
        for(int i=0; i<MAX_CLIENTS; i++){
            if(clients[i].active){
                if(pfd[p_idx].revents & POLLIN){
                    NxMsg m;
                    if(read(clients[i].fd, &m, sizeof(m)) == sizeof(m)){
                        if(m.type == NX_MSG_INIT){
                            strncpy(clients[i].title, m.title, 31);
                            clients[i].w = m.w; clients[i].h = m.h;
                            clients[i].app_id = m.app_id;
                            
                            clients[i].shm_id = shmget(IPC_PRIVATE, m.w * m.h * 4, IPC_CREAT | 0666);
                            clients[i].pixels = shmat(clients[i].shm_id, NULL, 0);

                            m.type = NX_MSG_INIT_ACK;
                            m.shm_id = clients[i].shm_id;
                            write(clients[i].fd, &m, sizeof(m));
                        }
                    } else {
                        client_close(i);
                    }
                }
                p_idx++;
            }
        }

        if(mouse_fd>=0) mouse_read(&ms,fb.w,fb.h);

        int click=(ms.btn&&!prev_btn);
        int release=(!ms.btn&&prev_btn);

        if(click){
            long now_ms=ms_now();
            int done=0;

            for(int i=wc-1;i>=0&&!done;i--){
                int idx=wo[i]; Client *c=&clients[idx];
                if(!c->visible)continue;
                int cbx=c->x+c->w+4-CBTN-8,cby=c->y+6;
                if(hit(ms.x,ms.y,cbx,cby,CBTN,CBTN)){
                    NxMsg m; memset(&m, 0, sizeof(m));
                    m.type = NX_MSG_EVENT; m.ev_type = NX_EV_QUIT;
                    write(c->fd, &m, sizeof(m));
                    done=1;break;
                }
                if(hit(ms.x,ms.y,c->x,c->y,c->w+4,c->h+TB_H+4)){
                    int re=resize_edge(ms.x,ms.y,c);
                    if(re>0){
                        resizing_win=idx;resize_dir=re;
                        rz_ox=ms.x;rz_oy=ms.y;rz_ow=c->w;rz_oh=c->h;
                        wfront(idx);done=1;break;
                    }
                }
                if(hit(ms.x,ms.y,c->x,c->y,c->w+4,TB_H)){
                    dragging_win=idx;drag_ox=ms.x-c->x;drag_oy=ms.y-c->y;
                    wfront(idx);done=1;break;
                }
                if(hit(ms.x,ms.y,c->x,c->y,c->w+4,c->h+TB_H+4)){
                    wfront(idx);
                    int lx=ms.x-(c->x+2),ly=ms.y-(c->y+TB_H+2);
                    NxMsg m; memset(&m, 0, sizeof(m));
                    m.type = NX_MSG_EVENT; m.ev_type = NX_EV_MOUSE;
                    m.mx = lx; m.my = ly; m.mbtn = ms.btn;
                    write(c->fd, &m, sizeof(m));
                    done=1;break;
                }
            }

            if(!done){
                int ty=fb.h-TASKBAR_H;
                if(ms.y>=ty){
                    int bx=82;
                    for(int i=0;i<wc;i++){
                        int idx = wo[i];
                        if(hit(ms.x,ms.y,bx,ty+3,88,TASKBAR_H-6)){
                            clients[idx].visible=!clients[idx].visible;
                            if(clients[idx].visible)wfront(idx);
                            done=1;break;
                        }
                        bx+=94;
                    }
                }
            }

            if(!done){
                int clicked_icon=-1;
                for(int i=0;i<icnt;i++) if(hit(ms.x,ms.y,icons[i].x,icons[i].y,ICON_W,ICON_H)){clicked_icon=i;break;}
                for(int i=0;i<icnt;i++)icons[i].selected=0;

                if(clicked_icon>=0){
                    icons[clicked_icon].selected=1;
                    if(clicked_icon==last_click_icon&&(now_ms-last_click_time)<400){
                        system(icons[clicked_icon].cmd);
                        last_click_icon=-1;last_click_time=0;
                    }else{
                        last_click_icon=clicked_icon;last_click_time=now_ms;
                        dragging_icon=clicked_icon;
                        idrag_ox=ms.x-icons[clicked_icon].x; idrag_oy=ms.y-icons[clicked_icon].y;
                    }
                }else{
                    last_click_icon=-1;last_click_time=0;
                }
            }
        }

        if(release){
            if(resizing_win >= 0){
                /* Finalize resize and send event to client */
                Client *c = &clients[resizing_win];
                int new_shm_id = shmget(IPC_PRIVATE, c->w * c->h * 4, IPC_CREAT | 0666);
                shmdt(c->pixels);
                shmctl(c->shm_id, IPC_RMID, NULL);
                c->shm_id = new_shm_id;
                c->pixels = shmat(c->shm_id, NULL, 0);

                NxMsg m; memset(&m, 0, sizeof(m));
                m.type = NX_MSG_EVENT; m.ev_type = NX_EV_RESIZE;
                m.w = c->w; m.h = c->h; m.shm_id = c->shm_id;
                write(c->fd, &m, sizeof(m));
            }

            dragging_win=-1; dragging_icon=-1; resizing_win=-1;
        }

        if(dragging_win>=0&&ms.btn){
            clients[dragging_win].x=ms.x-drag_ox;
            clients[dragging_win].y=ms.y-drag_oy;
        }

        if(dragging_icon>=0&&ms.btn){
            icons[dragging_icon].x=ms.x-idrag_ox;
            icons[dragging_icon].y=ms.y-idrag_oy;
        }

        if(resizing_win>=0&&ms.btn){
            Client *w=&clients[resizing_win];
            int dx=ms.x-rz_ox,dy=ms.y-rz_oy;
            switch(resize_dir){
                case 2: w->h=rz_oh+dy;break;
                case 4: w->w=rz_ow+dx;break;
                case 8: w->w=rz_ow+dx;w->h=rz_oh+dy;break;
                case 1: w->y=w->y+dy;w->h=rz_oh-dy;break;
                case 3: w->x=w->x+dx;w->w=rz_ow-dx;break;
                case 5: w->x=w->x+dx;w->w=rz_ow-dx;w->y=w->y+dy;w->h=rz_oh-dy;break;
                case 6: w->w=rz_ow+dx;w->y=w->y+dy;w->h=rz_oh-dy;break;
                case 7: w->x=w->x+dx;w->w=rz_ow-dx;w->h=rz_oh+dy;break;
            }
            if(w->w<MIN_WIN_W)w->w=MIN_WIN_W;
            if(w->h<MIN_WIN_H)w->h=MIN_WIN_H;
        }

        prev_btn=ms.btn;

        /* Keyboard */
        if(pfd[0].revents&POLLIN){
            char c;
            if(read(STDIN_FILENO,&c,1)==1){
                if(wf>=0 && clients[wf].visible){
                    NxMsg m; memset(&m, 0, sizeof(m));
                    m.type = NX_MSG_EVENT; m.ev_type = NX_EV_KEY;
                    m.key = c;
                    write(clients[wf].fd, &m, sizeof(m));
                }
            }
        }
    }

    tcsetattr(STDIN_FILENO,TCSANOW,&ot);
    fb_close(&fb);
    if(tty_fd>=0){ioctl(tty_fd,KDSETMODE,KD_TEXT);close(tty_fd);}
    return 0;
}
