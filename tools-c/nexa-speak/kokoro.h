/* Text to Kokoro tokens: the part of nexa-speak with no model in it.
 *
 * Kokoro (Kokoro-82M v1.0, Apache-2.0) reads phonemes in the alphabet of
 * misaki, its own English front end. espeak-ng gives IPA; kokoro_from_espeak
 * converts it the way misaki's espeak fallback does, so the voice hears what
 * it was trained on. espeak-ng runs as a separate program (it is GPL-3; NexaOS
 * code is MIT and never links it). */
#ifndef NEXA_KOKORO_H
#define NEXA_KOKORO_H

#include <stddef.h>
#include <stdint.h>

#define KOKORO_MAX_TOKENS 510   /* the model's limit, pads not counted */
#define KOKORO_CHUNK_TOKENS 400 /* where a long sentence is cut */
#define KOKORO_PAUSE_TOKENS 60  /* a chunk this long ends at the next pause */
#define KOKORO_RATE 24000

/* Text from an answer made speakable: code blocks dropped, Markdown marks
 * and links removed, each line ended like a sentence, spaces collapsed.
 * Returns a new string. */
char *kokoro_clean(const char *in);

/* A stretch of words up to a punctuation mark (or the end). */
typedef struct {
    char *words;        /* without the mark */
    char punct[8];      /* ",", ".", "?", "—" ... or "" */
    int sentence_end;   /* . ! ? (or the text's end): a chunk ends here */
} kokoro_piece;

/* Cut clean text into pieces; returns how many (*out to free with
 * kokoro_pieces_free). Abbreviations like "Mr." and "e.g." don't cut. */
int kokoro_pieces(const char *clean, kokoro_piece **out);
void kokoro_pieces_free(kokoro_piece *p, int n);

/* espeak-ng's IPA (made with --ipa --tie=^) in misaki's alphabet.
 * british: en-gb rules, else American. Returns a new string. */
char *kokoro_from_espeak(const char *ipa, int british);

/* Phonemes to token ids (no pads; characters outside the vocabulary are
 * dropped). Writes at most cap ids; returns how many there are in all. */
int kokoro_ids(const char *phonemes, int64_t *ids, int cap);

/* Byte offset in s after cutting at most max_tokens tokens at a space
 * (for a piece too long for the model). Returns strlen(s) if it fits. */
size_t kokoro_cut(const char *s, int max_tokens);
#endif
