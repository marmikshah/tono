#!/bin/sh
# sonarium installer — downloads the latest release binary for this machine.
#
#   curl -fsSL https://marmikshah.github.io/sonarium/install.sh | sh
#   curl -fsSL https://marmikshah.github.io/sonarium/install.sh | sh -s -- uninstall
#
# Options (environment variables):
#   SONARIUM_VERSION      install a specific tag (e.g. v0.1.0); default: latest
#   SONARIUM_INSTALL_DIR  where the binary goes; default: ~/.local/bin
#   SONARIUM_MODE         "stdio" (default) or "http" (background daemon)
set -eu

REPO="marmikshah/sonarium"
INSTALL_DIR="${SONARIUM_INSTALL_DIR:-$HOME/.local/bin}"
BIN="$INSTALL_DIR/sonarium"
MCP_URL="http://127.0.0.1:8787/mcp"

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
  "$BIN" service uninstall >/dev/null 2>&1 || true # stop the daemon if present
  rm -f "$BIN"
  say "Removed $BIN (and stopped the background daemon, if one was installed)."
  say "Sounds in ~/.sonarium are untouched. If registered with an MCP client,"
  say "deregister manually, e.g.: claude mcp remove sonarium"
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
VERSION="${SONARIUM_VERSION:-}"
if [ -z "$VERSION" ]; then
  VERSION="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
    "https://github.com/$REPO/releases/latest" | sed 's|.*/tag/||')"
  [ -n "$VERSION" ] || fail "could not resolve the latest release tag"
fi

URL="https://github.com/$REPO/releases/download/$VERSION/sonarium-$VERSION-$TARGET.tar.gz"

# -- download, extract, install --------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

say "Downloading sonarium $VERSION ($TARGET)..."
curl -fsSL "$URL" -o "$TMP/sonarium.tar.gz" \
  || fail "download failed: $URL"
tar -xzf "$TMP/sonarium.tar.gz" -C "$TMP"

mkdir -p "$INSTALL_DIR"
install -m 755 "$TMP/sonarium" "$BIN"

say "Installed: $BIN ($("$BIN" --version))"

# -- PATH hint --------------------------------------------------------------------
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say ""
     say "Note: $INSTALL_DIR is not on your PATH. Add it, e.g.:"
     say "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

# -- choose how sonarium runs ------------------------------------------------------
# stdio: each MCP client spawns its own sonarium (zero setup).
# http:  one shared background daemon (launchd / systemd --user) at $MCP_URL —
#        all clients and sessions share a sound library; survives reboot.
MODE="${SONARIUM_MODE:-}"
if [ -z "$MODE" ]; then
  say ""
  case "$(ask "Run mode — [S]tdio (client spawns it) or [h]ttp (shared background daemon)?")" in
    h|H) MODE=http ;;
    *)   MODE=stdio ;;
  esac
fi

if [ "$MODE" = "http" ]; then
  if "$BIN" service install; then
    say "Daemon running at $MCP_URL"
  else
    say "Daemon install failed — register over stdio instead."
    MODE=stdio
  fi
fi

# -- next step ---------------------------------------------------------------------
say ""
say "Register with your MCP client (then restart its session):"
if [ "$MODE" = "http" ]; then
  say "  claude mcp add --scope user --transport http sonarium $MCP_URL   # Claude Code / Kimi Code"
  say "  Cursor: ~/.cursor/mcp.json -> \"sonarium\": { \"url\": \"$MCP_URL\" }"
else
  say "  claude mcp add --scope user sonarium -- $BIN     # Claude Code / Kimi Code: same shape"
  say "  Cursor: ~/.cursor/mcp.json -> \"sonarium\": { \"command\": \"$BIN\" }"
fi
say ""
say "Optional, for real recorded instruments (the 'sampler' voice): download any"
say "free General MIDI SoundFont (FluidR3 GM, GeneralUser GS) and point the seq's"
say "sf2 parameter at it. Synth instruments need nothing."
