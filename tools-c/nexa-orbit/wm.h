/* nexa-orbit: the window manager, for both sessions NexaOS runs (Sway and
 * Hyprland). Windows are named in commands by their numeric id (Sway's
 * con_id, Hyprland's address), never by text a window chose itself. */
#ifndef NEXA_ORBIT_WM_H
#define NEXA_ORBIT_WM_H

#define WM_MAX_WIN 128

struct wm_win {
    char app_id[128];   /* as the window gives it (lowercased) */
    unsigned long id;   /* Sway con_id / Hyprland address */
    int pid;
    int minimized, focused, urgent;
    int rank;           /* 0 = most recently in front */
};

enum { WM_NONE, WM_SWAY, WM_HYPRLAND };

int wm_kind(void);
/* Every app window, including minimised ones. Count, or -1 if the
 * compositor didn't answer. */
int wm_windows(struct wm_win *out, int max);
/* Bring this window forward (out of minimised). 0 on success. */
int wm_focus(const struct wm_win *w);
int wm_close(const struct wm_win *w);
/* A socket that becomes readable when windows change; -1 if none. */
int wm_event_fd(void);
/* Reads what's waiting (Hyprland: notes urgent windows). 1 if windows may
 * have changed, 0 if not, -1 when the socket closed. */
int wm_event_read(int fd);
#endif
