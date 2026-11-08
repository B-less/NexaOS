/* nexa-voice: Nexa Mind's voice and ears on the desktop. The helix
 * (helix.c) is their face: a small overlay at the bottom of the screen that
 * moves with the sound while Nexa listens, thinks and speaks.
 *
 *   nexa-voice say [--voice NAME] [--speed X] [TEXT...]   TEXT, or stdin
 *   nexa-voice listen [--max SECONDS]   what you said, as an event
 *   nexa-voice serve                    commands on stdin (the assistant)
 *   nexa-voice stop                     the running one: stop talking, or
 *                                       finish listening (push-to-talk)
 *
 * Events, one JSON object a line on stdout:
 *   {"event":"speaking","text":S}  each sentence as it starts
 *   {"event":"done"}  {"event":"stopped"}
 *   {"event":"heard","text":S}     "" if nothing was said
 *   {"event":"error","error":S,"hint":S}
 * serve commands, one JSON object a line:
 *   {"say":TEXT,"voice":NAME,"speed":X} {"listen":true} {"think":true}
 *   {"stop":true} {"cancel":true} {"hide":true}
 *
 * The voice is Kokoro in nexa-mindd (POST /v1/audio/speech, sentence by
 * sentence); before that model is downloaded, espeak-ng; with neither, the
 * helix still moves, silently. The helix follows the audio itself: its
 * loudness, syllables and brightness are measured from the samples as they
 * play (helix.c, voiceprint). Hearing: the microphone through GStreamer,
 * 16 kHz mono, ending after a pause, then whisper in nexa-mindd.
 *
 * Settings: ~/.config/nexa/voice.conf (voice=, speed=, colours=voice|cycle|
 * calm|ember); colours from the theme (lock.conf); "reduce animations"
 * (gtk-enable-animations off) keeps the strands still.
 * Tests: NEXA_VOICE_STATE=<file> (what is shown, as JSON), NEXA_VOICE_SINK
 * and NEXA_VOICE_SOURCE (GStreamer descriptions), NEXA_MIND_SOCKET,
 * NEXA_VOICE_ESPEAK (path, or "none"). */
#include "helix.h"
#include "json.h"

#include <errno.h>
#include <gst/app/gstappsink.h>
#include <gst/app/gstappsrc.h>
#include <gst/gst.h>
#include <gtk-layer-shell/gtk-layer-shell.h>
#include <gtk/gtk.h>
#include <glib-unix.h>
#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <unistd.h>

#define W 520
#define H 176
#define HELIX_H 124
#define LISTEN_RATE 16000
#define MAX_TEXT (64 * 1024)

typedef struct { double r, g, b; } rgb;

typedef struct {
    long start, n;      /* samples */
    char *text;
    int announced;
} sentence;

enum task { T_NONE, T_SPEAK, T_LISTEN, T_HEAR, T_THINK };

static struct {
    GtkWidget *win, *area;
    helix hx;
    int serve, oneshot, shown;
    double fade, fade_to;
    gint64 last_us;
    rgb bg, text, muted, accent;
    double opacity;
    char voice[40];
    double speed;
    char theme_path[600], conf_path[600];
    enum task task;
    int gen;                    /* bumped on every stop: old threads go quiet */
    char caption[1024];
    /* playing */
    GstElement *play;
    GMutex lock;                /* the fields below are shared with threads */
    GArray *frames;             /* vp_frame, 100 a second of audio fed */
    GPtrArray *sentences;
    int rate, fed_all, speech_fd;
    long fed;                   /* samples handed to the player */
    int buffering, underruns;   /* paused until more speech is ready */
    GPid espeak_pid;
    long next_frame;
    const char *source;         /* kokoro, espeak, silent */
    /* listening */
    GstElement *rec;
    GByteArray *pcm;
    GArray *rframes;
    vp_state rvp;
    long voiced, silent_run, rec_frames;
    int max_s, rec_eos;
    /* what happened, for tests */
    GPtrArray *spoken;
    char *heard, *error;
    long events;
    int fps, frames_this_second;   /* how smoothly the helix moves */
    gint64 second_start;
} V;

/* ---------- small helpers ---------- */

static void esc(GString *s, const char *t)
{
    g_string_append_c(s, '"');
    for (const unsigned char *p = (const unsigned char *)(t ? t : ""); *p; p++) {
        if (*p == '"' || *p == '\\') g_string_append_printf(s, "\\%c", *p);
        else if (*p == '\n') g_string_append(s, "\\n");
        else if (*p < 0x20) g_string_append_printf(s, "\\u%04x", *p);
        else g_string_append_c(s, *p);
    }
    g_string_append_c(s, '"');
}

static void event(const char *name, const char *k1, const char *v1, const char *k2, const char *v2)
{
    GString *s = g_string_new("{\"event\":");
    esc(s, name);
    if (k1) { g_string_append_printf(s, ",\"%s\":", k1); esc(s, v1); }
    if (k2) { g_string_append_printf(s, ",\"%s\":", k2); esc(s, v2); }
    g_string_append(s, "}\n");
    fputs(s->str, stdout);
    fflush(stdout);
    g_string_free(s, TRUE);
    V.events++;
}

static void config_path(char *out, size_t cap, const char *name)
{
    const char *cfg = g_getenv("XDG_CONFIG_HOME");
    if (cfg && *cfg) snprintf(out, cap, "%s/nexa/%s", cfg, name);
    else snprintf(out, cap, "%s/.config/nexa/%s", g_get_home_dir(), name);
}

static rgb hexcol(const char *s, rgb fb)
{
    unsigned v;
    if (!s || sscanf(s, "#%6x", &v) != 1) return fb;
    return (rgb){((v >> 16) & 255) / 255.0, ((v >> 8) & 255) / 255.0, (v & 255) / 255.0};
}

static double hue_of(rgb c)
{
    double mx = fmax(c.r, fmax(c.g, c.b)), mn = fmin(c.r, fmin(c.g, c.b)), d = mx - mn;
    if (d < 1e-6) return 210;
    double h = mx == c.r ? fmod((c.g - c.b) / d, 6) : mx == c.g ? (c.b - c.r) / d + 2 : (c.r - c.g) / d + 4;
    return fmod(h * 60 + 360, 360);
}

static void load_theme(void)
{
    V.bg = hexcol("#080c1e", V.bg);
    V.text = hexcol("#e6ecff", V.text);
    V.muted = hexcol("#8f9bc4", V.muted);
    V.accent = hexcol("#4da3ff", V.accent);
    V.opacity = 0.78;
    char *t = NULL;
    if (g_file_get_contents(V.theme_path, &t, NULL, NULL)) {
        char **lines = g_strsplit(t, "\n", -1);
        for (char **l = lines; *l; l++) {
            char *eq = strchr(*l, '=');
            if (!eq || **l == '#') continue;
            *eq = 0;
            const char *k = *l, *v = eq + 1;
            if (!strcmp(k, "bg")) V.bg = hexcol(v, V.bg);
            else if (!strcmp(k, "text")) V.text = hexcol(v, V.text);
            else if (!strcmp(k, "muted")) V.muted = hexcol(v, V.muted);
            else if (!strcmp(k, "accent")) V.accent = hexcol(v, V.accent);
            else if (!strcmp(k, "opacity")) V.opacity = fmin(1.0, fmax(0.3, g_ascii_strtod(v, NULL)));
        }
        g_strfreev(lines);
        g_free(t);
    }
    V.hx.accent_hue = hue_of(V.accent);
}

