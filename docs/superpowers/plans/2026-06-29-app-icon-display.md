# App Icon Display + Brand Consistency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Laralux brand icon show in the GNOME/Wayland dock/taskbar + title bar, and unify the in-app sidebar logo to the same "L" mark.

**Architecture:** (1) Render the brand PNG as the sidebar logo via a Vite public asset. (2) Provide a `.desktop` whose name + `StartupWMClass` match the window app_id `com.laralux` with the brand `Icon=` — a committed dev-install script for `cargo run`, and a corrected `debian/` for installs. (3) Align the autostart entry's icon name.

**Tech Stack:** TypeScript/Vite (frontend), POSIX shell (dev script), debhelper (packaging), Rust (laralux-core autostart).

## Global Constraints

- The window app_id (GTK/Wayland) is the bundle **`identifier` = `com.laralux`**. For the compositor to show the icon, a `.desktop` must match it (filename `com.laralux.desktop` AND `StartupWMClass=com.laralux`) and its `Icon=` must resolve to the brand icon.
- Brand icon source: `src-tauri/icons/icon.png` (the blue/teal "L + sparkle"). Do NOT replace it.
- Installed/themed icon name: `com.laralux` (file `com.laralux.png`). Dev uses an **absolute** `Icon=` path (no theme install needed).
- laralux-core stays Tauri-free. No new crate dependency.
- Commits: **no `Co-Authored-By` trailer.** Work on `master`.
- Scope is GNOME/Wayland-correct via the standard `<app_id>.desktop` + `StartupWMClass` matching; the `tauri-action` Release `.deb` already matches (Tauri uses the identifier) and is not touched here.

---

### Task 1: Sidebar brand logo → the "L" icon

**Files:**

- Create: `public/laralux.png` (copy of the brand icon)
- Modify: `src/ui/render.ts` (the `brand` block, ~line 122)
- Modify: `src/styles.css` (`.brand-mark` + a new `.brand-logo`)

**Interfaces:**

- Consumes: the brand PNG served by Vite at `/laralux.png`.
- Produces: the sidebar renders the brand icon image.

- [ ] **Step 1: Copy the brand icon into the Vite public dir**

```bash
mkdir -p public
cp src-tauri/icons/icon.png public/laralux.png
```

- [ ] **Step 2: Render the image in `src/ui/render.ts`**

Replace (around line 122):

```ts
    '<div class="brand"><div class="brand-mark">' + I.cube + "</div>" +
```

with:

```ts
    '<div class="brand"><div class="brand-mark"><img class="brand-logo" src="/laralux.png" alt="Laralux" /></div>' +
```

(Leave `I.cube` defined in `icons.ts` — it is still exported; do not remove it.)

- [ ] **Step 3: Adjust `src/styles.css`**

Replace the `.brand-mark` rule:

```css
.brand-mark {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    background: var(--primary);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--on-primary);
}
```

with (drop the colored background since the icon is full-color; clip the rounded corners):

```css
.brand-mark {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
}
.brand-logo {
    width: 100%;
    height: 100%;
    display: block;
    object-fit: cover;
}
```

- [ ] **Step 4: Verify**

Run: `test -f public/laralux.png && npm run build`
Expected: the file exists and the build prints `✓ built` with no TypeScript errors.

- [ ] **Step 5: Commit**

```bash
git add public/laralux.png src/ui/render.ts src/styles.css
git commit -m "feat(ui): sidebar brand logo uses the Laralux icon"
```

---

### Task 2: Dev desktop-entry install script

**Files:**

- Create: `scripts/install-dev-desktop.sh` (executable)

**Interfaces:**

- Consumes: the built `target/{release,debug}/laralux-desktop` binary + `src-tauri/icons/icon.png`.
- Produces: `~/.local/share/applications/com.laralux.desktop` so the dev window's icon resolves.

- [ ] **Step 1: Write the script**

Create `scripts/install-dev-desktop.sh`:

```sh
#!/usr/bin/env sh
# Install (or uninstall) a dev desktop entry so the Laralux brand icon shows in
# the GNOME/Wayland dock/taskbar + title bar when running the dev build.
# Usage: scripts/install-dev-desktop.sh [install|uninstall]
set -eu

APP_ID="com.laralux"
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
```

Then make it executable: `chmod +x scripts/install-dev-desktop.sh`.

