/* Backup's engine on real folders (make test): what's left out, a first
 * backup, a second that copies only changes, putting back (beside a
 * changed file, into a folder), checking, damage, stopping, a hostile
 * backup list, thinning out old backups and tidying the drive. */
#include <glib/gstdio.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <utime.h>

#include "../src/engine.h"

static int fails, checks;

static void check(int ok, const char *what)
{
    checks++;
    if (!ok) fails++;
    printf("%s %s\n", ok ? "PASS" : "FAIL", what);
}

static void put(const char *base, const char *rel, const char *text, gsize n)
{
    char *p = g_build_filename(base, rel, NULL), *d = g_path_get_dirname(p);
    g_mkdir_with_parents(d, 0700);
    g_file_set_contents(p, text, (gssize)n, NULL);
    g_free(p);
    g_free(d);
}

static char *get(const char *base, const char *rel)
{
    char *p = g_build_filename(base, rel, NULL), *t = NULL;
    g_file_get_contents(p, &t, NULL, NULL);
    g_free(p);
    return t;
}

static int has(Snap *s, const char *rel)
{
    for (guint i = 0; i < s->items->len; i++)
        if (!strcmp(((Item *)s->items->pdata[i])->path, rel)) return 1;
    return 0;
}

static gint64 count_objects(const char *drive)
{
    char *root = backup_root(drive), *obj = g_build_filename(root, "objects", NULL);
    gint64 n = 0;
    GDir *d = g_dir_open(obj, 0, NULL);
    const char *sub;
    while (d && (sub = g_dir_read_name(d))) {
        if (!strcmp(sub, "tmp")) continue;
        char *p = g_build_filename(obj, sub, NULL);
        GDir *e = g_dir_open(p, 0, NULL);
        while (e && g_dir_read_name(e)) n++;
        if (e) g_dir_close(e);
        g_free(p);
    }
    if (d) g_dir_close(d);
    g_free(obj);
    g_free(root);
    return n;
}

static int stop_after;
static volatile gint stop_flag;
static void stopper(gpointer d, gint64 df, gint64 tf, gint64 db, gint64 tb, const char *p)
{
    (void)d; (void)tf; (void)db; (void)tb; (void)p;
    if (df >= stop_after) g_atomic_int_set(&stop_flag, 1);
}