static int name_ok(const char *v)
{
    size_t n = strlen(v);
    if (n < 3 || n > 32) return 0;
    for (const char *p = v; *p; p++)
        if (!g_ascii_islower(*p) && !g_ascii_isdigit(*p) && *p != '_') return 0;
    return 1;
}

static void load_conf(void)
{
    snprintf(V.voice, sizeof V.voice, "af_heart");
    V.speed = 1.0;
    V.hx.palette = HELIX_VOICE;
    char *t = NULL;
    if (!g_file_get_contents(V.conf_path, &t, NULL, NULL)) return;
    char **lines = g_strsplit(t, "\n", -1);
    for (char **l = lines; *l; l++) {
        char *eq = strchr(*l, '=');
        if (!eq || **l == '#') continue;
        *eq = 0;
        const char *k = g_strstrip(*l), *v = g_strstrip(eq + 1);
        if (!strcmp(k, "voice") && name_ok(v)) snprintf(V.voice, sizeof V.voice, "%s", v);
        else if (!strcmp(k, "speed")) {
            double s = g_ascii_strtod(v, NULL);
            if (s >= 0.5 && s <= 2) V.speed = s;
        } else if (!strcmp(k, "colours")) {
            V.hx.palette = !strcmp(v, "cycle") ? HELIX_CYCLE : !strcmp(v, "calm") ? HELIX_CALM
                         : !strcmp(v, "ember") ? HELIX_EMBER : HELIX_VOICE;
        }
    }
    g_strfreev(lines);
    g_free(t);
}

static void read_animations(void)
{
    gboolean on = TRUE;
    g_object_get(gtk_settings_get_default(), "gtk-enable-animations", &on, NULL);
    V.hx.animations = on;
}

/* ---------- state for tests ---------- */

static const char *mode_name(void)
{
    static const char *const names[] = {"idle", "listening", "thinking", "speaking"};
    return names[V.hx.mode];
}

static void write_state(void)
{
    const char *path = g_getenv("NEXA_VOICE_STATE");
    if (!path || !*path) return;
    static const char *const tasks[] = {"none", "speak", "listen", "hear", "think"};
    GString *s = g_string_new("{");
    g_string_append_printf(s, "\"mode\": \"%s\", \"task\": \"%s\", \"shown\": %d, \"animations\": %d, \"palette\": %d, ",
                           mode_name(), tasks[V.task], V.shown && V.fade_to > 0, V.hx.animations, V.hx.palette);
    g_mutex_lock(&V.lock);
    g_string_append(s, "\"source\": ");
    esc(s, V.source ? V.source : "");
    g_string_append_printf(s, ", \"rate\": %d, \"frames\": %u, \"played_frames\": %ld, ", V.rate,
                           V.frames ? V.frames->len : 0, V.next_frame);
    g_mutex_unlock(&V.lock);
    g_string_append_printf(s, "\"pops\": %ld, \"env_peak\": %.3f, \"recorded_ms\": %ld, \"voiced_ms\": %ld, \"events\": %ld, ",
                           V.hx.pops_total, V.hx.env_peak, V.rec_frames * 10, V.voiced * 10, V.events);
    g_string_append_printf(s, "\"fps\": %d, \"underruns\": %d, ", V.fps, V.underruns);
    g_string_append_printf(s, "\"voice\": \"%s\", \"speed\": %.2f, \"accent_hue\": %.0f, \"caption\": ", V.voice, V.speed, V.hx.accent_hue);
    esc(s, V.caption);
    g_string_append(s, ", \"heard\": ");
    if (V.heard) esc(s, V.heard); else g_string_append(s, "null");
    g_string_append(s, ", \"error\": ");
    if (V.error) esc(s, V.error); else g_string_append(s, "null");
    g_string_append(s, ", \"spoken\": [");
    for (guint i = 0; i < V.spoken->len; i++) {
        if (i) g_string_append(s, ", ");
        esc(s, g_ptr_array_index(V.spoken, i));
    }
    g_string_append(s, "]}\n");
    char *tmp = g_strdup_printf("%s.tmp", path);
    if (g_file_set_contents(tmp, s->str, -1, NULL)) rename(tmp, path);
    g_free(tmp);
    g_string_free(s, TRUE);
}

/* ---------- showing ---------- */

static guint tick_id;
static gboolean tick(gpointer d);

/* The clock runs only while there is something to show or do. */
static void ensure_tick(void)
{
    if (!tick_id) {
        V.last_us = 0;
        tick_id = g_timeout_add(16, tick, NULL);
    }
}

static void show(void)
{
    ensure_tick();
    if (!V.shown) {
        V.shown = 1;
        V.fade = V.hx.animations ? 0 : 1;
        gtk_widget_show_all(V.win);
    }
    V.fade_to = 1;
}

static void hide(void)
{
    V.fade_to = 0;
    if (!V.hx.animations) V.fade = 0;
}

static void set_caption(const char *t)
{
    snprintf(V.caption, sizeof V.caption, "%s", t ? t : "");
}

static void finish_oneshot(void);

/* ---------- talking to nexa-mindd ---------- */

static const char *mind_socket(void)
{
    const char *s = g_getenv("NEXA_MIND_SOCKET");
    return s && *s ? s : "/run/nexa-mind/mind.sock";
}

/* POST a body; returns the connection positioned at the response body and
 * sets *status, or NULL if the service can't be reached. */
static FILE *mind_post(const char *path, const char *type, const void *body, size_t len, int *status, int *fd_out)
{
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return NULL;
    struct sockaddr_un a = {.sun_family = AF_UNIX};
    snprintf(a.sun_path, sizeof a.sun_path, "%s", mind_socket());
    if (connect(fd, (struct sockaddr *)&a, sizeof a)) { close(fd); return NULL; }
    char head[512];
    int hl = snprintf(head, sizeof head, "POST %s HTTP/1.1\r\nHost: localhost\r\nContent-Type: %s\r\n"
                      "Content-Length: %zu\r\nConnection: close\r\n\r\n", path, type, len);
    const char *parts[2] = {head, body};
    size_t lens[2] = {(size_t)hl, len};
    for (int i = 0; i < 2; i++) {
        for (size_t off = 0; off < lens[i];) {
            ssize_t w = send(fd, parts[i] + off, lens[i] - off, MSG_NOSIGNAL);
            if (w < 0 && errno == EINTR) continue;
            if (w <= 0) { close(fd); return NULL; }
            off += w;
        }
    }
    FILE *f = fdopen(fd, "r");
    if (!f) { close(fd); return NULL; }
    char line[1024];
    *status = 0;
    if (!fgets(line, sizeof line, f) || sscanf(line, "HTTP/1.%*d %d", status) != 1) { fclose(f); return NULL; }
    while (fgets(line, sizeof line, f))
        if (!strcmp(line, "\r\n") || !strcmp(line, "\n")) break;
    if (fd_out) *fd_out = fd;
    return f;
}

static char *read_all(FILE *f, size_t cap)
{
    GString *s = g_string_new(NULL);
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0 && s->len < cap) g_string_append_len(s, buf, n);
    return g_string_free(s, FALSE);
}

