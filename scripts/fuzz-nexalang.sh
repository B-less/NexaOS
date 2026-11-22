#!/bin/sh
# Fuzz nexac (the NexaLang compiler): mutate real NexaLang programs and run
# each through the whole pipeline -- the tools (check/fmt/explain/lsp) and,
# unless --tools-only, a hardened build plus an ASan+UBSan+leak-counter run.
# See nexalang/tests/fuzz/fuzz.py. Runs in the Alpine test container, which
# has gcc, GTK, GtkSourceView, libcurl, Xvfb and openssl, so nothing is
# skipped. Findings land in nexalang/tests/fuzz/findings/ (not committed).
#
# Usage: sh scripts/fuzz-nexalang.sh [COUNT [SEED [--tools-only]]]
#   COUNT  how many mutants (default 2000)
#   SEED   the RNG seed (default: the current time, so each run differs)
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^ALPINE_VERSION=//p' "$repo/build.conf")
image=nexa-langtest:${version%.*}
count=${1:-2000}
seed=${2:-$(date +%s)}
rest=${3:-}
need='gtk+3.0-dev gtksourceview4-dev curl-dev pkgconf xvfb xdotool valgrind imagemagick openssl python3 font-inter font-jetbrains-mono font-dejavu hicolor-icon-theme adwaita-icon-theme'
if ! docker run --rm "$image" sh -c "apk info -e $need" >/dev/null 2>&1; then
    echo "making $image (once)"
    printf 'FROM nexa-rust:%s\nRUN apk add --no-cache %s\n' "${version%.*}" "$need" \
        | docker build -q -t "$image" - >/dev/null
fi
mkdir -p "$repo/build/nexalang-target"
docker run --rm -v "$repo:/src" -w /src/nexalang \
    -e CARGO_TARGET_DIR=/src/build/nexalang-target -e CARGO_HOME=/tmp/cargo -e HOME=/tmp \
    "$image" sh -c "
        set -e
        # Alpine ships Xvfb but not the xvfb-run wrapper the fuzzer uses for
        # window mutants; a small shim gives it one (own display per call).
        cat > /usr/local/bin/xvfb-run <<'SHIM'
#!/bin/sh
while [ \$# -gt 0 ]; do case \"\$1\" in -a|-e|-d|-w) shift;; -s|-f|-n|-p) shift 2;; --) shift; break;; -*) shift;; *) break;; esac; done
n=\$(( (\$\$ % 400) + 100 )); i=0
while [ \$i -lt 60 ]; do
  Xvfb :\$n -screen 0 1024x768x24 -nolisten tcp >/dev/null 2>&1 &
  xp=\$!; sleep 0.3
  if kill -0 \$xp 2>/dev/null && [ -S /tmp/.X11-unix/X\$n ]; then break; fi
  kill \$xp 2>/dev/null; wait \$xp 2>/dev/null; n=\$((n+1)); i=\$((i+1))
done
export DISPLAY=:\$n
\"\$@\"; rc=\$?
kill \$xp 2>/dev/null; wait \$xp 2>/dev/null
exit \$rc
SHIM
        chmod +x /usr/local/bin/xvfb-run
        cargo build --release -q
        nexac=/src/build/nexalang-target/release/nexac
        echo \"fuzzing \$(\$nexac --version): $count mutants, seed $seed $rest\"
        python3 tests/fuzz/fuzz.py \"\$nexac\" $count $seed $rest
    "
