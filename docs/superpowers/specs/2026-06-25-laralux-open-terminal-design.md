# Laralux — Open Terminal Button (Phase 2, Terminal slice B) Design Spec

**Date:** 2026-06-25
**Status:** Design approved, pending spec review
**Goal:** A per-site "Open terminal" button that launches the user's default terminal emulator in that site's directory. PHP/composer in the opened terminal follow Laralux's active version via the existing shell-integration toggle (Slice A) — this slice does not inject PATH itself.

This is **Slice B** of the "terminal integration" work; Slice A (CLI/composer sync + PATH toggle) is merged. The button is shown only for sites backed by a real directory (Scanned/Linked); reverse-proxy sites (no folder) have no terminal button.

---

## 1. Context & current state

Sites come from `core::sites::list_all_sites`: `Site { name, root, hostname, source, proxy }` with `source ∈ {Scanned, Linked, Proxy}`. Scanned/Linked sites have a real `root` directory; Proxy sites have an empty `root`. The frontend renders one row per site (Copy / Open / Edit / Remove buttons depending on source). Slice A added the shell-integration toggle that prepends `~/laralux/bin` to the user's rc PATH, so any new terminal already resolves `php`/`composer` to the active version when the toggle is on.

The host is Ubuntu 26.04 GNOME, whose default terminal is **ptyxis** (`x-terminal-emulator` → `/usr/bin/ptyxis`). ptyxis CLI: `-d, --working-directory=DIR`, `--new-window`. `core::bin::resolve_bin` already resolves program names against PATH.

## 2. Approach (chosen: detect default emulator + open in the site dir)

A core `terminal` module detects the default terminal emulator and launches it with its working-directory pointed at the site folder, as a detached process. The PATH/php concern is delegated entirely to Slice A's rc block (so no fragile per-emulator env injection, which is unreliable for daemon-based terminals like ptyxis/gnome-terminal). The button only appears on rows with a real directory.

Rejected:
- **Always inject `~/laralux/bin` into the opened terminal's PATH** (via `-x`/`-- COMMAND`): per-emulator command syntax differs and daemon terminals don't inherit the launcher env; the global toggle already covers this. Out of scope.
- **A configurable terminal command in Settings**: auto-detection (incl. `x-terminal-emulator`) covers the common cases; YAGNI for now.

## 3. Architecture & components

### 3.1 `core/src/terminal.rs` (new)

