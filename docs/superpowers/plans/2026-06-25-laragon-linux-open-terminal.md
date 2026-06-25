# Open Terminal Button (Phase 2, Terminal slice B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A per-site "Open terminal" button (only for real-directory sites) that launches the user's default terminal emulator in that site's directory.

**Architecture:** A core `terminal` module detects the default emulator (`$TERMINAL` → `x-terminal-emulator` → known list, canonicalized) and spawns it detached with its working-directory pointed at the site folder. PATH/php is left to the Slice A shell-integration toggle. A Tauri `open_terminal` command and a frontend terminal-icon button on non-proxy rows wire it up.

**Tech Stack:** Rust (laragon-core, zero Tauri deps), Tauri 2, vanilla JS frontend.

## Global Constraints

- `core` keeps **zero Tauri deps**.
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD: failing test first for the pure `terminal_argv`.
- Button shown only for sites with a real directory (`source !== "Proxy"`); the command also rejects a non-directory path.
- Emulator working-dir flags: ptyxis `--new-window --working-directory=<dir>`; gnome-terminal/xfce4-terminal/tilix `--working-directory=<dir>`; konsole `--workdir <dir>`; kitty `--directory <dir>`; alacritty `--working-directory <dir>`; wezterm `start --cwd <dir>`; others none (rely on process cwd).
- Detection order: `$TERMINAL`, then `["x-terminal-emulator","gnome-terminal","ptyxis","konsole","xfce4-terminal","kitty","alacritty","wezterm","xterm"]`; canonicalize the result so `x-terminal-emulator` resolves to the real binary.
- Spawn detached (do not wait); PATH handled by Slice A, not here.
- Run core tests with `cargo test -p laragon-core`; build `cargo build -p laragon-desktop`. If `cargo`/`node` aren't on PATH use `$HOME/.cargo/bin/cargo` / `$HOME/.nvm/versions/node/v24.16.0/bin/node`.

---

### Task 1: `core::terminal` — detect + launch

**Files:**
- Create: `core/src/terminal.rs`
- Modify: `core/src/lib.rs` (declare + re-export)

**Interfaces:**
- Consumes: `bin::resolve_bin`.
- Produces:
  - `terminal_argv(emulator: &str, dir: &Path) -> Vec<String>`
  - `detect_terminal() -> Option<PathBuf>`
  - `open_terminal(dir: &Path) -> Result<(), TerminalError>`
  - `enum TerminalError { NoTerminal, Spawn(String) }`

- [ ] **Step 1: Write the failing test**

Create `core/src/terminal.rs` with imports + the test module first:

```rust
use crate::bin::resolve_bin;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn argv_per_emulator() {
        let d = Path::new("/srv/app");
        assert_eq!(
            terminal_argv("ptyxis", d),
            vec!["--new-window".to_string(), "--working-directory=/srv/app".to_string()]
        );
        assert_eq!(
            terminal_argv("gnome-terminal", d),
            vec!["--working-directory=/srv/app".to_string()]
        );
        assert_eq!(
            terminal_argv("konsole", d),
            vec!["--workdir".to_string(), "/srv/app".to_string()]
        );
        assert_eq!(
            terminal_argv("kitty", d),
            vec!["--directory".to_string(), "/srv/app".to_string()]
        );
        assert_eq!(
            terminal_argv("wezterm", d),
            vec!["start".to_string(), "--cwd".to_string(), "/srv/app".to_string()]
        );
        assert!(terminal_argv("xterm", d).is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core terminal`
Expected: FAIL to compile — `terminal_argv` not found.

- [ ] **Step 3: Implement the module**

