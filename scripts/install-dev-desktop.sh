#!/usr/bin/env sh
# Install (or uninstall) a dev desktop entry so the Laralux brand icon shows in
# the GNOME/Wayland dock/taskbar + title bar when running the dev build.
# Usage: scripts/install-dev-desktop.sh [install|uninstall]
set -eu

APP_ID="com.laralux.linux"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
DEST_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
DEST="$DEST_DIR/$APP_ID.desktop"

if [ "${1:-install}" = "uninstall" ]; then
  rm -f "$DEST"
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DEST_DIR" 2>/dev/null || true
  fi
  echo "Removed $DEST"
  exit 0
fi

ICON="$REPO/src-tauri/icons/icon.png"
if [ -x "$REPO/target/release/laralux-desktop" ]; then
  BIN="$REPO/target/release/laralux-desktop"
elif [ -x "$REPO/target/debug/laralux-desktop" ]; then
  BIN="$REPO/target/debug/laralux-desktop"
else
  echo "error: laralux-desktop not built. Run 'cargo build -p laralux-desktop' first." >&2
  exit 1
fi

mkdir -p "$DEST_DIR"
cat > "$DEST" <<EOF
[Desktop Entry]
Type=Application
Name=Laralux
Exec=$BIN
Icon=$ICON
Terminal=false
StartupWMClass=$APP_ID
Categories=Development;WebDevelopment;
EOF

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$DEST_DIR" 2>/dev/null || true
fi
echo "Installed $DEST"
echo "Relaunch Laralux for the icon to appear in the dock/taskbar."