- `pub fn terminal_argv(emulator: &str, dir: &Path) -> Vec<String>` (pure, unit-tested): given the emulator **basename** (the file name, e.g. `ptyxis` from `/usr/bin/ptyxis`), return the working-directory arguments:
  - `ptyxis` → `["--new-window", "--working-directory=<dir>"]`
  - `gnome-terminal`, `xfce4-terminal`, `tilix` → `["--working-directory=<dir>"]`
  - `konsole` → `["--workdir", "<dir>"]`
  - `kitty` → `["--directory", "<dir>"]`
  - `alacritty` → `["--working-directory", "<dir>"]`
  - `wezterm` → `["start", "--cwd", "<dir>"]`
  - anything else (e.g. `xterm`) → `[]` (rely on the spawned process's `current_dir`).
- `const TERMINAL_CANDIDATES: [&str; 9]` = `["x-terminal-emulator", "gnome-terminal", "ptyxis", "konsole", "xfce4-terminal", "kitty", "alacritty", "wezterm", "xterm"]` (preference order; `$TERMINAL` is tried before these).
- `pub fn detect_terminal() -> Option<PathBuf>`:
  1. If `$TERMINAL` is set and `resolve_bin($TERMINAL, &[])` finds it → use it.
  2. Else the first of `TERMINAL_CANDIDATES` that `resolve_bin(c, &[])` finds.
  3. **Canonicalize** the result (`std::fs::canonicalize`) so the Debian `x-terminal-emulator` symlink resolves to the real binary (e.g. `ptyxis`) — the canonical file's basename then drives `terminal_argv`. Returns `None` if nothing is found.
- `pub fn open_terminal(dir: &Path) -> Result<(), TerminalError>`:
  - `emu = detect_terminal()` → `Err(TerminalError::NoTerminal)` if `None`.
  - `basename` = `emu.file_name()` (lossy). `args = terminal_argv(basename, dir)`.
  - `std::process::Command::new(&emu).args(&args).current_dir(dir).spawn()` — **detached** (do not `wait`); on spawn error → `Err(TerminalError::Spawn(..))`. Returns `Ok(())` once spawned.
- `pub enum TerminalError` (thiserror): `NoTerminal`, `Spawn(String)`.
- Re-export `open_terminal`/`TerminalError` from `lib.rs`.

### 3.2 IPC (Tauri) — `src-tauri/src/commands.rs`

- `open_terminal(state, path: String) -> Result<(), String>`:
  - `let dir = std::path::PathBuf::from(&path)`; if `!dir.is_dir()` → `Err("not a directory: <path>")` (the frontend only offers the button for real-dir sites, but the command validates defensively).
  - `core::open_terminal(&dir).map_err(|e| e.to_string())`.
  - Synchronous: detection + a detached spawn are fast and non-blocking. Register in `main.rs`.

### 3.3 Frontend — `dist/` (per-site terminal button)

- Add a terminal **icon** to the `I` icon set (inline SVG, monochrome `currentColor`, matching the existing icon style).
- In `sitesView`, render a terminal icon button **only for non-proxy rows** (`s.source !== "Proxy"`): `data-action="open-terminal" data-path="<site.root>"`, placed next to the Copy button. Click → `invoke("open_terminal", { path })`; on error show a toast (e.g. "No terminal emulator found"). Success is silent (the terminal window appears).
- Proxy rows do not get the button (no real directory).

## 4. Behavior details & decisions

- **Working directory**: passed both via the emulator's flag (when known) and the spawned process `current_dir` — covers daemon terminals (flag) and simple ones (cwd).
- **PATH/php**: not handled here. With the Slice A toggle ON, the opened terminal's shell sources `~/.bashrc` and gets the active `php`/`composer`. With the toggle OFF, it uses the distro php — consistent with the user's choice.
- **Detection order**: `$TERMINAL` first (user override), then `x-terminal-emulator` (distro default; canonicalized to the real binary for correct flags), then a known list.
- **Detached**: the emulator is spawned and not awaited, so the app never blocks on it; the child outlives the call.
- **Only real-dir sites**: the button is absent on proxy rows; the command also rejects a non-directory path.

## 5. Error handling

- No emulator found → `TerminalError::NoTerminal` → `Err(String)` → toast "No terminal emulator found".
- Spawn failure → `TerminalError::Spawn` → toast with detail.
- Non-directory path → `Err(String)` (defensive; not normally reachable from the UI).

## 6. Testing (TDD; no GUI/spawn in unit tests)

- `terminal_argv`: `ptyxis` → `["--new-window", "--working-directory=/srv/app"]`; `gnome-terminal` → `["--working-directory=/srv/app"]`; `konsole` → `["--workdir", "/srv/app"]`; `kitty` → `["--directory", "/srv/app"]`; an unknown emulator (`xterm`) → `[]`.
- `detect_terminal`/`open_terminal` shell out / spawn a window and are **verified live** (depends on the host's installed emulator); the pure `terminal_argv` carries the unit coverage.
- Frontend: `node --check dist/app.js`; live click verification.

## 7. Out of scope (backlog)

- Injecting `~/laralux/bin` into the opened terminal's PATH independent of the Slice A toggle.
- A configurable terminal command / preferred-emulator setting.
- Opening a DB client or file manager (separate Phase-2 "open" actions).
- A general (non-site) "Open terminal" button.
- Node (nvm) / MariaDB version slices.
