#!/usr/bin/env bash
# Install Recorde for the current user (no root required).
# Run from the repository root: ./packaging/install.sh

set -euo pipefail

BINARY_NAME="recorde"
INSTALL_DIR="$HOME/.local/bin"
ICON_SRC="assets/icon.png"
ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"
DESKTOP_SRC="packaging/recorde.desktop"
DESKTOP_DIR="$HOME/.local/share/applications"

echo "Building release binary…"
cargo build --release --locked

echo "Installing to $INSTALL_DIR/$BINARY_NAME"
mkdir -p "$INSTALL_DIR"
cp "target/release/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
chmod +x "$INSTALL_DIR/$BINARY_NAME"
install -m 644 packaging/install-portable.json "$INSTALL_DIR/recorde-install.json"

echo "Installing icon to $ICON_DIR"
mkdir -p "$ICON_DIR"
cp "$ICON_SRC" "$ICON_DIR/$BINARY_NAME.png"

echo "Installing .desktop entry to $DESKTOP_DIR"
mkdir -p "$DESKTOP_DIR"
cp "$DESKTOP_SRC" "$DESKTOP_DIR/$BINARY_NAME.desktop"

if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache &>/dev/null; then
    gtk-update-icon-cache "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi

echo ""
echo "Recorde installed."
echo "Make sure $INSTALL_DIR is in your PATH, then run: recorde"
