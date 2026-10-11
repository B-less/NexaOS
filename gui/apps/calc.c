#include "../libnexa.h"

static char cdisp[64] = "0";
static double ca_val = 0;
static char ca_op = 0;
static int ca_fresh = 1;
static const char *cbtn[]={"C","(",")","/","7","8","9","*","4","5","6","-","1","2","3","+","0",".","<","="};
#define CC 4
#define CR 5

static void cpress(const char *b){
    if(b[0]>='0'&&b[0]<='9'){
        if(ca_fresh){cdisp[0]=0;ca_fresh=0;}
        if(strlen(cdisp)<15){size_t l=strlen(cdisp);cdisp[l]=b[0];cdisp[l+1]=0;}
    }else if(b[0]=='.'){
        if(ca_fresh){strcpy(cdisp,"0.");ca_fresh=0;}
        else if(!strchr(cdisp,'.')){size_t l=strlen(cdisp);cdisp[l]='.';cdisp[l+1]=0;}
    }else if(b[0]=='C'){strcpy(cdisp,"0");ca_val=0;ca_op=0;ca_fresh=1;}
    else if(b[0]=='<'){
        size_t l=strlen(cdisp);if(l>1)cdisp[l-1]=0;
        else{cdisp[0]='0';cdisp[1]=0;ca_fresh=1;}
    }else if(b[0]=='+'||b[0]=='-'||b[0]=='*'||b[0]=='/'){
        ca_val=atof(cdisp);ca_op=b[0];ca_fresh=1;
    }else if(b[0]=='='){
        double bv=atof(cdisp),res=0;
        if(ca_op=='+')res=ca_val+bv;else if(ca_op=='-')res=ca_val-bv;
        else if(ca_op=='*')res=ca_val*bv;
        else if(ca_op=='/'&&bv!=0)res=ca_val/bv;else res=bv;
        if(res==(int)res)snprintf(cdisp,sizeof(cdisp),"%d",(int)res);
        else snprintf(cdisp,sizeof(cdisp),"%.6g",res);
        ca_op=0;ca_fresh=1;
    }
}

static void draw(void){
    nx_fill_rect(0, 0, nx_w, nx_h, NX_BG_DARK);
    nx_fill_rect(8, 6, nx_w-16, 30, C(17,24,39));
    nx_draw_rect(8, 6, nx_w-16, 30, NX_BG_LIGHTER);
    nx_draw_text(nx_w-12-nx_text_width(cdisp,2), 12, cdisp, 2, NX_TXT_GREEN);
    
    int dy=40, pad=8;
    int bw=(nx_w-pad*(CC+1))/CC, bh=(nx_h-dy-pad*(CR+1))/CR;
    for(int r=0; r<CR; r++) for(int c=0; c<CC; c++){
        int bx = pad+c*(bw+pad), by = dy+pad+r*(bh+pad);
        int idx = r*CC+c; const char *l = cbtn[idx];
        Color bg = NX_BG_MID;
        if(l[0]=='C') bg = NX_ROSE;
        else if(l[0]=='='||l[0]=='+'||l[0]=='-'||l[0]=='*'||l[0]=='/') bg = NX_BLUE;
        nx_fill_rect(bx, by, bw, bh, bg);
        nx_draw_rect(bx, by, bw, bh, NX_BG_LIGHTER);
        nx_draw_text(bx+(bw-nx_text_width(l,2))/2, by+(bh-14)/2, l, 2, NX_WHITE);
    }
    nexa_flip();
}

int main(void){
    if(nexa_init("CALCULATOR", 250, 340, APP_CALC) < 0) return 1;

    NxEvent ev;
    int running = 1;
    draw();

    while(running){
        if(nexa_poll_event(&ev)){
            if(ev.type == NX_EV_QUIT) running = 0;
            else if(ev.type == NX_EV_RESIZE) draw();
            else if(ev.type == NX_EV_MOUSE && ev.mbtn){
                int dy=40, pad=8;
                int bw=(nx_w-pad*(CC+1))/CC, bh=(nx_h-dy-pad*(CR+1))/CR;
                for(int r=0; r<CR; r++) for(int c=0; c<CC; c++){
                    int bx = pad+c*(bw+pad), by = dy+pad+r*(bh+pad);
                    if(ev.mx >= bx && ev.mx < bx+bw && ev.my >= by && ev.my < by+bh){
                        cpress(cbtn[r*CC+c]);
                        draw();
                    }
                }
            }
            else if(ev.type == NX_EV_KEY){
                char c = ev.key;
                if(c>='0'&&c<='9'){ char b[2]={c,0}; cpress(b); }
                else if(c=='+'||c=='-'||c=='*'||c=='/'){ char b[2]={c,0}; cpress(b); }
                else if(c=='='||c=='\n'||c=='\r') cpress("=");
                else if(c=='.') cpress(".");
                else if(c==127||c=='\b') cpress("<");
                else if(c=='c'||c=='C') cpress("C");
                draw();
            }
        }
    }
    return 0;
}