/* "error" and "hint" from a JSON reply */
static void reply_error(char *body, char **err, char **hint)
{
    struct jdoc d = {0};
    *err = *hint = NULL;
    if (!json_parse(&d, body) && d.count && d.n[0].type == J_OBJ) {
        *err = g_strdup(json_str(&d, 0, "error", "Nexa Mind refused"));
        const char *h = json_str(&d, 0, "hint", NULL);
        if (h) *hint = g_strdup(h);
    }
    json_free(&d);
    if (!*err) *err = g_strdup("Nexa Mind didn't answer");
}

/* ---------- speaking ---------- */

typedef struct {
    int gen;
    char *text, voice[40];
    double speed;
    GstElement *src;
    vp_state vp;
    long fed;
    int rate;
} job;

static int current(job *j)
{
    return g_atomic_int_get(&V.gen) == j->gen;
}

/* Audio for the player and the helix: measured, then pushed. */
static int feed(job *j, const int16_t *s, size_t n, int rate)
{
    if (!current(j)) return -1;
    if (j->rate != rate) {
        GstCaps *caps = gst_caps_new_simple("audio/x-raw", "format", G_TYPE_STRING, "S16LE", "rate", G_TYPE_INT, rate,
                                            "channels", G_TYPE_INT, 1, "layout", G_TYPE_STRING, "interleaved", NULL);
        gst_app_src_set_caps(GST_APP_SRC(j->src), caps);
        gst_caps_unref(caps);
        if (j->rate) j->fed = j->fed * rate / j->rate;
        j->rate = rate;
        vp_init(&j->vp, rate);
        g_mutex_lock(&V.lock);
        V.rate = rate;
        g_mutex_unlock(&V.lock);
    }
    vp_frame fr[512];
    for (size_t off = 0; off < n;) {
        size_t part = MIN(n - off, (size_t)rate / 2);
        int made = vp_feed(&j->vp, s + off, part, fr, 512);
        g_mutex_lock(&V.lock);
        if (current(j)) g_array_append_vals(V.frames, fr, made);
        g_mutex_unlock(&V.lock);
        GstBuffer *b = gst_buffer_new_memdup(s + off, part * 2);
        GST_BUFFER_PTS(b) = gst_util_uint64_scale(j->fed, GST_SECOND, rate);
        GST_BUFFER_DURATION(b) = gst_util_uint64_scale(part, GST_SECOND, rate);
        j->fed += part;
        if (gst_app_src_push_buffer(GST_APP_SRC(j->src), b) != GST_FLOW_OK) return -1;
        g_mutex_lock(&V.lock);
        if (current(j)) V.fed = j->fed;
        g_mutex_unlock(&V.lock);
        off += part;
    }
    return 0;
}

static void add_sentence(job *j, const char *text, long n)
{
    sentence *s = g_new0(sentence, 1);
    s->start = j->fed;
    s->n = n;
    s->text = g_strdup(text);
    g_mutex_lock(&V.lock);
    if (current(j)) g_ptr_array_add(V.sentences, s);
    else { g_free(s->text); g_free(s); }
    g_mutex_unlock(&V.lock);
}

static void set_source(job *j, const char *src)
{
    g_mutex_lock(&V.lock);
    if (current(j)) V.source = src;
    g_mutex_unlock(&V.lock);
}

typedef struct { int gen; char *err, *hint; } failure;

static gboolean on_speak_failed(gpointer p);

/* Kokoro through nexa-mindd. 1 spoken, 0 not installed or unreachable
 * (try espeak-ng), -1 failed or stopped. */
static int speak_kokoro(job *j, char **err, char **hint)
{
    GString *body = g_string_new("{\"text\":");
    esc(body, j->text);
    g_string_append(body, ",\"voice\":");
    esc(body, j->voice);
    g_string_append_printf(body, ",\"speed\":%.2f}", j->speed);
    int status = 0, fd = -1;
    FILE *f = mind_post("/v1/audio/speech", "application/json", body->str, body->len, &status, &fd);
    g_string_free(body, TRUE);
    if (!f) return 0;
    g_mutex_lock(&V.lock);
    if (current(j)) V.speech_fd = fd;
    g_mutex_unlock(&V.lock);
    if (!current(j)) { fclose(f); return -1; }
    if (status != 200) {
        char *b = read_all(f, 65536);
        fclose(f);
        int missing = status == 409;
        if (!missing) reply_error(b, err, hint);
        g_free(b);
        return missing ? 0 : -1;
    }
    set_source(j, "kokoro");
    int rc = 1;
    char line[256];
    int16_t *pcm = NULL;
    while (fgets(line, sizeof line, f)) {
        int rate;
        long samples, tlen;
        if (!strcmp(line, "END\n")) break;
        if (!strncmp(line, "ERR ", 4)) {
            line[strcspn(line, "\n")] = 0;
            *err = g_strdup(line + 4);
            rc = -1;
            break;
        }
        if (sscanf(line, "C %d %ld %ld", &rate, &samples, &tlen) != 3 || rate < 8000 || rate > 48000 ||
            samples < 0 || samples > 48000L * 120 || tlen < 0 || tlen > MAX_TEXT) { rc = -1; break; }
        char *text = g_malloc(tlen + 1);
        pcm = g_realloc(pcm, samples * 2 + 2);
        if (fread(text, 1, tlen, f) != (size_t)tlen || fread(pcm, 2, samples, f) != (size_t)samples) { g_free(text); rc = -1; break; }
        text[tlen] = 0;
        if (j->rate != rate) feed(j, pcm, 0, rate);
        add_sentence(j, text, samples);
        g_free(text);
        if (feed(j, pcm, samples, rate)) { rc = -1; break; }
    }
    if (rc == 1 && feof(f) && !current(j)) rc = -1;
    g_free(pcm);
    g_mutex_lock(&V.lock);
    if (V.speech_fd == fd) V.speech_fd = -1;
    g_mutex_unlock(&V.lock);
    fclose(f);
    if (!current(j)) return -1;
    if (rc < 0 && !*err) *err = g_strdup("the voice stopped part way");
    return rc;
}

/* espeak-ng, the fallback voice: 1 spoken, 0 not there, -1 stopped. */
static int speak_espeak(job *j)
{
    const char *bin = g_getenv("NEXA_VOICE_ESPEAK");
    if (!bin || !*bin) bin = "/usr/bin/espeak-ng";
    if (!strcmp(bin, "none") || access(bin, X_OK)) return 0;
    /* plain words: no Markdown marks read out */
    GString *t = g_string_new(NULL);
    for (const char *p = j->text; *p; p++)
        if (!strchr("*_`#>|~\\", *p)) g_string_append_c(t, *p);
    char speed[16];
    snprintf(speed, sizeof speed, "%d", (int)lround(175 * j->speed));
    char *argv[] = {(char *)bin, "--stdout", "-v", "en-us", "-s", speed, "--", t->str, NULL};
    GPid pid;
    int out = -1;
    if (!g_spawn_async_with_pipes(NULL, argv, NULL, G_SPAWN_DO_NOT_REAP_CHILD | G_SPAWN_STDERR_TO_DEV_NULL, NULL, NULL,
                                  &pid, NULL, &out, NULL, NULL)) {
        g_string_free(t, TRUE);
        return 0;
    }
    g_mutex_lock(&V.lock);
    if (current(j)) V.espeak_pid = pid;
    g_mutex_unlock(&V.lock);
    set_source(j, "espeak");
    FILE *f = fdopen(out, "r");
    unsigned char hdr[44];
    int rc = -1;
    if (f && fread(hdr, 1, 44, f) == 44 && !memcmp(hdr, "RIFF", 4) && !memcmp(hdr + 8, "WAVE", 4)) {
        int rate = hdr[24] | hdr[25] << 8 | hdr[26] << 16 | hdr[27] << 24;
        if (rate >= 8000 && rate <= 48000) {
            add_sentence(j, t->str, 0);
            int16_t buf[4096];
            size_t n;
            rc = 1;
            while ((n = fread(buf, 2, 4096, f)) > 0)
                if (feed(j, buf, n, rate)) { rc = -1; break; }
        }
    }
    if (f) fclose(f); else close(out);
    kill(pid, SIGTERM);
    waitpid(pid, NULL, 0);
    g_spawn_close_pid(pid);
    g_mutex_lock(&V.lock);
    if (V.espeak_pid == pid) V.espeak_pid = 0;
    g_mutex_unlock(&V.lock);
    g_string_free(t, TRUE);
    return current(j) ? rc : -1;
}

