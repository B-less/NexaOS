/* ================================================================
 *  LIBNEXA - Nexa OS Client GUI Library
 *  Provides a simple API for building standalone graphical
 *  applications that talk to the Nexa Display Server.
 * ================================================================ */
#ifndef LIBNEXA_H
#define LIBNEXA_H

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/shm.h>
#include <poll.h>
#include <math.h>

#define NEXA_SOCKET_PATH "/tmp/nexa_wm.sock"

/* Application IDs for Taskbar integration */
enum { APP_TERM=0, APP_CLOCK, APP_CALC, APP_FILES, APP_SYSMON,
       APP_EDITOR, APP_TODO, APP_ABOUT, APP_COUNT };

/* Colors */
typedef struct { uint8_t r, g, b; } Color;
static inline Color C(uint8_t r, uint8_t g, uint8_t b) { Color c={r,g,b}; return c; }

#define NX_BG_DARK    C(15,23,42)
#define NX_BG_MID     C(30,41,59)
#define NX_BG_LIGHT   C(51,65,85)
#define NX_BG_LIGHTER C(71,85,105)
#define NX_TXT        C(226,232,240)
#define NX_TXT_DIM    C(148,163,184)
#define NX_TXT_GREEN  C(52,211,153)
#define NX_TXT_RED    C(239,68,68)
#define NX_BLUE       C(59,130,246)
#define NX_BLUE_DK    C(37,99,235)
#define NX_BLUE_DP    C(29,78,216)
#define NX_PURPLE     C(139,92,246)
#define NX_AMBER      C(245,158,11)
#define NX_TEAL       C(20,184,166)
#define NX_ROSE       C(244,63,94)
#define NX_BLACK      C(0,0,0)
#define NX_WHITE      C(255,255,255)

/* Messages */
enum { NX_MSG_INIT=1, NX_MSG_INIT_ACK, NX_MSG_FLIP, NX_MSG_EVENT };
enum { NX_EV_NONE=0, NX_EV_MOUSE, NX_EV_KEY, NX_EV_RESIZE, NX_EV_QUIT };

typedef struct {
    int type;
    int shm_id;
    int w, h;
    int app_id;
    int ev_type;
    int mx, my, mbtn;
    int key;
    char title[32];
} NxMsg;

typedef struct {
    int type;
    int mx, my, mbtn;
    int key;
    int w, h;
} NxEvent;

/* Context */
static int nx_sock = -1;
static int nx_shm_id = -1;
static uint32_t *nx_pixels = NULL;
static int nx_w = 0;
static int nx_h = 0;

/* ================================================================
 *  FONT (5x7)
 * ================================================================ */
static const uint8_t nx_font[128][7]={
    [' ']={0},['\0']={0},
    ['!']={4,4,4,4,0,0,4},['"']={10,10,10,0,0,0,0},
    ['#']={10,10,31,10,31,10,10},['$']={4,15,16,14,1,30,4},
    ['%']={25,26,2,4,8,11,19},['&']={12,18,20,8,21,18,13},
    ['\'']={4,4,8,0,0,0,0},['(']={2,4,8,8,8,4,2},
    [')']={8,4,2,2,2,4,8},['*']={0,4,21,14,21,4,0},
    ['+']={0,4,4,31,4,4,0},[',']={0,0,0,0,0,4,8},
    ['-']={0,0,0,31,0,0,0},['.']={0,0,0,0,0,12,12},
    ['/']={1,1,2,4,8,16,16},
    ['0']={14,17,19,21,25,17,14},['1']={4,12,4,4,4,4,14},
    ['2']={14,17,1,2,4,8,31},['3']={31,2,4,2,1,17,14},
    ['4']={2,6,10,18,31,2,2},['5']={31,16,30,1,1,17,14},
    ['6']={14,16,30,17,17,17,14},['7']={31,1,2,4,8,8,8},
    ['8']={14,17,17,14,17,17,14},['9']={14,17,17,15,1,1,14},
    [':']={0,4,4,0,4,4,0},[';']={0,4,4,0,4,8,0},
    ['<']={2,4,8,16,8,4,2},['=']={0,0,31,0,31,0,0},
    ['>']={8,4,2,1,2,4,8},['?']={14,17,1,2,4,0,4},
    ['@']={14,17,21,21,13,16,14},
    ['A']={14,17,17,31,17,17,17},['B']={30,17,17,30,17,17,30},
    ['C']={14,17,16,16,16,17,14},['D']={30,17,17,17,17,17,30},
    ['E']={31,16,16,30,16,16,31},['F']={31,16,16,30,16,16,16},
    ['G']={14,17,16,23,17,17,14},['H']={17,17,17,31,17,17,17},
    ['I']={14,4,4,4,4,4,14},['J']={7,2,2,2,18,18,12},
    ['K']={17,18,20,24,20,18,17},['L']={16,16,16,16,16,16,31},
    ['M']={17,27,21,21,17,17,17},['N']={17,17,25,21,19,17,17},
    ['O']={14,17,17,17,17,17,14},['P']={30,17,17,30,16,16,16},
    ['Q']={14,17,17,17,21,18,13},['R']={30,17,17,30,20,18,17},
    ['S']={15,16,16,14,1,1,30},['T']={31,4,4,4,4,4,4},
    ['U']={17,17,17,17,17,17,14},['V']={17,17,17,17,17,10,4},
    ['W']={17,17,17,21,21,21,10},['X']={17,17,10,4,10,17,17},
    ['Y']={17,17,10,4,4,4,4},['Z']={31,1,2,4,8,16,31},
    ['[']={14,8,8,8,8,8,14},['\\']={16,16,8,4,2,1,1},
    [']']={14,2,2,2,2,2,14},['^']={4,10,17,0,0,0,0},
    ['_']={0,0,0,0,0,0,31},['`']={8,4,2,0,0,0,0},
    ['{']={2,4,4,8,4,4,2},['|']={4,4,4,4,4,4,4},
    ['}']={8,4,4,2,4,4,8},['~']={0,0,0,10,21,0,0},
};