Add above the `#[cfg(test)]` block in `core/src/terminal.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("no terminal emulator found")]
    NoTerminal,
    #[error("failed to launch terminal: {0}")]
    Spawn(String),
}

const TERMINAL_CANDIDATES: [&str; 9] = [
    "x-terminal-emulator",
    "gnome-terminal",
    "ptyxis",
    "konsole",
    "xfce4-terminal",
    "kitty",
    "alacritty",
    "wezterm",
    "xterm",
];

/// Working-directory arguments for a known terminal emulator (by basename).
/// Unknown emulators get no args and rely on the spawned process's cwd.
pub fn terminal_argv(emulator: &str, dir: &Path) -> Vec<String> {
    let d = dir.display().to_string();
    match emulator {
        "ptyxis" => vec!["--new-window".to_string(), format!("--working-directory={d}")],
        "gnome-terminal" | "xfce4-terminal" | "tilix" => vec![format!("--working-directory={d}")],
        "konsole" => vec!["--workdir".to_string(), d],
        "kitty" => vec!["--directory".to_string(), d],
        "alacritty" => vec!["--working-directory".to_string(), d],
        "wezterm" => vec!["start".to_string(), "--cwd".to_string(), d],
        _ => Vec::new(),
    }
}

/// Pick the default terminal emulator: $TERMINAL, then the known candidates.
/// The result is canonicalized so `x-terminal-emulator` resolves to the real
/// binary (e.g. ptyxis) — its basename then selects the right flags.
pub fn detect_terminal() -> Option<PathBuf> {
    let mut found: Option<PathBuf> = None;
    if let Ok(t) = std::env::var("TERMINAL") {
        if !t.is_empty() {
            found = resolve_bin(&t, &[]);
        }
    }
    if found.is_none() {
        for c in TERMINAL_CANDIDATES {
            if let Some(p) = resolve_bin(c, &[]) {
                found = Some(p);
                break;
            }
        }
    }
    found.map(|p| std::fs::canonicalize(&p).unwrap_or(p))
}

/// Launch the default terminal emulator in `dir`, detached.
pub fn open_terminal(dir: &Path) -> Result<(), TerminalError> {
    let emu = detect_terminal().ok_or(TerminalError::NoTerminal)?;
    let base = emu
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let args = terminal_argv(&base, dir);
    std::process::Command::new(&emu)
        .args(&args)
        .current_dir(dir)
        .spawn()
        .map_err(|e| TerminalError::Spawn(e.to_string()))?;
    Ok(())
}
```

- [ ] **Step 4: Re-export in lib.rs**

In `core/src/lib.rs`, add `pub mod terminal;` and:

```rust
pub use terminal::{open_terminal, TerminalError};
```

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p laragon-core terminal` then `cargo build -p laragon-core`
Expected: PASS — `argv_per_emulator` plus the crate builds (no unused-import warnings; `resolve_bin`/`PathBuf` are used).

- [ ] **Step 6: Commit**

```bash
git add core/src/terminal.rs core/src/lib.rs
git commit -m "feat(core): detect + launch the default terminal in a directory"
```

---

### Task 2: IPC `open_terminal` command

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs` (register)

**Interfaces:**
- Consumes: `laragon_core::open_terminal`.
- Produces: `open_terminal(path: String) -> Result<(), String>`.

- [ ] **Step 1: Add the command**

Append to `src-tauri/src/commands.rs` (call the core fn by full path so it doesn't clash with this command's name):

```rust
#[tauri::command]
pub fn open_terminal(path: String) -> Result<(), String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    laragon_core::open_terminal(&dir).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Register in main.rs**

In `src-tauri/src/main.rs` `generate_handler!`, add after `commands::set_terminal_integration,`:

```rust
            commands::open_terminal,
```

- [ ] **Step 3: Build**

Run: `cargo build -p laragon-desktop`
Expected: PASS — compiles cleanly (no warnings).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): open_terminal command"
```

---

### Task 3: Frontend — per-site terminal button

**Files:**
- Modify: `dist/app.js`

**Interfaces:**
- Consumes: `open_terminal({ path })`.
- Produces: UI only.

- [ ] **Step 1: Add the terminal icon**

In `dist/app.js`, in the `I` icon object (next to `copy`/`external`), add:

```js
    terminal: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3M13 15h4"/></svg>',
```

- [ ] **Step 2: Add the open-terminal helper**

Near `copySite`, add:

```js
  async function openTerminal(path) {
    try {
      await invoke("open_terminal", { path });
    } catch (e) {
      toast({ type: "error", title: "Couldn't open terminal", msg: String(e) });
    }
  }
```

- [ ] **Step 3: Render the button on non-proxy rows**

In `sitesView`'s row builder, add a `termBtn` alongside the existing `editBtn`/`removeBtn` definitions (after line defining `removeBtn`):

```js
            const termBtn = isProxy
              ? ""
              : '<button class="icon-btn sq32" data-action="open-terminal" data-path="' + esc(s.root) + '" aria-label="Open terminal" title="Open terminal here">' + I.terminal + "</button>";
```

Then insert `termBtn` into the returned row markup, immediately before the Copy button:

```js
              badge +
              termBtn +
              '<button class="icon-btn sq32" data-action="copy-site" data-name="' + esc(s.name) + '" aria-label="Copy URL">' + I.copy + "</button>" +
              editBtn + removeBtn +
```

- [ ] **Step 4: Wire the click handler**

In the delegated click handler, add (next to the `copy-site` case):

```js
    else if (a === "open-terminal") openTerminal(el.getAttribute("data-path"));
```

- [ ] **Step 5: Syntax-check the JS**

Run: `node --check dist/app.js`
Expected: PASS — exit 0, no output.

- [ ] **Step 6: Manual verification (live)**

Run: `cargo run -p laragon-desktop`. In **Sites**, a folder-backed site row (e.g. `demo`) shows a terminal icon button; clicking it opens the default terminal (ptyxis on this host) in that site's directory. A reverse-proxy row has no terminal button. With Slice A's Terminal-integration toggle on, `php`/`composer` in the opened terminal report the active version. (Spawning a real terminal window can't run in unit tests — human-verified.)

- [ ] **Step 7: Commit**

```bash
git add dist/app.js
git commit -m "feat(desktop): per-site Open terminal button"
```

---

## Self-Review

**1. Spec coverage:**
- §3.1 `terminal_argv` / `detect_terminal` (TERMINAL → candidates → canonicalize) / `open_terminal` (detached, current_dir) / `TerminalError` → Task 1. ✓
- §3.2 IPC `open_terminal(path)` (rejects non-dir; core by full path) + register → Task 2. ✓
- §3.3 frontend terminal icon + button on non-proxy rows + handler → Task 3. ✓
- §4 behavior (working-dir via flag + cwd; PATH via Slice A; detection order; detached; only real-dir) → Task 1 logic + Task 3 gating. ✓
- §6 testing: pure `terminal_argv` unit test (Task 1); node-check + live (Task 3). ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows full code. The one manual step (Task 3 Step 6) is an explicit human-verified gate (launching a real terminal window can't run in unit tests).

**3. Type consistency:**
- `open_terminal(&Path) -> Result<(), TerminalError>` (Task 1) called as `laragon_core::open_terminal(&dir)` in Task 2. ✓
- `terminal_argv(&str, &Path) -> Vec<String>` consumed by `open_terminal` with the canonical basename. ✓
- IPC arg name `path` matches the JS `invoke("open_terminal", { path })` (Task 3). ✓
- The button is gated on `isProxy` (already computed in the row builder) and passes `s.root`; the command validates `is_dir`. ✓
- `resolve_bin(name, &[])` matches the existing `resolve_bin(&str, &[PathBuf])` signature (empty extra-dirs slice). ✓

**Note:** `detect_terminal` canonicalizes so the Debian `x-terminal-emulator` alternative resolves to the real emulator (ptyxis here) and its basename selects the correct flags; if canonicalize fails it falls back to the resolved path. The button is absent on proxy rows (no directory), satisfying the "real-directory only" decision.