/* No voice at all: the helix "speaks" in silence, with syllables at a
 * talking pace, so it still shows Nexa answering. */
static void speak_silent(job *j)
{
    set_source(j, "silent");
    int rate = 16000;
    add_sentence(j, j->text, 0);
    char **words = g_strsplit_set(j->text, " \n\t", -1);
    for (char **w = words; *w && current(j); w++) {
        if (!**w) continue;
        int letters = 0;
        for (char *p = *w; *p; p++) letters += g_ascii_isalpha(*p);
        long n = (long)((0.12 + 0.058 * letters) * rate);
        int16_t *s = g_new0(int16_t, n);
        int syl = MAX(1, letters / 3);
        for (long i = 0; i < n; i++) {
            double ph = fmod((double)i / n * syl, 1.0);
            double env = ph < 0.15 ? ph / 0.15 : exp(-(ph - 0.15) * 4);
            s[i] = (int16_t)(env * 6000 * sin(i * 0.31) * sin(i * 0.013));
        }
        int gap = strchr(",.;:!?", (*w)[strlen(*w) - 1]) ? rate / 3 : rate / 16;
        s = g_renew(int16_t, s, n + gap);
        memset(s + n, 0, gap * 2);
        int stop = feed(j, s, n + gap, rate);
        g_free(s);
        if (stop) break;
    }
    g_strfreev(words);
}

static gpointer speak_thread(gpointer p)
{
    job *j = p;
    char *err = NULL, *hint = NULL;
    int rc = speak_kokoro(j, &err, &hint);
    if (rc == 0) rc = speak_espeak(j);
    if (rc == 0) speak_silent(j);
    g_mutex_lock(&V.lock);
    if (current(j)) V.fed_all = 1;
    g_mutex_unlock(&V.lock);
    if (current(j)) gst_app_src_end_of_stream(GST_APP_SRC(j->src));
    if (rc < 0 && err && current(j)) {
        failure *f = g_new0(failure, 1);
        f->gen = j->gen;
        f->err = err;
        f->hint = hint;
        g_idle_add(on_speak_failed, f);
    } else {
        g_free(err);
        g_free(hint);
    }
    gst_object_unref(j->src);
    g_free(j->text);
    g_free(j);
    return NULL;
}

static void stop_all(int tell);

static gboolean on_speak_failed(gpointer p)
{
    failure *f = p;
    if (f->gen == g_atomic_int_get(&V.gen) && V.task == T_SPEAK) {
        g_free(V.error);
        V.error = g_strdup(f->err);
        event("error", "error", f->err, f->hint ? "hint" : NULL, f->hint);
        stop_all(0);
        V.hx.mode = HELIX_IDLE;
        hide();
        finish_oneshot();
    }
    g_free(f->err);
    g_free(f->hint);
    g_free(f);
    return G_SOURCE_REMOVE;
}

static gboolean on_play_bus(GstBus *bus, GstMessage *m, gpointer gen)
{
    if (GPOINTER_TO_INT(gen) != g_atomic_int_get(&V.gen) || V.task != T_SPEAK) return G_SOURCE_REMOVE;
    if (GST_MESSAGE_TYPE(m) == GST_MESSAGE_EOS) {
        event("done", NULL, NULL, NULL, NULL);
        stop_all(0);
        V.hx.mode = HELIX_IDLE;
        hide();
        finish_oneshot();
        return G_SOURCE_REMOVE;
    }
    if (GST_MESSAGE_TYPE(m) == GST_MESSAGE_ERROR) {
        GError *e = NULL;
        gst_message_parse_error(m, &e, NULL);
        g_free(V.error);
        V.error = g_strdup(e ? e->message : "sound failed");
        event("error", "error", V.error, "hint", "Check the sound output in Settings > Sound");
        if (e) g_error_free(e);
        stop_all(0);
        V.hx.mode = HELIX_IDLE;
        hide();
        finish_oneshot();
        return G_SOURCE_REMOVE;
    }
    return G_SOURCE_CONTINUE;
}

/* A player that works: the sound output, or (no sound device) a clock. */
static GstElement *make_player(GstElement **src)
{
    const char *sink = g_getenv("NEXA_VOICE_SINK");
    if (!sink || !*sink) sink = "autoaudiosink";
    for (int attempt = 0; attempt < 2; attempt++) {
        char *desc = g_strdup_printf("appsrc name=src format=time ! queue max-size-time=0 max-size-buffers=0 max-size-bytes=0 "
                                     "! audioconvert ! audioresample ! %s", attempt ? "fakesink sync=true" : sink);
        GError *e = NULL;
        GstElement *p = gst_parse_launch(desc, &e);
        g_free(desc);
        if (e) g_error_free(e);
        if (p && gst_element_set_state(p, GST_STATE_PAUSED) != GST_STATE_CHANGE_FAILURE) {
            *src = gst_bin_get_by_name(GST_BIN(p), "src");
            return p;
        }
        if (p) { gst_element_set_state(p, GST_STATE_NULL); gst_object_unref(p); }
        if (!attempt) fprintf(stderr, "nexa-voice: no sound output (%s); the helix runs without sound\n", sink);
    }
    return NULL;
}

static void start_speaking(const char *text, const char *voice, double speed)
{
    stop_all(1);
    GstElement *src = NULL;
    V.play = make_player(&src);
    if (!V.play) {
        event("error", "error", "can't play sound", NULL, NULL);
        finish_oneshot();
        return;
    }
    g_mutex_lock(&V.lock);
    g_array_set_size(V.frames, 0);
    g_ptr_array_set_size(V.sentences, 0);
    V.next_frame = 0;
    V.rate = 0;
    V.fed = 0;
    V.fed_all = 0;
    V.source = NULL;
    g_mutex_unlock(&V.lock);
    V.buffering = 0;
    V.underruns = 0;
    g_ptr_array_set_size(V.spoken, 0);
    GstBus *bus = gst_element_get_bus(V.play);
    gst_bus_add_watch(bus, on_play_bus, GINT_TO_POINTER(g_atomic_int_get(&V.gen)));
    gst_object_unref(bus);
    gst_element_set_state(V.play, GST_STATE_PLAYING);
    job *j = g_new0(job, 1);
    j->gen = g_atomic_int_get(&V.gen);
    j->text = g_strndup(text, MAX_TEXT);
    snprintf(j->voice, sizeof j->voice, "%s", voice && name_ok(voice) ? voice : V.voice);
    j->speed = speed >= 0.5 && speed <= 2 ? speed : V.speed;
    j->src = src;   /* the thread's reference */
    V.task = T_SPEAK;
    V.hx.mode = HELIX_SPEAKING;
    set_caption("");
    show();
    g_thread_unref(g_thread_new("speak", speak_thread, j));
}