static void nx_detach_shm(void){
    if(nx_pixels && nx_pixels != (void*)-1){
        shmdt(nx_pixels);
        nx_pixels = NULL;
    }
}

/* API */
static int nexa_init(const char *title, int w, int h, int app_id){
    nx_sock = socket(AF_UNIX, SOCK_STREAM, 0);
    if(nx_sock < 0) return -1;
    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, NEXA_SOCKET_PATH, sizeof(addr.sun_path)-1);
    if(connect(nx_sock, (struct sockaddr*)&addr, sizeof(addr)) < 0) return -1;
    
    NxMsg m; memset(&m, 0, sizeof(m));
    m.type = NX_MSG_INIT;
    m.w = w; m.h = h; m.app_id = app_id;
    strncpy(m.title, title, 31);
    if(write(nx_sock, &m, sizeof(m)) != sizeof(m)) return -1;
    
    if(read(nx_sock, &m, sizeof(m)) != sizeof(m)) return -1;
    if(m.type != NX_MSG_INIT_ACK) return -1;
    
    nx_w = m.w; nx_h = m.h; nx_shm_id = m.shm_id;
    nx_pixels = shmat(nx_shm_id, NULL, 0);
    if(nx_pixels == (void*)-1) return -1;
    return 0;
}

static void nexa_flip(void){
    NxMsg m; memset(&m, 0, sizeof(m));
    m.type = NX_MSG_FLIP;
    write(nx_sock, &m, sizeof(m));
}

static int nexa_poll_event(NxEvent *ev){
    struct pollfd pfd;
    pfd.fd = nx_sock; pfd.events = POLLIN;
    if(poll(&pfd, 1, 10) > 0){
        NxMsg m;
        if(read(nx_sock, &m, sizeof(m)) == sizeof(m)){
            if(m.type == NX_MSG_EVENT){
                ev->type = m.ev_type;
                ev->mx = m.mx; ev->my = m.my; ev->mbtn = m.mbtn;
                ev->key = m.key; ev->w = m.w; ev->h = m.h;
                if(ev->type == NX_EV_RESIZE){
                    nx_detach_shm();
                    nx_w = m.w; nx_h = m.h; nx_shm_id = m.shm_id;
                    nx_pixels = shmat(nx_shm_id, NULL, 0);
                }
                return 1;
            }
        } else {
            /* Connection lost */
            ev->type = NX_EV_QUIT; return 1;
        }
    }
    return 0;
}

/* Graphics Primitives */
static inline void nx_px(int x, int y, Color c){
    if(x<0||y<0||x>=nx_w||y>=nx_h) return;
    uint32_t v = (255<<24) | (c.r<<16) | (c.g<<8) | c.b;
    nx_pixels[y*nx_w + x] = v;
}

static void nx_fill_rect(int x, int y, int w, int h, Color c){
    for(int j=y; j<y+h; j++) for(int i=x; i<x+w; i++) nx_px(i,j,c);
}

static void nx_draw_rect(int x, int y, int w, int h, Color c){
    for(int i=x; i<x+w; i++){ nx_px(i,y,c); nx_px(i,y+h-1,c); }
    for(int j=y; j<y+h; j++){ nx_px(x,j,c); nx_px(x+w-1,j,c); }
}

static void nx_draw_hline(int x, int y, int w, Color c){
    for(int i=x; i<x+w; i++) nx_px(i,y,c);
}

static void nx_fill_circle(int cx, int cy, int r, Color c){
    for(int j=-r; j<=r; j++) for(int i=-r; i<=r; i++)
        if(i*i+j*j <= r*r) nx_px(cx+i, cy+j, c);
}

static void nx_draw_circle(int cx, int cy, int r, Color c){
    for(int j=-r; j<=r; j++) for(int i=-r; i<=r; i++){
        int d = i*i+j*j;
        if(d>=(r-1)*(r-1) && d<=r*r) nx_px(cx+i, cy+j, c);
    }
}

static void nx_draw_char(int x, int y, char ch, int s, Color c){
    if(ch>='a'&&ch<='z') ch-=32;
    if((unsigned char)ch>=128) return;
    const uint8_t *g = nx_font[(int)ch];
    for(int r=0; r<7; r++) for(int co=0; co<5; co++)
        if(g[r]&(1<<(4-co))) nx_fill_rect(x+co*s, y+r*s, s, s, c);
}

static void nx_draw_text(int x, int y, const char *t, int s, Color c){
    for(int i=0; t[i]; i++) nx_draw_char(x+i*6*s, y, t[i], s, c);
}

static int nx_text_width(const char *t, int s){
    return (int)strlen(t)*6*s;
}

#endif