- [ ] **Step 2: Verify (syntax + a temp-HOME round-trip that doesn't touch your real home)**

```bash
sh -n scripts/install-dev-desktop.sh
cargo build -p laralux-desktop
T="$(mktemp -d)"
env -u XDG_DATA_HOME HOME="$T" sh scripts/install-dev-desktop.sh
grep -q "StartupWMClass=com.laralux" "$T/.local/share/applications/com.laralux.desktop" && echo "entry ok"
grep -q "Icon=.*src-tauri/icons/icon.png" "$T/.local/share/applications/com.laralux.desktop" && echo "icon ok"
env -u XDG_DATA_HOME HOME="$T" sh scripts/install-dev-desktop.sh uninstall
test ! -f "$T/.local/share/applications/com.laralux.desktop" && echo "uninstall ok"
rm -rf "$T"
```

Expected: `entry ok`, `icon ok`, `uninstall ok` (and `sh -n` prints nothing).

- [ ] **Step 3: Commit**

```bash
git add scripts/install-dev-desktop.sh
git commit -m "build(dev): install-dev-desktop.sh for the app icon under cargo run"
```

---

### Task 3: Icon-name consistency — `debian/` + autostart

**Files:**

- Rename: `debian/laralux.desktop` → `debian/com.laralux.desktop` (and edit contents)
- Modify: `debian/rules` (`override_dh_auto_install` install paths)
- Modify: `core/src/autostart.rs` (`entry_contents` Icon + its test assertion)

**Interfaces:**

- Consumes: the window app_id `com.laralux`.
- Produces: an installed `.desktop` that matches the app_id with the brand icon; a consistent autostart icon name.

- [ ] **Step 1: Rename + fix the packaged desktop entry**

```bash
git mv debian/laralux.desktop debian/com.laralux.desktop
```

Then set `debian/com.laralux.desktop` to exactly:

```
[Desktop Entry]
Name=Laralux
Comment=Local web-development environment manager
Exec=laralux
Icon=com.laralux
Terminal=false
Type=Application
StartupWMClass=com.laralux
Categories=Development;WebDevelopment;
```

- [ ] **Step 2: Update `debian/rules` install paths**

In `override_dh_auto_install`, replace the desktop + icon install lines:

```make
	install -Dm644 debian/laralux.desktop debian/laralux/usr/share/applications/laralux.desktop
	install -Dm644 src-tauri/icons/icon.png debian/laralux/usr/share/icons/hicolor/512x512/apps/laralux.png
```

with:

```make
	install -Dm644 debian/com.laralux.desktop debian/laralux/usr/share/applications/com.laralux.desktop
	install -Dm644 src-tauri/icons/icon.png debian/laralux/usr/share/icons/hicolor/512x512/apps/com.laralux.png
```

(Leave the `/usr/bin/laralux` binary install line unchanged.)

- [ ] **Step 3: Align the autostart icon name in `core/src/autostart.rs`**

In `entry_contents`, change the `Icon` line from `Icon=laralux\n\` to `Icon=com.laralux\n\`:

```rust
fn entry_contents(exec_path: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Laralux\n\
         Exec={}\n\
         Icon=com.laralux\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         Comment=Local web-development environment manager\n",
        exec_path.display()
    )
}
```

- [ ] **Step 4: Assert the icon name in the autostart test**

In `core/src/autostart.rs` `mod tests`, in `write_entry_then_remove_idempotent`, after the existing `assert!(body.contains("Type=Application"));`, add:

```rust
        assert!(body.contains("Icon=com.laralux"));
```

- [ ] **Step 5: Verify**

Run: `cargo test -p laralux-core autostart`
Expected: PASS (the write/remove test now also asserts the new icon name).

Run: `grep -q "StartupWMClass=com.laralux" debian/com.laralux.desktop && grep -q "com.laralux.png" debian/rules && echo "debian ok"`
Expected: prints `debian ok`.

- [ ] **Step 6: Commit**

```bash
git add debian/com.laralux.desktop debian/rules core/src/autostart.rs
git commit -m "fix(packaging): desktop entry + icon name match app_id com.laralux"
```

---

## Self-Review

- **Spec coverage:** §2 sidebar logo → Task 1; §3a dev script → Task 2; §3b debian fix → Task 3; §3c autostart icon → Task 3; §6 tests are the per-task verifies (npm build, the temp-HOME script round-trip, the autostart assertion + debian greps) plus the user's GNOME/Wayland manual smoke.
- **Placeholder scan:** none — every step has complete content. The temp-HOME test deliberately avoids writing to the real `~/.local/share` during the gate (the user installs for real later via the same script).
- **Identifier consistency:** `com.laralux` is identical across the dev script (`APP_ID`, the `.desktop` filename, `StartupWMClass`), the `debian/com.laralux.desktop` filename + `StartupWMClass` + `Icon`, the `debian/rules` install paths (`.desktop` + `.png`), and `autostart.rs` `Icon=`. The brand asset path `src-tauri/icons/icon.png` is the single source for `public/laralux.png` (Task 1), the dev script `Icon=` (Task 2), and the debian icon install (Task 3). The sidebar `<img src="/laralux.png">` matches the `public/laralux.png` filename.
- **No-regression:** Task 1 leaves `I.cube` exported (no unused-symbol break); the autostart change keeps the existing test passing (it only adds an assertion that the new content satisfies). Verifies are per-crate (npm build / shell / cargo test) and green at each task boundary; the real GNOME/Wayland icon appearance is the user's manual smoke (run the script, relaunch).