/* Where the voice is now: frames since the last tick, and the sentence. */
static void playing_now(double *level, double *onset, double *bright)
{
    gint64 pos = -1;
    if (!V.play || !gst_element_query_position(V.play, GST_FORMAT_TIME, &pos) || pos < 0) return;
    long fi = (long)(pos / (GST_SECOND / 100));
    g_mutex_lock(&V.lock);
    long have = V.frames->len;
    if (fi > have) fi = have;
    for (long i = V.next_frame; i < fi; i++) {
        vp_frame *f = &g_array_index(V.frames, vp_frame, i);
        if (i == fi - 1) { *level = f->level; *bright = f->bright; }
        if (f->onset > 0) helix_pop(&V.hx, f->onset, f->bright);   /* every syllable, even in a late frame */
    }
    if (fi > V.next_frame) V.next_frame = fi;
    long sample = V.rate ? (long)gst_util_uint64_scale(pos, V.rate, GST_SECOND) : 0;
    /* A slow computer can make speech slower than it plays: rather than run
     * dry (and skip words), pause until a second more is ready. */
    double ahead = V.rate ? (double)(V.fed - sample) / V.rate : 0;
    int all = V.fed_all;
    sentence *now = NULL;
    for (guint i = 0; i < V.sentences->len; i++) {
        sentence *s = g_ptr_array_index(V.sentences, i);
        if (s->start <= sample && (sample < s->start + s->n || s->n == 0)) now = s;
    }
    char *announce = NULL;
    if (now && !now->announced) {
        now->announced = 1;
        announce = g_strdup(now->text);
    }
    g_mutex_unlock(&V.lock);
    if (!V.buffering && !all && pos > 0 && ahead < 0.12) {
        V.buffering = 1;
        V.underruns++;
        gst_element_set_state(V.play, GST_STATE_PAUSED);
    } else if (V.buffering && (all || ahead > 1.0)) {
        V.buffering = 0;
        gst_element_set_state(V.play, GST_STATE_PLAYING);
    }
    if (announce) {
        set_caption(announce);
        g_ptr_array_add(V.spoken, announce);
        event("speaking", "text", announce, NULL, NULL);
        write_state();
    }
}

/* ---------- listening ---------- */

static void warm_voice(void);

static GstFlowReturn on_rec_sample(GstAppSink *sink, gpointer gen)
{
    GstSample *smp = gst_app_sink_pull_sample(sink);
    if (!smp) return GST_FLOW_EOS;
    GstBuffer *b = gst_sample_get_buffer(smp);
    GstMapInfo mi;
    if (b && gst_buffer_map(b, &mi, GST_MAP_READ)) {
        g_mutex_lock(&V.lock);
        if (GPOINTER_TO_INT(gen) == g_atomic_int_get(&V.gen) && V.pcm->len < (guint)(LISTEN_RATE * 2 * 125)) {
            g_byte_array_append(V.pcm, mi.data, mi.size);
            vp_frame fr[256];
            int n = vp_feed(&V.rvp, (const int16_t *)mi.data, mi.size / 2, fr, 256);
            g_array_append_vals(V.rframes, fr, n);
        }
        g_mutex_unlock(&V.lock);
        gst_buffer_unmap(b, &mi);
    }
    gst_sample_unref(smp);
    return GST_FLOW_OK;
}

static gboolean on_rec_bus(GstBus *bus, GstMessage *m, gpointer gen)
{
    if (GPOINTER_TO_INT(gen) != g_atomic_int_get(&V.gen)) return G_SOURCE_REMOVE;
    if (GST_MESSAGE_TYPE(m) == GST_MESSAGE_EOS) {
        V.rec_eos = 1;   /* a file source ran out: finish at the next tick */
        return G_SOURCE_REMOVE;
    }
    if (GST_MESSAGE_TYPE(m) == GST_MESSAGE_ERROR) {
        GError *e = NULL;
        gst_message_parse_error(m, &e, NULL);
        g_free(V.error);
        V.error = g_strdup_printf("can't hear the microphone: %s", e ? e->message : "?");
        if (e) g_error_free(e);
        event("error", "error", V.error, "hint", "Check the microphone in Settings > Sound");
        stop_all(0);
        V.hx.mode = HELIX_IDLE;
        hide();
        finish_oneshot();
        return G_SOURCE_REMOVE;
    }
    return G_SOURCE_CONTINUE;
}

