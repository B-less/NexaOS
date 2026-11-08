/* nexa-speak: Nexa Mind's voice. Kokoro-82M (Apache-2.0) through
 * onnxruntime, on the CPU, offline. nexa-mindd runs it as a child (the
 * nexa-mind user, under the service's Landlock rules) and keeps it loaded
 * while you talk; it is never started by apps.
 *
 *   nexa-speak --model FILE --voices PREFIX [--threads N] [--espeak PATH]
 *   nexa-speak --phonemes [--british]       text on stdin, phonemes out (tests)
 *
 * Requests on stdin, one after another:
 *   <voice> <speed> <bytes>\n<text>
 * voice: a name (af_heart -> PREFIXaf_heart.bin, 510 x 256 floats; a* is
 * American English, b* British), speed 0.5 to 2. nexa-mindd passes
 * --voices /var/lib/nexa-mind/models/kokoro-. Answer on stdout, sentence by sentence as
 * each is made (so speech starts before the rest is ready):
 *   C <rate> <samples> <text bytes>\n<text><samples x int16 LE, mono>
 *   ...
 *   END\n
 * or "ERR <message>\n" in place of END. The same framing goes to the
 * desktop unchanged (nexa-mindd POST /v1/audio/speech).
 *
 * Words become phonemes with espeak-ng, run as its own program per piece
 * of text (see kokoro.h). */
#include "kokoro.h"

#include <errno.h>
#include <fcntl.h>
#include <onnxruntime/onnxruntime_c_api.h>
#include <poll.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

#define MAX_TEXT (64 * 1024)
#define STYLE 256
#define VOICE_ROWS 510

static const OrtApi *ort;
static OrtSession *sess;
static OrtMemoryInfo *mem;
static char *in_names[3], *out_name;
static int in_kind[3];          /* 0 ids, 1 style, 2 speed */
static ONNXTensorElementDataType in_type[3];
static size_t n_in;
static const char *espeak = "/usr/bin/espeak-ng";
static const char *voices_dir;
static int timing;   /* NEXA_SPEAK_TIMING=1: how long each step took, on stderr */

static double now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static void die(const char *what, const char *why)
{
    fprintf(stderr, "nexa-speak: %s: %s\n", what, why);
    exit(1);
}

static void check(OrtStatus *st, const char *what)
{
    if (!st) return;
    die(what, ort->GetErrorMessage(st));
}

/* ---------- espeak-ng ---------- */

/* Phonemes for one piece of text; NULL if espeak-ng failed. */
static char *phonemize(const char *text, int british)
{
    int in[2], out[2];
    if (pipe2(in, O_CLOEXEC) || pipe2(out, O_CLOEXEC)) return NULL;
    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_adddup2(&fa, in[0], 0);
    posix_spawn_file_actions_adddup2(&fa, out[1], 1);
    char *argv[] = {(char *)espeak, "-q", "--ipa", "--tie=^", "-v", british ? "en-gb" : "en-us", NULL};
    pid_t pid;
    int rc = posix_spawn(&pid, espeak, &fa, NULL, argv, environ);
    posix_spawn_file_actions_destroy(&fa);
    close(in[0]);
    close(out[1]);
    if (rc) { close(in[1]); close(out[0]); return NULL; }
    /* pieces are short (a clause); write it all, then read */
    size_t len = strlen(text);
    const char *p = text;
    while (len) {
        ssize_t w = write(in[1], p, len);
        if (w < 0) { if (errno == EINTR) continue; break; }
        p += w;
        len -= w;
    }
    if (write(in[1], "\n", 1) < 0) {}
    close(in[1]);
    size_t cap = 4096, n = 0;
    char *res = malloc(cap);
    for (;;) {
        if (n + 1024 > cap) res = realloc(res, cap *= 2);
        ssize_t r = read(out[0], res + n, cap - n - 1);
        if (r < 0 && errno == EINTR) continue;
        if (r <= 0) break;
        n += r;
    }
    close(out[0]);
    res[n] = 0;
    int status;
    while (waitpid(pid, &status, 0) < 0 && errno == EINTR) {}
    if (!WIFEXITED(status) || WEXITSTATUS(status)) { free(res); return NULL; }
    return res;
}

/* ---------- the model ---------- */

