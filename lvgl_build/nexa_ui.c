#include "lvgl/lvgl.h"
#include "lv_drivers/display/fbdev.h"
#include "lv_drivers/indev/evdev.h"
#include <unistd.h>
#include <sys/time.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <time.h>

#define DISP_BUF_SIZE (1024 * 768)

/* ---- Globals ---- */
static lv_obj_t * time_lbl;
static lv_obj_t * calc_display;
static char calc_buf[64] = "0";
static double calc_acc = 0;
static char calc_op = 0;
static int calc_fresh = 1;

uint32_t custom_tick_get(void) {
    static uint64_t start_ms = 0;
    struct timeval tv_now;
    gettimeofday(&tv_now, NULL);
    uint64_t now_ms = (tv_now.tv_sec * 1000000 + tv_now.tv_usec) / 1000;
    if(start_ms == 0) start_ms = now_ms;
    return (uint32_t)(now_ms - start_ms);
}

/* ---- Close window callback ---- */
static void close_win_cb(lv_event_t * e) {
    lv_obj_t * win = lv_event_get_user_data(e);
    if(win) lv_obj_del(win);
}

/* ---- Clock timer callback ---- */
static void clock_timer_cb(lv_timer_t * t) {
    time_t now = time(NULL);
    struct tm * tm = localtime(&now);
    char buf[64];
    snprintf(buf, sizeof(buf), "Nexa OS  |  %02d:%02d:%02d", tm->tm_hour, tm->tm_min, tm->tm_sec);
    if(time_lbl) lv_label_set_text(time_lbl, buf);
}

/* ---- Calculator logic ---- */
static void calc_btn_cb(lv_event_t * e) {
    const char * txt = lv_label_get_text(lv_obj_get_child(lv_event_get_target(e), 0));
    if(!txt || !calc_display) return;

    if(strcmp(txt, "C") == 0) {
        strcpy(calc_buf, "0");
        calc_acc = 0; calc_op = 0; calc_fresh = 1;
    } else if(strcmp(txt, "=") == 0) {
        double val = atof(calc_buf);
        if(calc_op == '+') calc_acc += val;
        else if(calc_op == '-') calc_acc -= val;
        else if(calc_op == '*') calc_acc *= val;
        else if(calc_op == '/') calc_acc = val != 0 ? calc_acc / val : 0;
        else calc_acc = val;
        snprintf(calc_buf, sizeof(calc_buf), "%g", calc_acc);
        calc_op = 0; calc_fresh = 1;
    } else if(txt[0] == '+' || txt[0] == '-' || (txt[0] == '*') || txt[0] == '/') {
        double val = atof(calc_buf);
        if(calc_op == '+') calc_acc += val;
        else if(calc_op == '-') calc_acc -= val;
        else if(calc_op == '*') calc_acc *= val;
        else if(calc_op == '/') calc_acc = val != 0 ? calc_acc / val : 0;
        else calc_acc = val;
        calc_op = txt[0];
        calc_fresh = 1;
    } else {
        /* digit or dot */
        if(calc_fresh) { strcpy(calc_buf, ""); calc_fresh = 0; }
        if(strlen(calc_buf) < 15) strcat(calc_buf, txt);
    }
    lv_label_set_text(calc_display, calc_buf);
}

/* ---- APP: About ---- */
static void open_about(lv_event_t * e) {
    (void)e;
    lv_obj_t * win = lv_win_create(lv_scr_act(), 35);
    lv_obj_set_size(win, 500, 350);
    lv_obj_center(win);
    lv_win_add_title(win, LV_SYMBOL_HOME " About Nexa OS");
    lv_obj_t * cb = lv_win_add_btn(win, LV_SYMBOL_CLOSE, 35);
    lv_obj_add_event_cb(cb, close_win_cb, LV_EVENT_CLICKED, win);

    lv_obj_t * cont = lv_win_get_content(win);
    lv_obj_t * lbl = lv_label_create(cont);
    lv_label_set_text(lbl,
        "Nexa OS v2.0\n\n"
        "A custom operating system built entirely\n"
        "from scratch using the Linux kernel,\n"
        "BusyBox, musl libc, and LVGL.\n\n"
        "GUI Engine: LVGL 8.3\n"
        "Theme: Ubuntu Yaru Dark\n"
        "Compiler: musl-gcc (static)\n"
        "Kernel: Linux 6.x\n\n"
        "Created with Antigravity AI");
}