static void start_listening(void)
{
    stop_all(1);
    const char *src = g_getenv("NEXA_VOICE_SOURCE");
    if (!src || !*src) src = "autoaudiosrc";
    char *desc = g_strdup_printf("%s ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=%d,channels=1 "
                                 "! appsink name=sink emit-signals=true sync=false", src, LISTEN_RATE);
    GError *e = NULL;
    V.rec = gst_parse_launch(desc, &e);
    g_free(desc);
    if (e || !V.rec) {
        event("error", "error", e ? e->message : "no microphone", "hint", "Check the microphone in Settings > Sound");
        if (e) g_error_free(e);
        if (V.rec) { gst_object_unref(V.rec); V.rec = NULL; }
        finish_oneshot();
        return;
    }
    g_mutex_lock(&V.lock);
    g_byte_array_set_size(V.pcm, 0);
    g_array_set_size(V.rframes, 0);
    vp_init(&V.rvp, LISTEN_RATE);
    g_mutex_unlock(&V.lock);
    V.voiced = V.silent_run = V.rec_frames = 0;
    V.rec_eos = 0;
    int gen = g_atomic_int_get(&V.gen);
    GstElement *sink = gst_bin_get_by_name(GST_BIN(V.rec), "sink");
    g_signal_connect(sink, "new-sample", G_CALLBACK(on_rec_sample), GINT_TO_POINTER(gen));
    gst_object_unref(sink);
    GstBus *bus = gst_element_get_bus(V.rec);
    gst_bus_add_watch(bus, on_rec_bus, GINT_TO_POINTER(gen));
    gst_object_unref(bus);
    g_free(V.heard);
    V.heard = NULL;
    if (gst_element_set_state(V.rec, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE) {
        event("error", "error", "can't open the microphone", "hint", "Check the microphone in Settings > Sound");
        stop_all(0);
        finish_oneshot();
        return;
    }
    V.task = T_LISTEN;
    V.hx.mode = HELIX_LISTENING;
    set_caption("Listening\xe2\x80\xa6");
    show();
    warm_voice();
}

static gpointer warm_thread(gpointer p)
{
    int status = 0;
    FILE *f = mind_post("/v1/audio/warm", "application/json", "{}", 2, &status, NULL);
    if (f) fclose(f);
    return NULL;
}

/* You are talking to Nexa: have the voice loaded by the time it answers. */
static void warm_voice(void)
{
    g_thread_unref(g_thread_new("warm", warm_thread, NULL));
}

typedef struct { int gen; GByteArray *wav; char *text, *err, *hint; } hearing;

static gboolean on_heard(gpointer p)
{
    hearing *h = p;
    if (h->gen == g_atomic_int_get(&V.gen) && V.task == T_HEAR) {
        V.task = T_THINK;
        if (h->err) {
            g_free(V.error);
            V.error = g_strdup(h->err);
            event("error", "error", h->err, h->hint ? "hint" : NULL, h->hint);
            V.hx.mode = HELIX_IDLE;
            V.task = T_NONE;
            hide();
        } else {
            g_free(V.heard);
            V.heard = g_strdup(h->text);
            set_caption(*h->text ? h->text : "Didn't catch that");
            event("heard", "text", h->text, NULL, NULL);
            /* Nexa answers next: think until told to speak (serve), or
             * leave (listen). */
            if (!*h->text) { V.hx.mode = HELIX_IDLE; V.task = T_NONE; hide(); }
        }
        write_state();
        finish_oneshot();
    }
    g_free(h->text);
    g_free(h->err);
    g_free(h->hint);
    g_free(h);
    return G_SOURCE_REMOVE;
}

static gpointer hear_thread(gpointer p)
{
    hearing *h = p;
    int status = 0;
    FILE *f = mind_post("/v1/audio/transcriptions", "audio/wav", h->wav->data, h->wav->len, &status, NULL);
    g_byte_array_free(h->wav, TRUE);
    h->wav = NULL;
    if (!f) {
        h->err = g_strdup("Nexa Mind isn't running");
        h->hint = g_strdup("Its state is in Settings > Assistant");
    } else {
        char *body = read_all(f, 65536);
        fclose(f);
        if (status == 200) {
            struct jdoc d = {0};
            if (!json_parse(&d, body) && d.count && d.n[0].type == J_OBJ) h->text = g_strdup(json_str(&d, 0, "text", ""));
            json_free(&d);
            if (!h->text) h->err = g_strdup("Nexa Mind's answer made no sense");
        } else if (status == 409) {
            h->err = g_strdup("The speech model isn't downloaded yet");
            h->hint = g_strdup("Download it in Settings > Assistant > Voice");
        } else {
            reply_error(body, &h->err, &h->hint);
        }
        g_free(body);
    }
    g_idle_add(on_heard, h);
    return NULL;
}

static void wav_header(GByteArray *w, guint32 data)
{
    guint8 h[44];
    memcpy(h, "RIFF", 4);
    guint32 v = 36 + data;
    memcpy(h + 4, &v, 4);
    memcpy(h + 8, "WAVEfmt ", 8);
    v = 16; memcpy(h + 16, &v, 4);
    guint16 s = 1; memcpy(h + 20, &s, 2);
    s = 1; memcpy(h + 22, &s, 2);
    v = LISTEN_RATE; memcpy(h + 24, &v, 4);
    v = LISTEN_RATE * 2; memcpy(h + 28, &v, 4);
    s = 2; memcpy(h + 32, &s, 2);
    s = 16; memcpy(h + 34, &s, 2);
    memcpy(h + 36, "data", 4);
    memcpy(h + 40, &data, 4);
    g_byte_array_append(w, h, 44);
}

/* Done listening: send what was said to whisper (or say nothing was). */
static void finish_listening(void)
{
    if (V.task != T_LISTEN) return;
    if (V.rec) {
        gst_element_set_state(V.rec, GST_STATE_NULL);
        gst_object_unref(V.rec);
        V.rec = NULL;
    }
    g_mutex_lock(&V.lock);
    GByteArray *wav = g_byte_array_sized_new(V.pcm->len + 44);
    wav_header(wav, V.pcm->len);
    g_byte_array_append(wav, V.pcm->data, V.pcm->len);
    g_mutex_unlock(&V.lock);
    if (V.voiced < 25) {     /* under a quarter of a second of voice */
        g_byte_array_free(wav, TRUE);
        g_free(V.heard);
        V.heard = g_strdup("");
        set_caption("Didn't catch that");
        event("heard", "text", "", NULL, NULL);
        V.task = T_NONE;
        V.hx.mode = HELIX_IDLE;
        hide();
        finish_oneshot();
        return;
    }
    V.task = T_HEAR;
    V.hx.mode = HELIX_THINKING;
    set_caption("\xe2\x80\xa6");
    hearing *h = g_new0(hearing, 1);
    h->gen = g_atomic_int_get(&V.gen);
    h->wav = wav;
    g_thread_unref(g_thread_new("hear", hear_thread, h));
}

static void listening_now(double *level, double *onset, double *bright)
{
    g_mutex_lock(&V.lock);
    for (guint i = 0; i < V.rframes->len; i++) {
        vp_frame *f = &g_array_index(V.rframes, vp_frame, i);
        *level = f->level;
        *bright = f->bright;
        if (f->onset > 0) helix_pop(&V.hx, f->onset, f->bright);
        V.rec_frames++;
        if (f->level > 0.22) { V.voiced++; V.silent_run = 0; }
        else V.silent_run++;
    }
    g_array_set_size(V.rframes, 0);
    g_mutex_unlock(&V.lock);
    /* the end of what you said: a pause after speech, nothing at all for
     * eight seconds, the time limit, or the end of a test recording */
    if ((V.voiced >= 30 && V.silent_run >= 120) || (!V.voiced && V.rec_frames >= 800) ||
        V.rec_frames >= V.max_s * 100L || V.rec_eos)
        finish_listening();
}

/* ---------- stopping ---------- */

/* Stop whatever is going on. tell: say so ({"event":"stopped"}) if
 * speech was cut off. */
static void stop_all(int tell)
{
    int was_speaking = V.task == T_SPEAK;
    g_atomic_int_inc(&V.gen);
    g_mutex_lock(&V.lock);
    if (V.speech_fd >= 0) shutdown(V.speech_fd, SHUT_RDWR);
    V.speech_fd = -1;
    if (V.espeak_pid) kill(V.espeak_pid, SIGTERM);
    g_mutex_unlock(&V.lock);
    if (V.play) {
        gst_element_set_state(V.play, GST_STATE_NULL);
        gst_object_unref(V.play);
        V.play = NULL;
    }
    if (V.rec) {
        gst_element_set_state(V.rec, GST_STATE_NULL);
        gst_object_unref(V.rec);
        V.rec = NULL;
    }
    V.task = T_NONE;
    if (tell && was_speaking) event("stopped", NULL, NULL, NULL, NULL);
}

/* Stop pressed (nexa-voice stop, a click, {"stop":true}): listening ends
 * and is heard; speaking stops. */
static void on_stop(void)
{
    if (V.task == T_LISTEN) { finish_listening(); return; }
    if (V.task == T_SPEAK) {
        stop_all(1);
        V.hx.mode = HELIX_IDLE;
        hide();
        finish_oneshot();
        return;
    }
    if (V.task == T_THINK || V.task == T_NONE) {
        stop_all(0);
        V.hx.mode = HELIX_IDLE;
        hide();
        finish_oneshot();
    }
}

static gboolean on_usr1(gpointer d)
{
    on_stop();
    return G_SOURCE_CONTINUE;
}

static gboolean on_click(GtkWidget *w, GdkEventButton *e, gpointer d)
{
    on_stop();
    return TRUE;
}

static gboolean quit_soon(gpointer d)
{
    gtk_main_quit();
    return G_SOURCE_REMOVE;
}

/* say/listen: leave once the work is done and the helix has faded. */
static void finish_oneshot(void)
{
    if (!V.oneshot) return;
    if (V.task == T_SPEAK || V.task == T_LISTEN || V.task == T_HEAR) return;
    write_state();
    g_timeout_add(V.hx.animations ? 350 : 50, quit_soon, NULL);
    V.oneshot = 0;
}

/* ---------- drawing and the clock ---------- */

static void rounded(cairo_t *cr, double x, double y, double w, double h, double r)
{
    cairo_new_sub_path(cr);
    cairo_arc(cr, x + w - r, y + r, r, -M_PI / 2, 0);
    cairo_arc(cr, x + w - r, y + h - r, r, 0, M_PI / 2);
    cairo_arc(cr, x + r, y + h - r, r, M_PI / 2, M_PI);
    cairo_arc(cr, x + r, y + r, r, M_PI, 3 * M_PI / 2);
    cairo_close_path(cr);
}

static gboolean draw(GtkWidget *w, cairo_t *cr, gpointer d)
{
    double ww = gtk_widget_get_allocated_width(w), hh = gtk_widget_get_allocated_height(w);
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    cairo_set_source_rgba(cr, 0, 0, 0, 0);
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
    double a = V.fade;
    if (a <= 0.001) return TRUE;
    cairo_push_group(cr);
    rounded(cr, 0.5, 0.5, ww - 1, hh - 1, 22);
    cairo_set_source_rgba(cr, V.bg.r, V.bg.g, V.bg.b, V.opacity);
    cairo_fill_preserve(cr);
    cairo_set_source_rgba(cr, V.accent.r, V.accent.g, V.accent.b, 0.18 + 0.25 * V.hx.env);
    cairo_set_line_width(cr, 1);
    cairo_stroke(cr);
    cairo_save(cr);
    cairo_rectangle(cr, 0, 0, ww, HELIX_H + 8);
    cairo_clip(cr);
    helix_draw(&V.hx, cr, ww, HELIX_H + 8);
    cairo_restore(cr);
    if (V.caption[0]) {
        PangoLayout *l = gtk_widget_create_pango_layout(w, NULL);
        PangoFontDescription *fd = pango_font_description_from_string("Inter 11");
        pango_layout_set_font_description(l, fd);
        pango_font_description_free(fd);
        pango_layout_set_text(l, V.caption, -1);
        pango_layout_set_width(l, (int)((ww - 48) * PANGO_SCALE));
        pango_layout_set_alignment(l, PANGO_ALIGN_CENTER);
        pango_layout_set_ellipsize(l, PANGO_ELLIPSIZE_START);
        pango_layout_set_height(l, -2);
        int lw, lh;
        pango_layout_get_pixel_size(l, &lw, &lh);
        double ty = HELIX_H + (hh - HELIX_H - lh) / 2;
        rgb c = V.hx.mode == HELIX_SPEAKING ? V.text : V.muted;
        cairo_set_source_rgba(cr, c.r, c.g, c.b, 0.95);
        cairo_move_to(cr, 24, ty);
        pango_cairo_show_layout(cr, l);
        g_object_unref(l);
    }
    cairo_pop_group_to_source(cr);
    cairo_paint_with_alpha(cr, a);
    return TRUE;
}

static gboolean tick(gpointer d)
{
    gint64 now = g_get_monotonic_time();
    double dt = V.last_us ? (now - V.last_us) / 1e6 : 0.016;
    V.last_us = now;
    double level = 0, onset = 0, bright = 0;
    if (V.task == T_SPEAK) playing_now(&level, &onset, &bright);
    else if (V.task == T_LISTEN) listening_now(&level, &onset, &bright);
    helix_step(&V.hx, dt, level, onset, bright);
    V.frames_this_second++;
    if (now - V.second_start >= G_USEC_PER_SEC) {
        V.fps = V.second_start ? V.frames_this_second : 0;
        V.frames_this_second = 0;
        V.second_start = now;
    }
    if (V.fade != V.fade_to) {
        double step = dt / 0.22;
        V.fade = V.fade_to > V.fade ? fmin(V.fade_to, V.fade + step) : fmax(V.fade_to, V.fade - step);
        if (V.fade <= 0 && V.fade_to <= 0 && V.shown) {
            V.shown = 0;
            gtk_widget_hide(V.win);
        }
    }
    if (V.shown) gtk_widget_queue_draw(V.area);
    static gint64 last_state;
    if (now - last_state > 100000) {
        last_state = now;
        write_state();
    }
    if (!V.shown && V.task == T_NONE) {
        write_state();
        tick_id = 0;
        return G_SOURCE_REMOVE;
    }
    return G_SOURCE_CONTINUE;
}

/* ---------- serve: commands on stdin ---------- */

static gboolean on_stdin(GIOChannel *ch, GIOCondition c, gpointer d)
{
    char *line = NULL;
    gsize len = 0;
    GIOStatus st = g_io_channel_read_line(ch, &line, &len, NULL, NULL);
    if (st == G_IO_STATUS_EOF || st == G_IO_STATUS_ERROR) {
        stop_all(0);
        gtk_main_quit();
        return G_SOURCE_REMOVE;
    }
    if (st != G_IO_STATUS_NORMAL || !line) return G_SOURCE_CONTINUE;
    struct jdoc doc = {0};
    if (len > MAX_TEXT * 2 || json_parse(&doc, line) || !doc.count || doc.n[0].type != J_OBJ) {
        event("error", "error", "not a command", NULL, NULL);
    } else if (json_get(&doc, 0, "say") >= 0) {
        const char *t = json_str(&doc, 0, "say", "");
        if (*t) start_speaking(t, json_str(&doc, 0, "voice", NULL), json_num(&doc, 0, "speed", 0));
    } else if (json_get(&doc, 0, "listen") >= 0) {
        V.max_s = (int)json_num(&doc, 0, "max", 30);
        if (V.max_s < 2 || V.max_s > 120) V.max_s = 30;
        start_listening();
    } else if (json_get(&doc, 0, "think") >= 0) {
        stop_all(1);
        V.task = T_THINK;
        V.hx.mode = HELIX_THINKING;
        set_caption("");
        show();
        warm_voice();
    } else if (json_get(&doc, 0, "stop") >= 0) {
        on_stop();
    } else if (json_get(&doc, 0, "cancel") >= 0 || json_get(&doc, 0, "hide") >= 0) {
        stop_all(1);
        V.hx.mode = HELIX_IDLE;
        hide();
    } else {
        event("error", "error", "unknown command", NULL, NULL);
    }
    json_free(&doc);
    g_free(line);
    write_state();
    return G_SOURCE_CONTINUE;
}

/* ---------- start ---------- */

static void pid_path(char *out, size_t cap)
{
    const char *rt = g_getenv("XDG_RUNTIME_DIR");
    snprintf(out, cap, "%s/nexa-voice.pid", rt && *rt ? rt : g_get_tmp_dir());
}

static int stop_running(void)
{
    char path[600], *t = NULL;
    pid_path(path, sizeof path);
    if (!g_file_get_contents(path, &t, NULL, NULL)) return 1;
    long pid = strtol(t, NULL, 10);
    g_free(t);
    if (pid <= 1) return 1;
    char comm[64] = "", cpath[64];
    snprintf(cpath, sizeof cpath, "/proc/%ld/comm", pid);
    FILE *f = fopen(cpath, "r");
    if (f) { if (!fgets(comm, sizeof comm, f)) comm[0] = 0; fclose(f); }
    if (strncmp(comm, "nexa-voice", 10)) return 1;   /* gone, or someone else now */
    return kill((pid_t)pid, SIGUSR1) ? 1 : 0;
}

static void usage(void)
{
    fprintf(stderr, "usage: nexa-voice say [--voice NAME] [--speed X] [TEXT...]\n"
                    "       nexa-voice listen [--max SECONDS]\n"
                    "       nexa-voice serve | stop\n");
    exit(2);
}

static void on_anim(GObject *o, GParamSpec *p, gpointer d) { read_animations(); }

static void on_file(GFileMonitor *m, GFile *f, GFile *o, GFileMonitorEvent e, gpointer which)
{
    if (e != G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT && e != G_FILE_MONITOR_EVENT_CREATED) return;
    if (GPOINTER_TO_INT(which) == 1) load_theme(); else load_conf();
}

static GFileMonitor *watch(const char *path, int which)
{
    GFile *f = g_file_new_for_path(path);
    GFileMonitor *m = g_file_monitor_file(f, G_FILE_MONITOR_NONE, NULL, NULL);
    g_object_unref(f);
    if (m) g_signal_connect(m, "changed", G_CALLBACK(on_file), GINT_TO_POINTER(which));
    return m;
}

int main(int argc, char **argv)
{
    if (argc < 2) usage();
    const char *cmd = argv[1];
    if (!strcmp(cmd, "stop")) return stop_running();
    if (strcmp(cmd, "say") && strcmp(cmd, "listen") && strcmp(cmd, "serve")) usage();
    char voice[40] = "";
    double speed = 0;
    V.max_s = 30;
    GString *text = g_string_new(NULL);
    for (int i = 2; i < argc; i++) {
        if (!strcmp(argv[i], "--voice") && i + 1 < argc) snprintf(voice, sizeof voice, "%s", argv[++i]);
        else if (!strcmp(argv[i], "--speed") && i + 1 < argc) speed = g_ascii_strtod(argv[++i], NULL);
        else if (!strcmp(argv[i], "--max") && i + 1 < argc) V.max_s = atoi(argv[++i]);
        else if (!strcmp(cmd, "say")) { if (text->len) g_string_append_c(text, ' '); g_string_append(text, argv[i]); }
        else usage();
    }
    if (V.max_s < 2 || V.max_s > 120) V.max_s = 30;
    if (!strcmp(cmd, "say") && !text->len) {
        char buf[4096];
        size_t n;
        while ((n = fread(buf, 1, sizeof buf, stdin)) > 0 && text->len < MAX_TEXT) g_string_append_len(text, buf, n);
    }
    if (!strcmp(cmd, "say") && !g_strstrip(text->str)[0]) { fprintf(stderr, "nexa-voice: nothing to say\n"); return 2; }

    gst_init(&argc, &argv);
    gtk_init(&argc, &argv);
    g_mutex_init(&V.lock);
    V.frames = g_array_new(FALSE, FALSE, sizeof(vp_frame));
    V.rframes = g_array_new(FALSE, FALSE, sizeof(vp_frame));
    V.sentences = g_ptr_array_new();
    V.spoken = g_ptr_array_new_with_free_func(g_free);
    V.pcm = g_byte_array_new();
    V.speech_fd = -1;
    helix_init(&V.hx, 210, (unsigned)g_get_monotonic_time());
    config_path(V.theme_path, sizeof V.theme_path, "lock.conf");
    config_path(V.conf_path, sizeof V.conf_path, "voice.conf");
    load_theme();
    load_conf();
    read_animations();

    char pidf[600];
    pid_path(pidf, sizeof pidf);
    char pidt[32];
    snprintf(pidt, sizeof pidt, "%ld\n", (long)getpid());
    g_file_set_contents(pidf, pidt, -1, NULL);

    V.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(V.win), "Nexa voice");
    gtk_window_set_default_size(GTK_WINDOW(V.win), W, H);
    GdkVisual *vis = gdk_screen_get_rgba_visual(gtk_widget_get_screen(V.win));
    if (vis) gtk_widget_set_visual(V.win, vis);
    gtk_widget_set_app_paintable(V.win, TRUE);
    if (gtk_layer_is_supported()) {
        gtk_layer_init_for_window(GTK_WINDOW(V.win));
        gtk_layer_set_namespace(GTK_WINDOW(V.win), "nexa-voice");   /* Hyprland blurs nexa-* layers */
        gtk_layer_set_layer(GTK_WINDOW(V.win), GTK_LAYER_SHELL_LAYER_OVERLAY);
        gtk_layer_set_anchor(GTK_WINDOW(V.win), GTK_LAYER_SHELL_EDGE_BOTTOM, TRUE);
        gtk_layer_set_margin(GTK_WINDOW(V.win), GTK_LAYER_SHELL_EDGE_BOTTOM, 96);
        gtk_layer_set_keyboard_mode(GTK_WINDOW(V.win), GTK_LAYER_SHELL_KEYBOARD_MODE_NONE);
    }
    V.area = gtk_drawing_area_new();
    gtk_widget_set_size_request(V.area, W, H);
    gtk_widget_add_events(V.area, GDK_BUTTON_PRESS_MASK);
    g_signal_connect(V.area, "draw", G_CALLBACK(draw), NULL);
    g_signal_connect(V.area, "button-press-event", G_CALLBACK(on_click), NULL);
    g_signal_connect(V.win, "destroy", G_CALLBACK(gtk_main_quit), NULL);
    gtk_container_add(GTK_CONTAINER(V.win), V.area);

    GFileMonitor *m1 = watch(V.theme_path, 1), *m2 = watch(V.conf_path, 2);
    g_signal_connect(gtk_settings_get_default(), "notify::gtk-enable-animations", G_CALLBACK(on_anim), NULL);
    g_unix_signal_add(SIGUSR1, on_usr1, NULL);

    if (!strcmp(cmd, "serve")) {
        V.serve = 1;
        GIOChannel *ch = g_io_channel_unix_new(0);
        g_io_add_watch(ch, G_IO_IN | G_IO_HUP | G_IO_ERR, on_stdin, NULL);
        event("ready", NULL, NULL, NULL, NULL);
    } else if (!strcmp(cmd, "say")) {
        V.oneshot = 1;
        start_speaking(text->str, voice[0] ? voice : NULL, speed);
    } else {
        V.oneshot = 1;
        start_listening();
    }
    g_string_free(text, TRUE);
    write_state();
    gtk_main();
    stop_all(0);
    write_state();
    /* the pid file is ours only if no newer nexa-voice took it */
    char *t = NULL;
    if (g_file_get_contents(pidf, &t, NULL, NULL) && !strcmp(t, pidt)) unlink(pidf);
    g_free(t);
    if (m1) g_object_unref(m1);
    if (m2) g_object_unref(m2);
    return 0;
}
