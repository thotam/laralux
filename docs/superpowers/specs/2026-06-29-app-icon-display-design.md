# Laralux — App icon display + brand consistency

**Date:** 2026-06-29
**Status:** Design (approved for spec).
**Goal:** Make the Laralux brand icon (the blue/teal "L + sparkle", `src-tauri/icons/icon.png`)
appear in the GNOME/Wayland **dock/taskbar and window title bar**, and unify the **in-app sidebar
logo** to use that same "L" mark (replacing the current green cube glyph).

---

## 1. Context & current state

- **The icon asset is correct.** `src-tauri/icons/icon.png` (671×671 RGBA) is the real Laralux brand
  mark (white "L" + sparkle on a blue→teal rounded square), NOT a default Tauri icon.
- **Why it doesn't show at runtime.** The user runs GNOME/**Wayland**. On Wayland the dock/taskbar
  and title-bar icon are resolved by matching the window's **app_id** to a `.desktop` file (by
  filename `<app_id>.desktop` or by `StartupWMClass=<app_id>`), then reading that entry's `Icon=`.
  Tauri sets the GTK/Wayland app_id from the bundle **`identifier`** = `com.laralux.linux`. There is
  **no installed `.desktop`** matching that app_id (verified: nothing in
  `~/.local/share/applications`), so the compositor falls back to a generic icon. `bundle.icon` /
  embedded window icons are NOT used by most Wayland compositors for the taskbar.
- **Latent packaging bug.** The `debian/` source package installs `laralux.desktop` (filename
  `laralux`, `Icon=`/`Exec=laralux`), but the window app_id is `com.laralux.linux` — they don't
  match, so even the **installed** package would show a generic icon. The fix must align the
  installed entry to the app_id.
- **Branding inconsistency.** The OS app icon is the blue "L" (`icon.png`), but the in-app sidebar
  brand mark is `I.cube` (a green-tinted cube SVG) rendered at `src/ui/render.ts:122`
  (`'<div class="brand"><div class="brand-mark">' + I.cube + '</div>...'`). The user chose to unify
  on the blue "L".
- **Autostart entry.** `core/src/autostart.rs` writes `~/.config/autostart/laralux.desktop` with
  `Icon=laralux`; that icon name will not resolve either. Align it for consistency.

## 2. In-app sidebar logo → brand "L" (`src/ui/render.ts` + asset + CSS)

- Make the brand icon available to the frontend bundle: copy `src-tauri/icons/icon.png` to the Vite
  public dir as `public/laralux.png` (Vite serves `public/` at the site root, so it loads at
  `/laralux.png` and is copied into `dist`).
- In `src/ui/render.ts` (the `brand` block, ~line 122), replace the `I.cube` SVG inside
  `.brand-mark` with an `<img>` of the brand icon (e.g.
  `'<img class="brand-logo" src="/laralux.png" alt="Laralux" />'`).
- Add minimal CSS for `.brand-logo` (size to the existing brand-mark box, rounded corners) so the
  rounded icon sits correctly; if `.brand-mark`'s green background no longer suits a full-color icon,
  drop/adjust that rule. `I.cube` may remain exported in `icons.ts` (harmless) or be removed if
  unused elsewhere — keep it unless `noUnusedLocals` flags it.

## 3. App-icon resolution — desktop entry matched to the app_id

The window app_id is `com.laralux.linux`. The fix is to provide a `.desktop` the compositor can match
(filename = app_id, **and** `StartupWMClass=com.laralux.linux` as belt-and-suspenders) whose `Icon=`
points at the brand icon — for both dev and packaged installs.

### 3a. Dev — `scripts/install-dev-desktop.sh` (+ uninstall)
A POSIX shell script (committed, executable) that installs a dev desktop entry so the icon shows
under `cargo run`:
- Resolve the repo root from the script's own location; pick the built binary
  (`target/release/laralux-desktop` if present, else `target/debug/laralux-desktop`) and the icon
  (`<repo>/src-tauri/icons/icon.png`) as **absolute** paths.