static void load_model(const char *path, int threads)
{
    ort = OrtGetApiBase()->GetApi(ORT_API_VERSION);
    if (!ort) die("onnxruntime", "API version not supported");
    OrtEnv *env;
    check(ort->CreateEnv(ORT_LOGGING_LEVEL_ERROR, "nexa-speak", &env), "onnxruntime");
    OrtSessionOptions *so;
    check(ort->CreateSessionOptions(&so), "options");
    check(ort->SetIntraOpNumThreads(so, threads), "threads");
    check(ort->SetInterOpNumThreads(so, 1), "threads");
    check(ort->SetSessionGraphOptimizationLevel(so, ORT_ENABLE_ALL), "options");
    check(ort->CreateSession(env, path, so, &sess), path);
    check(ort->CreateCpuMemoryInfo(OrtArenaAllocator, OrtMemTypeDefault, &mem), "memory");
    OrtAllocator *al;
    check(ort->GetAllocatorWithDefaultOptions(&al), "allocator");
    check(ort->SessionGetInputCount(sess, &n_in), "inputs");
    if (n_in != 3) die(path, "not a Kokoro model (expected 3 inputs)");
    int seen = 0;
    for (size_t i = 0; i < n_in; i++) {
        check(ort->SessionGetInputName(sess, i, al, &in_names[i]), "input");
        const char *nm = in_names[i];
        in_kind[i] = strstr(nm, "style") ? 1 : strstr(nm, "speed") ? 2 : 0;
        seen |= 1 << in_kind[i];
        OrtTypeInfo *ti;
        const OrtTensorTypeAndShapeInfo *tsi;
        check(ort->SessionGetInputTypeInfo(sess, i, &ti), "input");
        check(ort->CastTypeInfoToTensorInfo(ti, &tsi), "input");
        check(ort->GetTensorElementType(tsi, &in_type[i]), "input");
        ort->ReleaseTypeInfo(ti);
    }
    if (seen != 7) die(path, "not a Kokoro model (inputs: tokens, style, speed)");
    check(ort->SessionGetOutputName(sess, 0, al, &out_name), "output");
}

typedef struct { char name[40]; float *rows; } voice;
static voice cache[4];

static const float *voice_rows(const char *name)
{
    for (int i = 0; i < 4; i++)
        if (cache[i].rows && !strcmp(cache[i].name, name)) return cache[i].rows;
    char path[4096];
    snprintf(path, sizeof path, "%s%s.bin", voices_dir, name);
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    float *rows = malloc(sizeof(float) * STYLE * VOICE_ROWS);
    size_t got = fread(rows, sizeof(float), STYLE * VOICE_ROWS, f);
    int extra = fgetc(f);
    fclose(f);
    if (got != STYLE * VOICE_ROWS || extra != EOF) { free(rows); return NULL; }
    int slot = 0;
    for (int i = 0; i < 4; i++) if (!cache[i].rows) { slot = i; break; }
    free(cache[slot].rows);
    snprintf(cache[slot].name, sizeof cache[slot].name, "%s", name);
    cache[slot].rows = rows;
    return rows;
}

