#!/bin/sh
# pirs installer: detects platform, downloads the latest release from GitHub.
# Usage: curl -fsSL https://raw.githubusercontent.com/xmonader/pirs/main/scripts/install.sh | sh
#
# Installs: pirs (harness), pirs-claw (agent), pirs-orchestrator (fleet).
# Override install dir: PIRS_INSTALL_DIR=~/bin
set -eu

REPO="xmonader/pirs"
INSTALL_DIR="${PIRS_INSTALL_DIR:-$HOME/.local/bin}"

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
case "$OS-$ARCH" in
  linux-x86_64)   TARGET="x86_64-unknown-linux-gnu" ;;
  linux-aarch64)  TARGET="aarch64-unknown-linux-gnu" ;;
  darwin-aarch64) TARGET="aarch64-apple-darwin" ;;
  darwin-x86_64)  TARGET="x86_64-apple-darwin" ;;
  *) echo "unsupported platform: $OS-$ARCH" >&2; exit 1 ;;
esac

echo "fetching latest pirs release for $TARGET..."
URL=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | grep "browser_download_url" \
  | grep "$TARGET" \
  | cut -d '"' -f 4)

if [ -z "$URL" ]; then
  echo "no release binary found for $TARGET" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL "$URL" -o "$TMP/pirs-bundle.tar.gz"
tar -xzf "$TMP/pirs-bundle.tar.gz" -C "$TMP"
for bin in pirs pirs-claw pirs-orchestrator; do
  if [ -f "$TMP/$bin" ]; then
    install -m 755 "$TMP/$bin" "$INSTALL_DIR/$bin"
    echo "installed $bin -> $INSTALL_DIR/$bin"
  fi
done

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "note: add $INSTALL_DIR to your PATH" ;;
esac
echo "done."
echo "  harness: pirs --mode tui"
echo "  agent:   pirs-claw chat \"…\"  |  pirs-claw serve --channel telegram"