- Write `~/.local/share/applications/com.laralux.linux.desktop`:
  ```
  [Desktop Entry]
  Type=Application
  Name=Laralux
  Exec=<abs binary>
  Icon=<abs icon path>
  Terminal=false
  StartupWMClass=com.laralux.linux
  Categories=Development;WebDevelopment;
  ```
  (an absolute `Icon=` path avoids needing an icon-theme install in dev).
- `update-desktop-database ~/.local/share/applications` (best-effort; ignore if absent).
- An `uninstall` mode (e.g. `install-dev-desktop.sh uninstall`) removes the entry + refreshes.
- Print a one-line note: rebuild/relaunch the app for the icon to appear; error clearly if the
  binary isn't built yet.

### 3b. Packaged — fix `debian/`
- Rename `debian/laralux.desktop` → `debian/com.laralux.linux.desktop`; add
  `StartupWMClass=com.laralux.linux` and set `Icon=com.laralux.linux` (keep `Exec=laralux`,
  `Name=Laralux`, the existing categories).
- `debian/rules` `override_dh_auto_install`: install the desktop as
  `usr/share/applications/com.laralux.linux.desktop` and the icon as
  `usr/share/icons/hicolor/512x512/apps/com.laralux.linux.png` (so `Icon=com.laralux.linux`
  resolves). Update the two install paths accordingly.

### 3c. Autostart entry — `core/src/autostart.rs`
- Change `Icon=laralux` → `Icon=com.laralux.linux` in `entry_contents` for naming consistency with
  the installed icon. (Functionally minor — the autostart entry only launches the app; its icon shows
  in "Startup Applications" lists.)

## 4. Data / resolution flow
1. Dev: developer runs `scripts/install-dev-desktop.sh` once → a matching `.desktop` with the brand
   `Icon=` lands in `~/.local/share/applications` → on next launch the compositor matches the window
   app_id `com.laralux.linux` to it and shows the brand icon in the dock/taskbar (and the title bar
   where the DE draws one).
2. Packaged: `apt install` lays down `com.laralux.linux.desktop` + the hicolor icon → same match,
   automatically, no script.
3. Sidebar: the app renders `<img src="/laralux.png">` as the brand mark — consistent "L" everywhere.

## 5. Error handling
- The dev script: if the binary isn't built, print a clear error and exit non-zero (do not write a
  broken entry). `update-desktop-database` missing → ignored (best-effort). Re-running is idempotent
  (overwrites the entry).
- The `<img>` brand logo: `alt="Laralux"` so it degrades to text if the asset fails to load.
- Packaging: the renamed desktop + icon paths are verified by the build; a wrong path would surface
  in `dpkg-buildpackage`/`lintian` (the user's packaging smoke).

## 6. Testing
- `core/src/autostart.rs`: update/extend the existing test to assert the entry contains
  `Icon=com.laralux.linux` (the write/remove test already checks `Name`/`Exec`/`Type`).
- `cargo test -p laralux-core` green; `npm run build` green (sidebar `<img>` + CSS compile).
- Desktop entry validity: `desktop-file-validate` on the dev-generated and `debian/` entries if the
  tool is present (else structural review).
- Manual smoke (needs GNOME/Wayland display): build the app, run
  `scripts/install-dev-desktop.sh`, relaunch → the blue "L" icon appears in the dock/taskbar and the
  title bar; the sidebar shows the same "L". `scripts/install-dev-desktop.sh uninstall` removes the
  dev entry.

## 7. Out of scope / backlog
- Regenerating a multi-resolution icon set (the single 671×671 PNG is sufficient; a future `tauri
  icon` pass can add sizes).
- The Release `.deb` built by `tauri-apps/tauri-action` (Tauri auto-generates its Linux `.desktop`
  from the `identifier`, so it already matches the app_id) — only the hand-authored `debian/` source
  package is corrected here.
- Non-GNOME desktop quirks beyond the standard `StartupWMClass`/`<app_id>.desktop` matching.
- Auto-installing the dev desktop entry from within the app on first run (kept as an explicit script
  to avoid surprise writes to the user's applications dir).
- A dedicated SVG redraw of the "L" mark for the sidebar (using the existing PNG via `<img>` instead).