/* Run the model on one chunk; writes its frame. 0 on success. */
static int speak_chunk(const char *phonemes, const char *text, const float *rows, float speed)
{
    int n = kokoro_ids(phonemes, NULL, 0);
    if (n == 0) return 0;
    if (n > KOKORO_MAX_TOKENS) n = KOKORO_MAX_TOKENS;
    int64_t *ids = calloc(n + 2, sizeof *ids);
    kokoro_ids(phonemes, ids + 1, n);   /* pads (0) at both ends */
    float style[STYLE];
    memcpy(style, rows + (size_t)n * STYLE, sizeof style);
    int32_t speed_i = 1;   /* exports that take a whole number */
    OrtValue *in[3] = {0};
    const char *names[3];
    int64_t ids_shape[2] = {1, n + 2}, style_shape[2] = {1, STYLE}, one[1] = {1};
    int rc = 0;
    for (size_t i = 0; i < n_in; i++) {
        names[i] = in_names[i];
        OrtStatus *st;
        if (in_kind[i] == 0)
            st = ort->CreateTensorWithDataAsOrtValue(mem, ids, sizeof *ids * (n + 2), ids_shape, 2, ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64, &in[i]);
        else if (in_kind[i] == 1)
            st = ort->CreateTensorWithDataAsOrtValue(mem, style, sizeof style, style_shape, 2, ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT, &in[i]);
        else if (in_type[i] == ONNX_TENSOR_ELEMENT_DATA_TYPE_INT32)
            st = ort->CreateTensorWithDataAsOrtValue(mem, &speed_i, sizeof speed_i, one, 1, ONNX_TENSOR_ELEMENT_DATA_TYPE_INT32, &in[i]);
        else
            st = ort->CreateTensorWithDataAsOrtValue(mem, &speed, sizeof speed, one, 1, ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT, &in[i]);
        if (st) { fprintf(stderr, "nexa-speak: %s\n", ort->GetErrorMessage(st)); ort->ReleaseStatus(st); rc = -1; }
    }
    OrtValue *out = NULL;
    if (!rc) {
        const char *on[1] = {out_name};
        double t0 = now_ms();
        OrtStatus *st = ort->Run(sess, NULL, names, (const OrtValue *const *)in, n_in, on, 1, &out);
        if (st) { fprintf(stderr, "nexa-speak: %s\n", ort->GetErrorMessage(st)); ort->ReleaseStatus(st); rc = -1; }
        if (timing) fprintf(stderr, "nexa-speak: %d tokens in %.0f ms\n", n, now_ms() - t0);
    }
    if (!rc) {
        float *wave = NULL;
        OrtTensorTypeAndShapeInfo *info = NULL;
        size_t count = 0;
        OrtStatus *st = ort->GetTensorMutableData(out, (void **)&wave);
        if (!st) st = ort->GetTensorTypeAndShape(out, &info);
        if (!st) st = ort->GetTensorShapeElementCount(info, &count);
        if (info) ort->ReleaseTensorTypeAndShapeInfo(info);
        if (st) { ort->ReleaseStatus(st); count = 0; }
        int16_t *pcm = malloc(count * sizeof *pcm + 1);
        for (size_t i = 0; i < count; i++) {
            float v = wave[i] * 32767.0f;
            pcm[i] = v > 32767 ? 32767 : v < -32768 ? -32768 : (int16_t)v;
        }
        size_t tl = strlen(text);
        printf("C %d %zu %zu\n", KOKORO_RATE, count, tl);
        fwrite(text, 1, tl, stdout);
        fwrite(pcm, sizeof *pcm, count, stdout);
        fflush(stdout);
        free(pcm);
    }
    if (out) ort->ReleaseValue(out);
    for (size_t i = 0; i < n_in; i++) if (in[i]) ort->ReleaseValue(in[i]);
    free(ids);
    return rc;
}

/* ---------- requests ---------- */

typedef struct { char *ph; size_t phn; char *tx; size_t txn; } chunk;

static void add(char **s, size_t *n, const char *t)
{
    size_t l = strlen(t);
    *s = realloc(*s, *n + l + 1);
    memcpy(*s + *n, t, l + 1);
    *n += l;
}

/* Speak a finished chunk (or, for --phonemes, print it) and empty it. */
static const char *emit(chunk *c, const float *rows, float speed, int dry)
{
    const char *err = NULL;
    if (c->phn) {
        if (dry) printf("%s\t%s\n", c->txn ? c->tx : "", c->ph);
        else if (speak_chunk(c->ph, c->txn ? c->tx : "", rows, speed)) err = "the voice model failed";
    }
    free(c->ph);
    free(c->tx);
    memset(c, 0, sizeof *c);
    return err;
}

