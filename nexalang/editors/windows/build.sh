#!/bin/sh
# Build nexac.exe for Windows (64-bit) and the VS Code extension (.vsix),
# into nexalang/dist/ (not committed). NexaLang 0.9.
#
#   sh editors/windows/build.sh            (from nexalang/)
#
# Needs: Rust, mingw-w64 (Ubuntu/Debian: gcc-mingw-w64-x86-64; NexaOS:
# apk add mingw-w64-gcc), Node.js 22+ and npm. For Rust's Windows
# libraries, either
#   rustup target add x86_64-pc-windows-gnu          (the usual way), or
#   Rust's library source in the toolchain (rustup component add rust-src,
#   or a copy of rust-lang/rust's library/ at the same version as rustc,
#   linked to "$(rustc --print sysroot)/lib/rustlib/src/rust"); the
#   libraries are then built from it (what NexaLang 0.9's tests used).
set -eu
cd "$(dirname "$0")/../.."
TARGET=x86_64-pc-windows-gnu
SYSROOT=$(rustc --print sysroot)
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"
command -v "$CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER" >/dev/null || {
    echo "error: $CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER not found: install mingw-w64" >&2; exit 1; }
# The same nexac.exe from every build: no paths of this computer in it, no
# symbol table (it names temporary files), no link time in its header.
export RUSTFLAGS="--remap-path-prefix=$SYSROOT=/rust --remap-path-prefix=$PWD=/nexalang --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo -C strip=symbols -C link-arg=-Wl,--no-insert-timestamp"
TD=target/windows
if [ -d "$SYSROOT/lib/rustlib/$TARGET/lib" ]; then
    cargo build --release --locked --target $TARGET --target-dir $TD
elif [ -d "$SYSROOT/lib/rustlib/src/rust/library/std" ]; then
    RUSTC_BOOTSTRAP=1 cargo build --release --locked -Z build-std=std,panic_abort --target $TARGET --target-dir $TD
else
    echo "error: Rust has no Windows libraries: run 'rustup target add $TARGET' (or see the top of this file)" >&2
    exit 1
fi
mkdir -p dist
cp $TD/$TARGET/release/nexac.exe dist/nexac.exe
(cd editors/vscode && npm ci --no-audit --no-fund && npx vsce package --out ../../dist/nexalang-$(node -p 'require("./package.json").version').vsix)
cd dist && sha256sum nexac.exe nexalang-*.vsix