/* ---- APP: Calculator ---- */
static void open_calculator(lv_event_t * e) {
    (void)e;
    strcpy(calc_buf, "0"); calc_acc = 0; calc_op = 0; calc_fresh = 1;

    lv_obj_t * win = lv_win_create(lv_scr_act(), 35);
    lv_obj_set_size(win, 300, 420);
    lv_obj_center(win);
    lv_win_add_title(win, LV_SYMBOL_SETTINGS " Calculator");
    lv_obj_t * cb = lv_win_add_btn(win, LV_SYMBOL_CLOSE, 35);
    lv_obj_add_event_cb(cb, close_win_cb, LV_EVENT_CLICKED, win);

    lv_obj_t * cont = lv_win_get_content(win);
    lv_obj_set_flex_flow(cont, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_style_pad_all(cont, 8, 0);

    /* Display */
    calc_display = lv_label_create(cont);
    lv_label_set_text(calc_display, "0");
    lv_obj_set_width(calc_display, LV_PCT(100));
    lv_obj_set_style_text_align(calc_display, LV_TEXT_ALIGN_RIGHT, 0);
    lv_obj_set_style_text_font(calc_display, &lv_font_montserrat_14, 0);
    lv_obj_set_style_bg_color(calc_display, lv_color_hex(0x333333), 0);
    lv_obj_set_style_bg_opa(calc_display, LV_OPA_COVER, 0);
    lv_obj_set_style_text_color(calc_display, lv_color_hex(0x00FF00), 0);
    lv_obj_set_style_pad_all(calc_display, 10, 0);

    /* Button grid */
    static const char * btns[] = {
        "7","8","9","/",
        "4","5","6","*",
        "1","2","3","-",
        "C","0",".","+" ,
        "=", NULL
    };
    int row = 0, col = 0;
    for(int i = 0; btns[i] != NULL; i++) {
        if(i == 16) {
            /* Equals button - full width */
            lv_obj_t * b = lv_btn_create(cont);
            lv_obj_set_size(b, LV_PCT(100), 45);
            lv_obj_set_style_bg_color(b, lv_color_hex(0xE95420), 0);
            lv_obj_t * l = lv_label_create(b);
            lv_label_set_text(l, btns[i]);
            lv_obj_center(l);
            lv_obj_add_event_cb(b, calc_btn_cb, LV_EVENT_CLICKED, NULL);
            break;
        }
        if(col == 0) {
            /* Intentionally leave empty - flex handles it */
        }
        lv_obj_t * b = lv_btn_create(cont);
        lv_obj_set_size(b, 58, 45);
        if(btns[i][0] == '/' || btns[i][0] == '*' || btns[i][0] == '-' || btns[i][0] == '+')
            lv_obj_set_style_bg_color(b, lv_color_hex(0xE95420), 0);
        else if(btns[i][0] == 'C')
            lv_obj_set_style_bg_color(b, lv_color_hex(0xCC0000), 0);
        lv_obj_t * l = lv_label_create(b);
        lv_label_set_text(l, btns[i]);
        lv_obj_center(l);
        lv_obj_add_event_cb(b, calc_btn_cb, LV_EVENT_CLICKED, NULL);
        col++;
        if(col >= 4) { col = 0; row++; }
    }
}

/* ---- APP: Clock ---- */
static void open_clock(lv_event_t * e) {
    (void)e;
    lv_obj_t * win = lv_win_create(lv_scr_act(), 35);
    lv_obj_set_size(win, 400, 300);
    lv_obj_center(win);
    lv_win_add_title(win, LV_SYMBOL_BELL " Clock");
    lv_obj_t * cb = lv_win_add_btn(win, LV_SYMBOL_CLOSE, 35);
    lv_obj_add_event_cb(cb, close_win_cb, LV_EVENT_CLICKED, win);

    lv_obj_t * cont = lv_win_get_content(win);

    time_t now = time(NULL);
    struct tm * tm = localtime(&now);
    char buf[128];
    snprintf(buf, sizeof(buf),
        "Current Time\n\n%02d : %02d : %02d\n\n"
        "Date: %04d-%02d-%02d\n\n"
        "Uptime: %u seconds",
        tm->tm_hour, tm->tm_min, tm->tm_sec,
        tm->tm_year + 1900, tm->tm_mon + 1, tm->tm_mday,
        custom_tick_get() / 1000);

    lv_obj_t * lbl = lv_label_create(cont);
    lv_label_set_text(lbl, buf);
    lv_obj_set_style_text_font(lbl, &lv_font_montserrat_14, 0);
    lv_obj_set_style_text_align(lbl, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_center(lbl);
}

/* ---- APP: Notepad ---- */
static void open_notepad(lv_event_t * e) {
    (void)e;
    lv_obj_t * win = lv_win_create(lv_scr_act(), 35);
    lv_obj_set_size(win, 550, 400);
    lv_obj_center(win);
    lv_win_add_title(win, LV_SYMBOL_FILE " Notepad");
    lv_obj_t * cb = lv_win_add_btn(win, LV_SYMBOL_CLOSE, 35);
    lv_obj_add_event_cb(cb, close_win_cb, LV_EVENT_CLICKED, win);

    lv_obj_t * cont = lv_win_get_content(win);

    lv_obj_t * ta = lv_textarea_create(cont);
    lv_obj_set_size(ta, LV_PCT(100), LV_PCT(60));
    lv_textarea_set_placeholder_text(ta, "Type your notes here...");
    lv_textarea_set_text(ta, "");

    lv_obj_t * kb = lv_keyboard_create(cont);
    lv_obj_set_size(kb, LV_PCT(100), LV_PCT(40));
    lv_keyboard_set_textarea(kb, ta);
}

/* ---- APP: System Monitor ---- */
static void open_sysmon(lv_event_t * e) {
    (void)e;
    lv_obj_t * win = lv_win_create(lv_scr_act(), 35);
    lv_obj_set_size(win, 500, 380);
    lv_obj_center(win);
    lv_win_add_title(win, LV_SYMBOL_CHARGE " System Monitor");
    lv_obj_t * cb = lv_win_add_btn(win, LV_SYMBOL_CLOSE, 35);
    lv_obj_add_event_cb(cb, close_win_cb, LV_EVENT_CLICKED, win);

    lv_obj_t * cont = lv_win_get_content(win);
    lv_obj_set_flex_flow(cont, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_style_pad_all(cont, 10, 0);

    /* CPU Meter */
    lv_obj_t * l1 = lv_label_create(cont);
    lv_label_set_text(l1, "CPU Usage");
    lv_obj_t * bar1 = lv_bar_create(cont);
    lv_obj_set_size(bar1, LV_PCT(90), 20);
    lv_bar_set_value(bar1, 35, LV_ANIM_ON);
    lv_obj_set_style_bg_color(bar1, lv_color_hex(0xE95420), LV_PART_INDICATOR);

    /* Memory Meter */
    lv_obj_t * l2 = lv_label_create(cont);
    lv_label_set_text(l2, "Memory Usage");
    lv_obj_t * bar2 = lv_bar_create(cont);
    lv_obj_set_size(bar2, LV_PCT(90), 20);
    lv_bar_set_value(bar2, 52, LV_ANIM_ON);
    lv_obj_set_style_bg_color(bar2, lv_color_hex(0x77216F), LV_PART_INDICATOR);

    /* Disk Meter */
    lv_obj_t * l3 = lv_label_create(cont);
    lv_label_set_text(l3, "Disk Usage");
    lv_obj_t * bar3 = lv_bar_create(cont);
    lv_obj_set_size(bar3, LV_PCT(90), 20);
    lv_bar_set_value(bar3, 18, LV_ANIM_ON);
    lv_obj_set_style_bg_color(bar3, lv_color_hex(0x308280), LV_PART_INDICATOR);

    /* System Info */
    lv_obj_t * info = lv_label_create(cont);
    char buf[256];
    snprintf(buf, sizeof(buf),
        "\nKernel: Linux 6.x\n"
        "GUI: LVGL 8.3 (Framebuffer)\n"
        "Uptime: %u sec\n"
        "Architecture: x86_64\n"
        "Shell: BusyBox ash",
        custom_tick_get() / 1000);
    lv_label_set_text(info, buf);
}

/* ---- APP: Calendar ---- */
static void open_calendar(lv_event_t * e) {
    (void)e;
    lv_obj_t * win = lv_win_create(lv_scr_act(), 35);
    lv_obj_set_size(win, 400, 400);
    lv_obj_center(win);
    lv_win_add_title(win, LV_SYMBOL_LIST " Calendar");
    lv_obj_t * cb = lv_win_add_btn(win, LV_SYMBOL_CLOSE, 35);
    lv_obj_add_event_cb(cb, close_win_cb, LV_EVENT_CLICKED, win);

    lv_obj_t * cont = lv_win_get_content(win);
    lv_obj_t * cal = lv_calendar_create(cont);
    lv_obj_set_size(cal, LV_PCT(100), LV_PCT(100));
    lv_calendar_header_arrow_create(cal);

    time_t now = time(NULL);
    struct tm * tm = localtime(&now);
    lv_calendar_set_today_date(cal, tm->tm_year + 1900, tm->tm_mon + 1, tm->tm_mday);
    lv_calendar_set_showed_date(cal, tm->tm_year + 1900, tm->tm_mon + 1);
}

/* ---- Helper: create dock button ---- */
static lv_obj_t * add_dock_btn(lv_obj_t * dock, const char * icon, lv_color_t color,
                                int y_offset, lv_event_cb_t cb) {
    lv_obj_t * btn = lv_btn_create(dock);
    lv_obj_set_size(btn, 42, 42);
    lv_obj_align(btn, LV_ALIGN_TOP_MID, 0, y_offset);
    lv_obj_set_style_bg_color(btn, color, 0);
    lv_obj_set_style_radius(btn, 10, 0);
    lv_obj_t * lbl = lv_label_create(btn);
    lv_label_set_text(lbl, icon);
    lv_obj_center(lbl);
    lv_obj_add_event_cb(btn, cb, LV_EVENT_CLICKED, NULL);
    return btn;
}

/* ---- MAIN ---- */
int main(void)
{
    lv_init();
    fbdev_init();

    static lv_color_t buf[DISP_BUF_SIZE];
    static lv_disp_draw_buf_t disp_buf;
    lv_disp_draw_buf_init(&disp_buf, buf, NULL, DISP_BUF_SIZE);

    static lv_disp_drv_t disp_drv;
    lv_disp_drv_init(&disp_drv);
    disp_drv.draw_buf   = &disp_buf;
    disp_drv.flush_cb   = fbdev_flush;
    disp_drv.hor_res    = 1024;
    disp_drv.ver_res    = 768;
    lv_disp_drv_register(&disp_drv);

    evdev_init();
    static lv_indev_drv_t indev_drv;
    lv_indev_drv_init(&indev_drv);
    indev_drv.type = LV_INDEV_TYPE_POINTER;
    indev_drv.read_cb = evdev_read;
    lv_indev_t * mouse_indev = lv_indev_drv_register(&indev_drv);

    lv_obj_t * cursor_obj = lv_label_create(lv_scr_act());
    lv_label_set_text(cursor_obj, LV_SYMBOL_UP);
    lv_indev_set_cursor(mouse_indev, cursor_obj);

    /* ---- Desktop Background ---- */
    lv_obj_t * bg = lv_obj_create(lv_scr_act());
    lv_obj_set_size(bg, 1024, 768);
    lv_obj_set_style_bg_color(bg, lv_color_hex(0x2C001E), 0);
    lv_obj_set_style_bg_grad_color(bg, lv_color_hex(0x77216F), 0);
    lv_obj_set_style_bg_grad_dir(bg, LV_GRAD_DIR_VER, 0);
    lv_obj_set_style_border_width(bg, 0, 0);

    /* ---- Top Bar ---- */
    lv_obj_t * top_bar = lv_obj_create(lv_scr_act());
    lv_obj_set_size(top_bar, 1024, 30);
    lv_obj_align(top_bar, LV_ALIGN_TOP_MID, 0, 0);
    lv_obj_set_style_radius(top_bar, 0, 0);
    lv_obj_set_style_bg_color(top_bar, lv_color_hex(0x111111), 0);
    lv_obj_set_style_border_width(top_bar, 0, 0);

    /* Activities label (left) */
    lv_obj_t * act_lbl = lv_label_create(top_bar);
    lv_label_set_text(act_lbl, "Activities");
    lv_obj_align(act_lbl, LV_ALIGN_LEFT_MID, 10, 0);
    lv_obj_set_style_text_color(act_lbl, lv_color_hex(0xFFFFFF), 0);

    /* Clock label (center) */
    time_lbl = lv_label_create(top_bar);
    lv_label_set_text(time_lbl, "Nexa OS  |  00:00:00");
    lv_obj_align(time_lbl, LV_ALIGN_CENTER, 0, 0);
    lv_obj_set_style_text_color(time_lbl, lv_color_hex(0xFFFFFF), 0);

    /* Power icon (right) */
    lv_obj_t * pwr_lbl = lv_label_create(top_bar);
    lv_label_set_text(pwr_lbl, LV_SYMBOL_POWER " " LV_SYMBOL_WIFI " " LV_SYMBOL_BATTERY_FULL);
    lv_obj_align(pwr_lbl, LV_ALIGN_RIGHT_MID, -10, 0);
    lv_obj_set_style_text_color(pwr_lbl, lv_color_hex(0xFFFFFF), 0);

    /* ---- Left Dock ---- */
    lv_obj_t * dock = lv_obj_create(lv_scr_act());
    lv_obj_set_size(dock, 58, 738);
    lv_obj_align(dock, LV_ALIGN_BOTTOM_LEFT, 0, 0);
    lv_obj_set_style_radius(dock, 0, 0);
    lv_obj_set_style_bg_color(dock, lv_color_hex(0x1A1A1A), 0);
    lv_obj_set_style_bg_opa(dock, LV_OPA_90, 0);
    lv_obj_set_style_border_width(dock, 0, 0);

    /* Dock buttons */
    add_dock_btn(dock, LV_SYMBOL_HOME,     lv_color_hex(0xE95420), 10,  open_about);
    add_dock_btn(dock, LV_SYMBOL_SETTINGS, lv_color_hex(0x308280), 60,  open_calculator);
    add_dock_btn(dock, LV_SYMBOL_BELL,     lv_color_hex(0x77216F), 110, open_clock);
    add_dock_btn(dock, LV_SYMBOL_FILE,     lv_color_hex(0x5E2750), 160, open_notepad);
    add_dock_btn(dock, LV_SYMBOL_CHARGE,   lv_color_hex(0xCC0000), 210, open_sysmon);
    add_dock_btn(dock, LV_SYMBOL_LIST,     lv_color_hex(0x2C001E), 260, open_calendar);

    /* ---- Live clock timer ---- */
    lv_timer_create(clock_timer_cb, 1000, NULL);

    /* ---- Main loop ---- */
    while(1) {
        lv_timer_handler();
        usleep(5000);
    }
    return 0;
}
