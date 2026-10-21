#!/bin/sh
# Publish the Nexa Store's online catalogue: copy the signed catalogue that
# `make nexa-apps` builds (build/store-repo) into the public nexa-apps
# repository (GitHub Pages serves it at https://b-less.github.io/nexa-apps/)
# and push. Signing happens here on the build machine; the key itself is
# never copied: only packages, icons and the signed index go out, and the
# catalogue must verify against NexaOS's trusted keys first.
#
# Usage: sh scripts/publish-store.sh
#   NEXA_APPS_REMOTE     git remote to publish to (default: the GitHub repo)
#   NEXA_STORE_REPO_DIR  the catalogue to publish (default: build/store-repo)
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
src=${NEXA_STORE_REPO_DIR:-$repo/build/store-repo}
site=$repo/build/nexa-apps-site
remote=${NEXA_APPS_REMOTE:-https://github.com/B-less/nexa-apps.git}

[ -f "$src/index.json" ] || { echo "No catalogue yet: run 'make nexa-apps' first." >&2; exit 1; }
# Only a catalogue signed by a key NexaOS trusts goes out.
report=$("$repo/build/native/nexa-pack" repo verify "$src" --keys "$repo/assets/keys")
echo "$report"
case "$report" in
    *"signed by trusted publisher"*) ;;
    *) echo "The catalogue isn't signed by a trusted NexaOS key (is the release key on this machine?) - not publishing." >&2; exit 1 ;;
esac
# Never a secret key, whatever ended up in the folder.
if find "$src" -name '*.key' | grep -q .; then
    echo "A .key file is in $src - not publishing." >&2
    exit 1
fi

if [ ! -d "$site/.git" ]; then
    rm -rf "$site"
    git clone -q "$remote" "$site"
fi
git -C "$site" pull -q --ff-only 2>/dev/null || true

# Replace the published catalogue with this one (packages that were
# dropped from the catalogue go too).
rm -rf "$site/index.json" "$site/index.sig" "$site/icons" "$site/packages"
cp -a "$src/index.json" "$src/index.sig" "$src/icons" "$src/packages" "$site/"
touch "$site/.nojekyll"   # serve the files as they are
apps=$(sed -n 's/.*"id": *"\([^"]*\)".*/\1/p' "$src/index.json" | sort -u | tr '\n' ' ')
cat > "$site/README.md" << EOF
# NexaOS apps

The online catalogue of the Nexa Store: signed \`.nexa\` packages built by the
NexaOS project. The Store reads it from https://b-less.github.io/nexa-apps/
and checks every signature; you don't need to download anything here by hand.

Apps: $apps

Signed by the NexaOS release key \`7003a4c26d287fdf\`.
EOF

cd "$site"
git add index.json index.sig icons packages README.md .nojekyll
git add -u
if git diff --cached --quiet; then
    echo "The published catalogue is already up to date."
    exit 0
fi
git -c user.name=nexa -c user.email=boatengbless23@gmail.com commit -q -m "Catalogue: $apps"
git push -q origin HEAD:main
echo "Published: $apps"