/* Text in pieces -> chunks of whole sentences (or as much as fits). */
static const char *speak_text(const char *text, const float *rows, int british, float speed, int dry)
{
    char *clean = kokoro_clean(text);
    kokoro_piece *pc;
    int n = kokoro_pieces(clean, &pc);
    chunk c = {0};
    const char *err = NULL;
    int first = 1;
    for (int i = 0; i < n && !err; i++) {
        char *ipa = phonemize(pc[i].words, british);
        if (!ipa) { err = "espeak-ng failed"; break; }
        char *ph = kokoro_from_espeak(ipa, british);
        free(ipa);
        /* a chunk that can't take this piece is spoken first */
        if (c.phn && kokoro_ids(c.ph, NULL, 0) + kokoro_ids(ph, NULL, 0) + 2 > KOKORO_CHUNK_TOKENS)
            err = emit(&c, rows, speed, dry);
        if (c.txn) add(&c.tx, &c.txn, " ");
        add(&c.tx, &c.txn, pc[i].words);
        /* a piece too long for the model is cut at spaces */
        char *rest = ph;
        while (*rest && !err) {
            size_t cut = kokoro_cut(rest, KOKORO_CHUNK_TOKENS);
            char save = rest[cut];
            rest[cut] = 0;
            if (c.phn) add(&c.ph, &c.phn, " ");
            add(&c.ph, &c.phn, rest);
            rest[cut] = save;
            rest += cut;
            while (*rest == ' ') rest++;
            if (*rest) err = emit(&c, rows, speed, dry);
        }
        free(ph);
        add(&c.ph, &c.phn, pc[i].punct);
        add(&c.tx, &c.txn, pc[i].punct);
        /* Sentences are spoken whole, but the first chunk ends at its first
         * pause so speech starts sooner, and a long sentence is cut at a
         * pause: on a laptop CPU Kokoro takes about as long as the audio
         * lasts, so short chunks keep the next one ready in time. */
        int at_pause = pc[i].punct[0] != 0;
        if (!err && (pc[i].sentence_end || (at_pause && (first || kokoro_ids(c.ph, NULL, 0) >= KOKORO_PAUSE_TOKENS)))) {
            err = emit(&c, rows, speed, dry);
            first = 0;
        }
    }
    if (!err) err = emit(&c, rows, speed, dry);
    free(c.ph);
    free(c.tx);
    kokoro_pieces_free(pc, n);
    free(clean);
    return err;
}

static int voice_name_ok(const char *v)
{
    size_t n = strlen(v);
    if (n < 3 || n > 32 || v[2] != '_') return 0;
    for (const char *p = v; *p; p++)
        if (!((*p >= 'a' && *p <= 'z') || (*p >= '0' && *p <= '9') || *p == '_')) return 0;
    return 1;
}

int main(int argc, char **argv)
{
    const char *model = NULL;
    int threads = 4, dry = 0, british = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--model") && i + 1 < argc) model = argv[++i];
        else if (!strcmp(argv[i], "--voices") && i + 1 < argc) voices_dir = argv[++i];
        else if (!strcmp(argv[i], "--threads") && i + 1 < argc) threads = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--espeak") && i + 1 < argc) espeak = argv[++i];
        else if (!strcmp(argv[i], "--phonemes")) dry = 1;
        else if (!strcmp(argv[i], "--british")) british = 1;
        else {
            fprintf(stderr, "usage: nexa-speak --model FILE --voices PREFIX [--threads N] [--espeak PATH]\n"
                            "       nexa-speak --phonemes [--british] < text\n");
            return 2;
        }
    }
    if (threads < 1 || threads > 64) threads = 4;
    if (dry) {
        static char text[MAX_TEXT + 1];
        size_t n = fread(text, 1, MAX_TEXT, stdin);
        text[n] = 0;
        const char *err = speak_text(text, NULL, british, 1, 1);
        if (err) die("phonemes", err);
        return 0;
    }
    if (!model || !voices_dir) die("usage", "--model and --voices are needed");
    timing = getenv("NEXA_SPEAK_TIMING") != NULL;
    double t0 = now_ms();
    load_model(model, threads);
    if (timing) fprintf(stderr, "nexa-speak: model loaded in %.0f ms\n", now_ms() - t0);
    fprintf(stderr, "nexa-speak: ready (%d threads)\n", threads);
    char line[256];
    static char text[MAX_TEXT + 1];
    while (fgets(line, sizeof line, stdin)) {
        char vname[64];
        float speed;
        size_t bytes;
        if (sscanf(line, "%63s %f %zu", vname, &speed, &bytes) != 3 || bytes > MAX_TEXT) {
            printf("ERR bad request\n");
            fflush(stdout);
            return 1;   /* the stream is out of step: start again */
        }
        size_t got = fread(text, 1, bytes, stdin);
        text[got] = 0;
        if (got != bytes) return 1;
        if (speed < 0.5f || speed > 2.0f) speed = 1.0f;
        const float *rows = voice_name_ok(vname) ? voice_rows(vname) : NULL;
        if (!rows) {
            printf("ERR no voice named %s\n", vname);
            fflush(stdout);
            continue;
        }
        const char *err = speak_text(text, rows, vname[0] == 'b', speed, 0);
        if (err) printf("ERR %s\n", err);
        else printf("END\n");
        fflush(stdout);
    }
    return 0;
}
