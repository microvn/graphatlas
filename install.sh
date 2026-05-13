#!/usr/bin/env bash
# graphatlas install.sh — AS-004 one-liner installer.
#
# Usage (normal):
#   curl -fsSL https://graphatlas.dev/install.sh | sh
#
# Env overrides (primarily for tests + alternative release channels):
#   GRAPHATLAS_RELEASE_BASE   base URL for release assets
#                             (default: https://github.com/graphatlas-dev/graphatlas/releases/latest/download)
#   GRAPHATLAS_VERSION        override detected version tag (default: latest)
#   GRAPHATLAS_BIN_DIR        install target (default: ~/.local/bin)
#   GRAPHATLAS_SKIP_SHA256    "1" to skip sha256 check — STRONGLY NOT RECOMMENDED
#
# What it does:
#   1. Detect OS + arch → pick tarball name.
#   2. Download tarball + .sha256 sibling.
#   3. Verify sha256 (unless GRAPHATLAS_SKIP_SHA256=1).
#   4. Extract binary → $GRAPHATLAS_BIN_DIR/graphatlas (mode 0755).
#   5. Print PATH hint if target dir not in $PATH.

set -euo pipefail

RELEASE_BASE="${GRAPHATLAS_RELEASE_BASE:-https://github.com/graphatlas-dev/graphatlas/releases/latest/download}"
BIN_DIR="${GRAPHATLAS_BIN_DIR:-$HOME/.local/bin}"
SKIP_SHA="${GRAPHATLAS_SKIP_SHA256:-0}"

err() { printf 'error: %s\n' "$*" >&2; exit 1; }
info() { printf '%s\n' "$*"; }

# --- detect target --------------------------------------------------------

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Darwin)
        case "$arch" in
            arm64|aarch64) target="darwin-arm64" ;;
            x86_64)        target="darwin-x86_64" ;;
            *) err "unsupported macOS arch: $arch" ;;
        esac
        ;;
    Linux)
        # Musl detection: common signal is that `ldd --version` mentions 'musl'
        # and dynamic-link check on /bin/sh comes back with ld-musl-*.
        libc=gnu
        if ldd --version 2>&1 | grep -qi musl; then
            libc=musl
        fi
        case "$arch" in
            x86_64)  target="linux-x86_64-$libc" ;;
            aarch64) target="linux-aarch64" ;;
            *) err "unsupported Linux arch: $arch" ;;
        esac
        ;;
    MINGW*|MSYS*|CYGWIN*)
        target="windows-x86_64"
        ;;
    *) err "unsupported OS: $os" ;;
esac

info "detected target: $target"

tar_name="graphatlas-${target}.tar.gz"
tar_url="${RELEASE_BASE}/${tar_name}"
sha_url="${tar_url}.sha256"

# --- download -------------------------------------------------------------

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

tar_path="$tmp/$tar_name"
sha_path="$tmp/${tar_name}.sha256"

info "downloading $tar_url"
if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$tar_url" -o "$tar_path"
    if [ "$SKIP_SHA" != "1" ]; then
        curl -fsSL "$sha_url" -o "$sha_path"
    fi
elif command -v wget >/dev/null 2>&1; then
    wget -q "$tar_url" -O "$tar_path"
    if [ "$SKIP_SHA" != "1" ]; then
        wget -q "$sha_url" -O "$sha_path"
    fi
else
    err "need either curl or wget installed"
fi

# --- verify ---------------------------------------------------------------

if [ "$SKIP_SHA" = "1" ]; then
    info "WARNING: GRAPHATLAS_SKIP_SHA256=1 — skipping integrity check"
else
    expected="$(cat "$sha_path" | awk '{print $1}')"
    [ -n "$expected" ] || err "empty sha256 file"
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$tar_path" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$tar_path" | awk '{print $1}')"
    else
        err "need sha256sum or shasum to verify download"
    fi
    [ "$expected" = "$actual" ] || err "sha256 mismatch (expected $expected, got $actual)"
    info "sha256 verified"
fi

# --- install --------------------------------------------------------------

mkdir -p "$BIN_DIR"
tar -xzf "$tar_path" -C "$tmp"

# Tarball layout: graphatlas binary at the root.
src_bin="$tmp/graphatlas"
[ -f "$src_bin" ] || err "tarball missing graphatlas binary at root"

cp "$src_bin" "$BIN_DIR/graphatlas"
chmod 0755 "$BIN_DIR/graphatlas"

info "installed: $BIN_DIR/graphatlas"

# --- PATH hint ------------------------------------------------------------

case ":$PATH:" in
    *":$BIN_DIR:"*)
        info "PATH already includes $BIN_DIR — you're set"
        ;;
    *)
        info ""
        info "NOTE: $BIN_DIR is not in your PATH."
        info "Add this line to your shell rc (~/.bashrc, ~/.zshrc, etc):"
        info "    export PATH=\"$BIN_DIR:\$PATH\""
        ;;
esac

info ""
info "next step: graphatlas install --client claude"
info "           (or cursor / cline) to wire the MCP config."
