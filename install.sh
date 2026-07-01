#!/bin/sh
# tono installer — downloads the latest release binary for this machine.
#
#   curl -fsSL https://raw.githubusercontent.com/marmikshah/tono/master/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/marmikshah/tono/master/install.sh | sh -s -- uninstall
#
# Options (environment variables):
#   TONO_VERSION      install a specific tag (e.g. v0.1.0); default: latest
#   TONO_INSTALL_DIR  where the binary goes; default: ~/.local/bin
set -eu

REPO="marmikshah/tono"
INSTALL_DIR="${TONO_INSTALL_DIR:-$HOME/.local/bin}"
BIN="$INSTALL_DIR/tono"

say()  { printf '%s\n' "$*"; }
fail() { printf 'install: %s\n' "$*" >&2; exit 1; }

# Interactive prompts read /dev/tty (stdin is the pipe under `curl | sh`);
# without a usable terminal every prompt falls back to its default.
ask() { # ask <question> -> stdout: the answer ("" when no terminal)
  { true < /dev/tty; } 2>/dev/null || { printf ''; return; }
  printf '%s ' "$1" > /dev/tty
  read -r ans < /dev/tty || ans=""
  printf '%s' "$ans"
}

# -- uninstall ------------------------------------------------------------------
do_uninstall() {
  [ -x "$BIN" ] || fail "nothing to uninstall at $BIN"
  rm -f "$BIN"
  say "Removed $BIN."
  exit 0
}
[ "${1:-}" = "uninstall" ] && do_uninstall

# -- existing installation ------------------------------------------------------
if [ -x "$BIN" ]; then
  CURRENT="$("$BIN" --version 2>/dev/null || echo "unknown version")"
  case "$(ask "Found $CURRENT at $BIN — [R]einstall or [u]ninstall?")" in
    u|U) do_uninstall ;;
    *)   say "Updating existing installation." ;;
  esac
fi

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar  >/dev/null 2>&1 || fail "tar is required"

# -- pick the release target for this machine -----------------------------------
OS="$(uname -s)" ARCH="$(uname -m)"
case "$OS" in
  Darwin)
    case "$ARCH" in
      arm64) TARGET="aarch64-apple-darwin" ;;
      *) fail "Intel macOS binaries are not published — install with: cargo install --git https://github.com/$REPO" ;;
    esac ;;
  Linux)
    case "$ARCH" in
      x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
      *) fail "no prebuilt binary for Linux/$ARCH — install with: cargo install --git https://github.com/$REPO" ;;
    esac ;;
  MINGW*|MSYS*|CYGWIN*)
    fail "on Windows, download the .zip from https://github.com/$REPO/releases/latest" ;;
  *) fail "unsupported platform: $OS/$ARCH" ;;
esac

# -- resolve the version (the /releases/latest redirect carries the tag) --------
VERSION="${TONO_VERSION:-}"
if [ -z "$VERSION" ]; then
  VERSION="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
    "https://github.com/$REPO/releases/latest" | sed 's|.*/tag/||')"
  [ -n "$VERSION" ] || fail "could not resolve the latest release tag"
fi

URL="https://github.com/$REPO/releases/download/$VERSION/tono-$VERSION-$TARGET.tar.gz"

# -- download, extract, install --------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

say "Downloading tono $VERSION ($TARGET)..."
curl -fsSL "$URL" -o "$TMP/tono.tar.gz" \
  || fail "download failed: $URL"
tar -xzf "$TMP/tono.tar.gz" -C "$TMP"

mkdir -p "$INSTALL_DIR"
install -m 755 "$TMP/tono" "$BIN"

say "Installed: $BIN ($("$BIN" --version))"

# -- PATH hint --------------------------------------------------------------------
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say ""
     say "Note: $INSTALL_DIR is not on your PATH. Add it, e.g.:"
     say "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

# -- next step --------------------------------------------------------------------
say ""
say "Render a sound:  tono render your-sound.json -o out/"
say "See the format:  tono --help  (and docs/cookbook.md)"
say ""
say "Optional, for real recorded instruments (the 'sampler' voice): download any"
say "free General MIDI SoundFont (FluidR3 GM, GeneralUser GS) and point the seq's"
say "sf2 parameter at it. Synth instruments need nothing."
