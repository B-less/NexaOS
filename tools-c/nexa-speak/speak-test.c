/* Unit tests for kokoro.c: cleaning, pieces, espeak to misaki, tokens. */
#include "kokoro.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails, checks;

#define CHECK(cond, what) do { checks++; if (cond) printf("PASS %s\n", what); \
    else { fails++; printf("FAIL %s (line %d)\n", what, __LINE__); } } while (0)

static int same(char *got, const char *want)
{
    int ok = !strcmp(got, want);
    if (!ok) printf("  got:  [%s]\n  want: [%s]\n", got, want);
    free(got);
    return ok;
}

static void cleaning(void)
{
    CHECK(same(kokoro_clean("Hello **there**, `x` and _y_."), "Hello there, x and y."), "markdown marks dropped");
    CHECK(same(kokoro_clean("# Title\n- one\n- two\n"), "Title. one. two."), "lines end like sentences");
    CHECK(same(kokoro_clean("Run this:\n```\nrm -rf /\n```\nDone"), "Run this: There's some code here. Done."), "code isn't read out");
    CHECK(same(kokoro_clean("See https://nexa.os/x?a=1 now"), "See a link now."), "links");
    CHECK(same(kokoro_clean("Read [the guide](https://x.y) first."), "Read the guide first."), "link text kept");
    CHECK(same(kokoro_clean("  lots   of\t space  "), "lots of space."), "spaces collapsed");
    CHECK(same(kokoro_clean(""), ""), "empty");
    CHECK(same(kokoro_clean("Wait... what?"), "Wait\xe2\x80\xa6 what?"), "three dots are one ellipsis");
}

static void pieces(void)
{
    kokoro_piece *p;
    int n = kokoro_pieces("Hello, I'm Nexa. It's 6:30 and 3.5 degrees; Mr. Smith called! Ok?! Fine", &p);
    CHECK(n == 6, "six pieces");
    if (n == 6) {
        CHECK(!strcmp(p[0].words, "Hello") && !strcmp(p[0].punct, ",") && !p[0].sentence_end, "a comma doesn't end a sentence");
        CHECK(!strcmp(p[1].words, "I'm Nexa") && p[1].sentence_end, "a full stop does");
        CHECK(!strcmp(p[2].words, "It's 6:30 and 3.5 degrees") && !strcmp(p[2].punct, ";"), "times and decimals stay whole");
        CHECK(!strcmp(p[3].words, "Mr. Smith called") && !strcmp(p[3].punct, "!"), "Mr. doesn't cut");
        CHECK(!strcmp(p[4].words, "Ok") && !strcmp(p[4].punct, "?!"), "marks together");
        CHECK(!strcmp(p[5].words, "Fine") && !strcmp(p[5].punct, "") && p[5].sentence_end, "the end ends a sentence");
    }
    kokoro_pieces_free(p, n);
    n = kokoro_pieces("", &p);
    CHECK(n == 0, "nothing to say");
    kokoro_pieces_free(p, n);
    n = kokoro_pieces("Well\xe2\x80\x94maybe\xe2\x80\xa6 Yes.", &p);
    CHECK(n == 3 && !strcmp(p[0].punct, "\xe2\x80\x94") && p[1].sentence_end, "dash and ellipsis");
    kokoro_pieces_free(p, n);
}

static void misaki(void)
{
    /* real espeak-ng 1.52 output (--ipa --tie=^ -v en-us) */
    CHECK(same(kokoro_from_espeak("h\xc9\x99l\xcb\x88o^\xca\x8a", 0), "h\xc9\x99l\xcb\x88O"), "hello: o^ʊ is O");
    CHECK(same(kokoro_from_espeak("a^\xc9\xaa\xc9\x90m n\xcb\x88\xc9\x9bks\xc9\x99", 0), "I\xc9\x99m n\xcb\x88\xc9\x9bks\xc9\x99"), "I am Nexa: aɪ is I, ɐ is ə");
    CHECK(same(kokoro_from_espeak("m\xcb\x88" "e^\xc9\xaal", 0), "m\xcb\x88" "Al"), "mail: eɪ is A");
    CHECK(same(kokoro_from_espeak("\xc3\xb0\xc9\x99 t^\xca\x83\xcb\x88\xc9\x9c\xcb\x90t^\xca\x83 d^\xca\x92\xcb\x88\xca\x8c" "d^\xca\x92", 0),
               "\xc3\xb0\xc9\x99 \xca\xa7\xcb\x88\xc9\x9c\xc9\xb9\xca\xa7 \xca\xa4\xcb\x88\xca\x8c\xca\xa4"), "church judge: ʧ ʤ, ɜː is ɜɹ");
    CHECK(same(kokoro_from_espeak("b\xcb\x88\xca\x8c\xca\x94n\xcc\xa9", 0), "b\xcb\x88\xca\x8ct\xe1\xb5\x8an"), "button: ʔn̩ is tᵊn");
    CHECK(same(kokoro_from_espeak("m\xcb\x88\xc9\xaast\xc9\x9a\nsm\xcb\x88\xc9\xaa\xce\xb8", 0), "m\xcb\x88\xc9\xaast\xc9\x99\xc9\xb9 sm\xcb\x88\xc9\xaa\xce\xb8"), "clauses joined, ɚ is əɹ");
    CHECK(same(kokoro_from_espeak("f\xcb\x8c\xc9\x94\xcb\x90\xc9\xb9\xc9\x9b\xc9\xa1z\xcb\x88\xc3\xa6mp\xc9\x99^l", 0),
               "f\xcb\x8c\xc9\x94\xc9\xb9\xc9\x9b\xc9\xa1z\xcb\x88\xc3\xa6mp\xe1\xb5\x8al"), "for example: ə^l is ᵊl, no length marks");
    CHECK(same(kokoro_from_espeak("h\xc9\x99l\xcb\x88\xc9\x99^\xca\x8a", 1), "h\xc9\x99l\xcb\x88Q"), "British hello: əʊ is Q");
    CHECK(same(kokoro_from_espeak("k\xcb\x88\xc9\x91\xcb\x90", 1), "k\xcb\x88\xc9\x91\xcb\x90"), "British keeps length marks");
    CHECK(same(kokoro_from_espeak("", 0), ""), "empty");
}

static void tokens(void)
{
    int64_t ids[16];
    int n = kokoro_ids("h\xc9\x99l\xcb\x88O, w\xcb\x88\xc9\x9cld!", ids, 16);
    int64_t want[] = {50, 83, 54, 156, 31, 3, 16, 65, 156, 87, 54, 46, 5};
    CHECK(n == 13 && !memcmp(ids, want, sizeof want), "ids from the vocabulary");
    CHECK(kokoro_ids("h\xe2\x98\x83" "a\xff", ids, 16) == 2, "unknown and broken characters dropped");
    CHECK(kokoro_ids("aaaa", ids, 2) == 4 && ids[1] == 43, "counts past the cap, writes only the cap");
    CHECK(kokoro_cut("ab cd ef", 4) == 2, "cut at the last space that fits");
    CHECK(kokoro_cut("ab cd", 10) == 5, "fits whole");
    CHECK(kokoro_cut("abcdef", 3) == 3, "no space: cut anyway");
}

int main(void)
{
    cleaning();
    pieces();
    misaki();
    tokens();
    printf("%d/%d passed\n", checks - fails, checks);
    return fails != 0;
}