int main(void)
{
    check(always_left_out(".ssh") && always_left_out(".ssh/id_ed25519") && always_left_out(".cache/fontconfig/x"), "keys and caches left out");
    check(always_left_out(".local/share/nexa-browser/passwords.vault") && always_left_out(".config/nexaos-keys/release.key") &&
              always_left_out("Projects/server.key") && always_left_out(".local/share/Trash/files/x") &&
              always_left_out(".mozilla/firefox/x"),
          "NexaOS's private data, secret keys anywhere, the Trash, browser profiles left out");
    check(!always_left_out("Documents/cv.pdf") && !always_left_out(".config/nexa/strata.conf") && !always_left_out(".sshconfig"),
          "your own files and settings are backed up");
    char *e = escape_field("a\tb\nc\\d"), *u = unescape_field(e);
    check(!strcmp(u, "a\tb\nc\\d") && !strchr(e, '\t') && !strchr(e, '\n'), "odd names survive the list");
    g_free(e);
    g_free(u);
    char *r1 = restored_name("Docs/cv.pdf"), *r2 = restored_name("Docs/Makefile"), *r3 = restored_name(".bashrc");
    check(!strcmp(r1, "Docs/cv (restored).pdf") && !strcmp(r2, "Docs/Makefile (restored)") && !strcmp(r3, ".bashrc (restored)"),
          "a restored copy's name");
    g_free(r1); g_free(r2); g_free(r3);

    char *home = g_dir_make_tmp("bk-home-XXXXXX", NULL), *drive = g_dir_make_tmp("bk-drive-XXXXXX", NULL);
    char *dest = g_dir_make_tmp("bk-dest-XXXXXX", NULL);
    put(home, "Documents/cv.txt", "my cv, version 1", 16);
    put(home, "Documents/copy of cv.txt", "my cv, version 1", 16);  /* same contents: stored once */
    put(home, "Documents/deep/er/notes.md", "notes", 5);
    put(home, "Pictures/odd\tname \"x\"\n.txt", "odd", 3);
    put(home, "empty.txt", "", 0);
    put(home, "Documents/only-in-the-first.txt", "gone later", 10);
    put(home, ".bashrc", "alias ll='ls -l'", 16);
    put(home, ".ssh/id_ed25519", "SECRET KEY", 10);
    put(home, ".cache/junk.bin", "junk", 4);
    put(home, "Downloads/big.iso", "iso", 3);
    char *big = g_malloc(3 * 1024 * 1024 + 7);
    for (int i = 0; i < 3 * 1024 * 1024 + 7; i++) big[i] = (char)(i * 31 + 7);
    put(home, "Videos/clip.bin", big, 3 * 1024 * 1024 + 7);
    char *lnk = g_build_filename(home, "Documents/latest", NULL);
    symlink("cv.txt", lnk);
    char *unread = g_build_filename(home, "Documents/locked.txt", NULL);
    put(home, "Documents/locked.txt", "nope", 4);
    chmod(unread, 0);
    int root_user = geteuid() == 0;

    Choice c = {g_hash_table_new(g_str_hash, g_str_equal)};
    g_hash_table_add(c.off, "Downloads");
    gint64 nfiles = 0, need = backup_estimate(home, drive, &c, &nfiles, NULL);
    check(nfiles == 9 && need > 3 * 1024 * 1024, "the estimate: files and bytes to copy");
    Result res;
    GError *err = NULL;
    gboolean ok = backup_run(home, drive, &c, 1793469600, NULL, &res, &err);
    check(ok && res.snap_name, err ? err->message : "the first backup");
    check(root_user || (res.skipped == 1 && res.problems->len == 1 && strstr(res.problems->pdata[0], "locked.txt")),
          "a file that can't be read is skipped and named, the rest goes on");
    GPtrArray *all = snaps_list(drive);
    check(all->len == 1, "one backup on the drive");
    Snap *s1 = all->len ? all->pdata[0] : NULL;
    check(s1 && snap_load(s1, NULL) && has(s1, "Documents/cv.txt") && has(s1, "Documents/deep/er/notes.md") &&
              has(s1, "Pictures/odd\tname \"x\"\n.txt") && has(s1, "empty.txt") && has(s1, "Documents/latest") && has(s1, ".bashrc"),
          "files, folders, a link and odd names in it");
    check(s1 && !has(s1, ".ssh/id_ed25519") && !has(s1, ".cache/junk.bin") && !has(s1, "Downloads/big.iso"),
          "no secret key, no cache, not what was switched off");
    gint64 objects = count_objects(drive);
    check(objects == res.files - 1, "identical files stored once");
    result_clear(&res);

    /* A second backup: only what changed is copied. */
    put(home, "Documents/cv.txt", "my cv, version 2!", 17);
    struct utimbuf later = {1793470000, 1793470000};
    char *cvp = g_build_filename(home, "Documents/cv.txt", NULL);
    utime(cvp, &later);
    put(home, "Documents/new.txt", "new", 3);
    char *gonep = g_build_filename(home, "Documents/only-in-the-first.txt", NULL);
    g_unlink(gonep);
    ok = backup_run(home, drive, &c, 1793473200, NULL, &res, &err);
    check(ok && res.copied == 2 && res.copied_bytes == 20, "the next backup copies only the two changed files");
    result_clear(&res);
    g_ptr_array_free(all, TRUE);
    all = snaps_list(drive);
    check(all->len == 2 && ((Snap *)all->pdata[0])->when == 1793473200, "two backups, newest first");

    /* Putting back the first one: into a folder, and over the changed home. */
    Snap *first = all->pdata[1];
    gint64 back = 0;
    GPtrArray *problems = g_ptr_array_new_with_free_func(g_free);
    ok = restore_run(drive, first, NULL, home, dest, NULL, &back, problems, &err);
    char *t = get(dest, "Documents/cv.txt"), *bigback = get(dest, "Videos/clip.bin");
    check(ok && t && !strcmp(t, "my cv, version 1") && bigback && !memcmp(bigback, big, 3 * 1024 * 1024 + 7),
          "everything back into a folder, as it was then");
    g_free(t);
    g_free(bigback);
    char *lk = g_build_filename(dest, "Documents/latest", NULL), target[64] = "";
    ssize_t ln = readlink(lk, target, sizeof target - 1);
    check(ln > 0 && !strcmp(target, "cv.txt"), "the link too");
    g_free(lk);
    const char *one[] = {"Documents/cv.txt", NULL};
    ok = restore_run(drive, first, one, home, NULL, NULL, &back, problems, &err);
    char *now = get(home, "Documents/cv.txt"), *old = get(home, "Documents/cv (restored).txt");
    check(ok && back == 1 && now && !strcmp(now, "my cv, version 2!") && old && !strcmp(old, "my cv, version 1"),
          "put back where it was: the changed one is kept, the old one beside it");
    g_free(now);
    g_free(old);
    const char *notes[] = {"Documents/deep", NULL};
    char *np = g_build_filename(home, "Documents/deep/er/notes.md", NULL);
    g_unlink(np);
    ok = restore_run(drive, first, notes, home, NULL, NULL, &back, problems, &err);
    char *nt = get(home, "Documents/deep/er/notes.md");
    check(ok && nt && !strcmp(nt, "notes"), "a deleted file put back from a folder");
    g_free(nt);

    /* Checking, and damage found. */
    check(check_snap(drive, first, NULL, NULL) == 0, "Check: every file of the backup is fine");
    char *root = backup_root(drive);
    Item *cv = NULL;
    for (guint i = 0; i < first->items->len; i++)
        if (!strcmp(((Item *)first->items->pdata[i])->path, "Documents/deep/er/notes.md")) cv = first->items->pdata[i];
    char sub[3] = {cv->hash[0], cv->hash[1], 0};
    char *obj = g_build_filename(root, "objects", sub, cv->hash, NULL);
    chmod(obj, 0600);
    g_file_set_contents(obj, "NOTES", 5, NULL);
    check(check_snap(drive, first, NULL, NULL) == 1, "Check finds a damaged copy");
    g_unlink(np);
    g_ptr_array_set_size(problems, 0);
    restore_run(drive, first, notes, home, NULL, NULL, &back, problems, &err);
    check(!g_file_test(np, G_FILE_TEST_EXISTS) && problems->len == 1 && strstr(problems->pdata[0], "damaged"),
          "a damaged copy isn't put back as if it were fine");

    g_file_set_contents(obj, "notes", 5, NULL);  /* (mended for what follows) */

    /* Stopping halfway: no new backup appears. */
    put(home, "Documents/more.txt", "more", 4);
    stop_after = 1;
    Job job = {&stop_flag, stopper, NULL};
    g_clear_error(&err);
    ok = backup_run(home, drive, &c, 1793476800, &job, &res, &err);
    result_clear(&res);
    GPtrArray *after = snaps_list(drive);
    check(!ok && err && after->len == 2, "stopped: the earlier backups are all there, no half one");
    g_ptr_array_free(after, TRUE);
    g_clear_error(&err);

    /* A hostile list on the drive. */
    char *evil = g_build_filename(root, "snapshots", "2020-01-01_000000.snap", NULL);
    const char *bad = "NXBK1\t1577836800\t2\t2\t0\n"
                      "F\t644\t0\t2\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\t../../etc/passwd\t\n"
                      "F\t644\t0\t2\tnot-a-hash\tok.txt\t\nL\t777\t0\t0\t\t/abs\t/etc\n";
    g_file_set_contents(evil, bad, -1, NULL);
    GPtrArray *three = snaps_list(drive);
    Snap *ev = three->len == 3 ? three->pdata[2] : NULL;
    check(ev && snap_load(ev, NULL) && ev->items->len == 0, "a hostile backup list: nothing outside home, nothing without a hash");
    g_ptr_array_free(three, TRUE);
    g_unlink(evil);
    g_free(evil);

    /* Thinning out old backups. */
    GPtrArray *fake = g_ptr_array_new_with_free_func((GDestroyNotify)snap_free);
    gint64 t0 = 1800000000;
    for (int h = 0; h < 24 * 400; h += 6) {  /* one every 6 hours, 400 days */
        Snap *x = g_new0(Snap, 1);
        x->name = g_strdup_printf("s%d", h);
        x->when = t0 - (gint64)h * 3600;
        g_ptr_array_add(fake, x);
    }
    GPtrArray *gone = prune_plan(fake, t0);
    guint kept = fake->len - gone->len;
    check(kept >= 8 + 12 + 6 + 10 && kept <= 8 + 13 + 7 + 14, "old backups thinned: all of 2 days, daily, weekly, monthly");
    gboolean newest_kept = TRUE;
    for (guint i = 0; i < gone->len; i++) newest_kept &= strcmp(gone->pdata[i], "s0") != 0;
    check(newest_kept, "the newest one is always kept");
    g_ptr_array_free(gone, TRUE);
    g_ptr_array_free(fake, TRUE);

    /* Deleting a backup: only what nothing else needs goes. */
    gint64 before = count_objects(drive);
    char *only = NULL;
    for (guint i = 0; i < first->items->len; i++)
        if (!strcmp(((Item *)first->items->pdata[i])->path, "Documents/only-in-the-first.txt")) {
            Item *it = first->items->pdata[i];
            char d2[3] = {it->hash[0], it->hash[1], 0};
            only = g_build_filename(root, "objects", d2, it->hash, NULL);
        }
    check(snap_delete(drive, first->name, NULL), "the first backup deleted");
    GPtrArray *left = snaps_list(drive);
    check(left->len == 1 && count_objects(drive) == before - 1 && only && !g_file_test(only, G_FILE_TEST_EXISTS) &&
              check_snap(drive, left->pdata[0], NULL, NULL) == 0,
          "its own copies went; the other backup is complete");
    check(!snap_delete(drive, "../../x", NULL), "only backups can be deleted");
    g_ptr_array_free(left, TRUE);

    printf("%d of %d checks passed\n", checks - fails, checks);
    g_free(only); g_free(gonep);
    g_free(big); g_free(lnk); g_free(unread); g_free(cvp); g_free(np); g_free(obj); g_free(root);
    g_ptr_array_free(problems, TRUE);
    g_ptr_array_free(all, TRUE);
    g_hash_table_destroy(c.off);
    return fails != 0;
}
