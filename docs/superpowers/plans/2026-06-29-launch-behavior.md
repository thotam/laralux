# App Launch Behavior Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three persisted, independent launch-behavior settings — Start on login, Start minimized to tray, Auto-start services on launch — each applied on every app launch.

**Architecture:** A `LaunchConfig` in core config + a Tauri-free `core/src/autostart.rs` managing the XDG `~/.config/autostart/laralux.desktop` entry. The desktop layer adds two commands (read + set, with the autostart `.desktop` written via `std::env::current_exe()`), reads the flags in `main.rs` `.setup` to show/hide the window and optionally run the existing `run_full_start`, and the frontend exposes three Settings toggles.

**Tech Stack:** Rust (laralux-core: zero Tauri deps; laralux-desktop: Tauri 2), TypeScript (Vite strict).

## Global Constraints

- **laralux-core keeps ZERO Tauri dependencies.** `autostart.rs` takes the executable path as a parameter (the desktop layer passes `std::env::current_exe()`); core never references Tauri or a hardcoded binary path.
- `LaunchConfig { start_on_login, start_minimized, autostart_services }`, all `bool`, **default `false`**, `#[serde(default)]` so old config files load. The three are independent and apply on every launch.
- Autostart uses the **XDG autostart** spec: `~/.config/autostart/laralux.desktop` (honoring `XDG_CONFIG_HOME`), NOT a systemd unit.
- The `set_launch_option` command writes/removes the `.desktop` **only** for the `start_on_login` key; the other two just persist config.
- The autostart `.desktop` carries no special flags (behaviors are config-driven, read every launch).
- `autostart_services` reuses `commands::run_full_start` guarded by `state.starting`; a pkexec prompt on a changed environment is expected (an unchanged restart self-skips).
- The main window becomes `visible: false`; `.setup` shows it unless `start_minimized` (the tray Dashboard item is the recovery path).
- Commits: **no `Co-Authored-By` trailer.** Work on `master`. Reference patterns: `ServicesConfig` (config.rs), `set_service_enabled` (commands.rs), `toggleServiceEnabled`/`loadServiceFlags` (settings.ts/render.ts), the tray `stack_toggle` thread + `ResetGuard` (main.rs).

---

### Task 1: Core — `LaunchConfig` + `autostart.rs`

**Files:**
- Modify: `core/src/config.rs` (`LaunchConfig` struct; `Config.launch` field + Default)
- Create: `core/src/autostart.rs`
- Modify: `core/src/lib.rs` (`pub mod autostart;`; exports for `LaunchConfig` and the autostart fns)

**Interfaces:**
- Consumes: nothing new (std + serde).
- Produces: `LaunchConfig { start_on_login: bool, start_minimized: bool, autostart_services: bool }` (Default = all false); `autostart_path() -> PathBuf`, `enable_autostart(exec_path: &Path) -> std::io::Result<()>`, `disable_autostart() -> std::io::Result<()>`, `is_autostart_enabled() -> bool`.

- [ ] **Step 1: Add `LaunchConfig` to `core/src/config.rs`**

After the `ServicesConfig` struct + its `Default` impl, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LaunchConfig {
    #[serde(default)]
    pub start_on_login: bool,
    #[serde(default)]
    pub start_minimized: bool,
    #[serde(default)]
    pub autostart_services: bool,
}
```

In the `Config` struct, add the field (after `proc_autostart`):

```rust
    #[serde(default)]
    pub launch: LaunchConfig,
```

In `impl Default for Config`, add `launch: LaunchConfig::default()` to the constructed struct (alongside `proc_autostart: BTreeSet::new()`):

```rust
        Self { tld: default_tld(), php_version: default_php(), services: ServicesConfig::default(), versions: BTreeMap::new(), symlinks: BTreeSet::new(), php_ini: crate::php_ini::PhpIniSettings::default(), proc_autostart: BTreeSet::new(), launch: LaunchConfig::default() }
```

- [ ] **Step 2: Add the config test**

In `config.rs` `mod tests`, add:

```rust
    #[test]
    fn launch_config_defaults_false_and_roundtrips() {
        let c = Config::default();
        assert!(!c.launch.start_on_login && !c.launch.start_minimized && !c.launch.autostart_services);
        let mut c2 = Config::default();
        c2.launch.start_on_login = true;
        c2.launch.autostart_services = true;
        let toml = toml::to_string(&c2).unwrap();
        let back: Config = toml::from_str(&toml).unwrap();
        assert!(back.launch.start_on_login && back.launch.autostart_services && !back.launch.start_minimized);
        // old config without the field loads to all-false
        let old: Config = toml::from_str("tld = \"dev\"\nphp_version = \"8.4\"\n").unwrap();
        assert_eq!(old.launch, LaunchConfig::default());
    }
```

- [ ] **Step 3: Write `core/src/autostart.rs`**

```rust
use std::path::{Path, PathBuf};

/// Resolve the XDG config base from the relevant env values (pure — no env read,
/// so it is deterministically testable).
fn resolve_base(xdg_config_home: Option<&str>, home: &str) -> PathBuf {
    match xdg_config_home {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(home).join(".config"),
    }
}

/// Path to the XDG autostart desktop entry for Laralux:
/// `$XDG_CONFIG_HOME/autostart/laralux.desktop` (else `~/.config/autostart/...`).
pub fn autostart_path() -> PathBuf {
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    resolve_base(xdg.as_deref(), &home)
        .join("autostart")
        .join("laralux.desktop")
}

fn entry_contents(exec_path: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Laralux\n\
         Exec={}\n\
         Icon=laralux\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         Comment=Local web-development environment manager\n",
        exec_path.display()
    )
}

fn write_entry(path: &Path, exec_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, entry_contents(exec_path))
}

fn remove_entry(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Write the autostart entry pointing at `exec_path` (caller passes the running
/// executable's path, e.g. `std::env::current_exe()`).
pub fn enable_autostart(exec_path: &Path) -> std::io::Result<()> {
    write_entry(&autostart_path(), exec_path)
}

/// Remove the autostart entry. A missing file is success (idempotent).
pub fn disable_autostart() -> std::io::Result<()> {
    remove_entry(&autostart_path())
}

/// Whether the autostart entry currently exists.
pub fn is_autostart_enabled() -> bool {
    autostart_path().exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_base_prefers_xdg_then_home_config() {
        assert_eq!(resolve_base(Some("/x/cfg"), "/home/u"), PathBuf::from("/x/cfg"));
        assert_eq!(resolve_base(Some(""), "/home/u"), PathBuf::from("/home/u/.config"));
        assert_eq!(resolve_base(None, "/home/u"), PathBuf::from("/home/u/.config"));
    }

    #[test]
    fn write_entry_then_remove_idempotent() {
        let dir = std::env::temp_dir().join(format!("lara-autostart-{}", std::process::id()));
        let path = dir.join("autostart").join("laralux.desktop");
        write_entry(&path, Path::new("/usr/bin/laralux")).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("Exec=/usr/bin/laralux"));
        assert!(body.contains("Name=Laralux"));
        assert!(body.contains("Type=Application"));
        // remove, then remove again — both succeed
        remove_entry(&path).unwrap();
        assert!(!path.exists());
        remove_entry(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 4: Register + export in `core/src/lib.rs`**

Add near the other `pub mod` lines:

```rust
pub mod autostart;
```

Update the config re-export (currently `pub use config::{Config, ServicesConfig};`) to:

```rust
pub use config::{Config, LaunchConfig, ServicesConfig};
```

Add the autostart re-export near the other `pub use` lines:

```rust
pub use autostart::{autostart_path, disable_autostart, enable_autostart, is_autostart_enabled};
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p laralux-core autostart` then `cargo test -p laralux-core config`
Expected: PASS (`resolve_base_prefers_xdg_then_home_config`, `write_entry_then_remove_idempotent`, `launch_config_defaults_false_and_roundtrips`). A single pre-existing flaky test `orchestrator::tests::start_reaps_prior_session_orphan` (ETXTBSY under parallel load) is unrelated; if only that fails, re-run it in isolation and note it.

- [ ] **Step 6: Commit**

```bash
git add core/src/config.rs core/src/autostart.rs core/src/lib.rs
git commit -m "feat(core): LaunchConfig + XDG autostart entry management"
```

---

### Task 2: Desktop commands — `launch_config` + `set_launch_option`

**Files:**
- Modify: `src-tauri/src/commands.rs` (imports; two new commands)
- Modify: `src-tauri/src/main.rs` (`generate_handler!` registration)

**Interfaces:**
- Consumes: `laralux_core::{LaunchConfig, enable_autostart, disable_autostart}` (Task 1); existing `Config`, `AppState`, `lock_err` (unused here), `state.paths.config_file()`.
- Produces: Tauri commands `launch_config() -> LaunchConfig` and `set_launch_option(key, enabled) -> LaunchConfig`.

- [ ] **Step 1: Extend imports in `commands.rs`**

Add `LaunchConfig`, `enable_autostart`, `disable_autostart` to the existing `use laralux_core::{ ... };` block (the first import block).

- [ ] **Step 2: Add the two commands**

Append to `commands.rs`:

```rust
#[tauri::command]
pub fn launch_config(state: tauri::State<AppState>) -> Result<LaunchConfig, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    Ok(config.launch)
}

/// Persist a launch flag; for `start_on_login` also write/remove the XDG
/// autostart entry (pointing at the running executable). Returns the new config.
#[tauri::command]
pub fn set_launch_option(
    state: tauri::State<AppState>,
    key: String,
    enabled: bool,
) -> Result<LaunchConfig, String> {
    let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
    match key.as_str() {
        "start_on_login" => config.launch.start_on_login = enabled,
        "start_minimized" => config.launch.start_minimized = enabled,
        "autostart_services" => config.launch.autostart_services = enabled,
        _ => return Err(format!("unknown launch option: {key}")),
    }
    config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
    if key == "start_on_login" {
        if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            enable_autostart(&exe).map_err(|e| e.to_string())?;
        } else {
            disable_autostart().map_err(|e| e.to_string())?;
        }
    }
    Ok(config.launch)
}
```

- [ ] **Step 3: Register in `main.rs`**

In `tauri::generate_handler![ ... ]`, add:

```rust
            commands::launch_config,
            commands::set_launch_option,
```

- [ ] **Step 4: Build**

Run: `cargo build -p laralux-desktop`
Expected: Finished (0 errors).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): launch_config + set_launch_option commands"
```

---

### Task 3: Desktop setup — apply launch behavior in `main.rs` + `tauri.conf.json`

**Files:**
- Modify: `src-tauri/tauri.conf.json` (window `visible`)
- Modify: `src-tauri/src/main.rs` (`.setup` closure)

**Interfaces:**
- Consumes: `laralux_core::Config` + `LaunchConfig` (Task 1); `AppState` (`paths`, `starting`); `commands::run_full_start` (existing).
- Produces: startup behavior (window shown unless minimized; services auto-started when enabled).

- [ ] **Step 1: Make the window start hidden in `tauri.conf.json`**

In the `app.windows[0]` object, change `"visible": true` to:

```json
        "visible": false,
```

- [ ] **Step 2: Apply launch behavior in `.setup`**

In `main.rs`, inside the `.setup(|app| { ... })` closure, AFTER the tray (`let tray = TrayIconBuilder...build(app)?;`) and BEFORE the realtime-status monitor block, insert:

```rust
            // Apply launch-behavior config: show the window unless "start
            // minimized", and optionally auto-start the stack on launch.
            let launch = {
                let st = app.state::<AppState>();
                laralux_core::Config::load(&st.paths.config_file())
                    .unwrap_or_default()
                    .launch
            };
            if !launch.start_minimized {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            if launch.autostart_services {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    let Some(state) = handle.try_state::<AppState>() else { return };
                    if state.starting.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        return;
                    }
                    struct ResetGuard<'a>(&'a std::sync::atomic::AtomicBool);
                    impl Drop for ResetGuard<'_> {
                        fn drop(&mut self) {
                            self.0.store(false, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let _reset = ResetGuard(&state.starting);
                    let _ = commands::run_full_start(&state);
                });
            }
```

(`app.state`, `app.get_webview_window`, `app.handle` are all available on the `&mut App` in setup — the tray handlers already use `get_webview_window("main")` and `try_state::<AppState>()`, and the tray `stack_toggle` arm uses the identical `starting.swap` + `ResetGuard` + `run_full_start` pattern.)

- [ ] **Step 3: Build**

Run: `cargo build -p laralux-desktop`
Expected: Finished (0 errors).

Note: runtime behavior (window hidden when minimized; services auto-start) is verified by the user's manual smoke (needs a display/login), not this task's gate.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tauri.conf.json src-tauri/src/main.rs
git commit -m "feat(desktop): apply launch behavior on startup (window + auto-start)"
```

---

### Task 4: Frontend — IPC, state, Settings toggles

**Files:**
- Modify: `src/ipc/types.ts` (`LaunchConfig`)
- Modify: `src/ipc/commands.ts` (`launchConfig`, `setLaunchOption`)
- Modify: `src/state.ts` (`launch` field + init)
- Modify: `src/ui/render.ts` (`loadLaunchConfig`)
- Modify: `src/ui/views/settings.ts` (replace the "Start on login" placeholder with three real toggles + handler)
- Modify: `src/ui/events.ts` (dispatch `launch-option`)
- Modify: `src/main.ts` (load launch config on boot)

**Interfaces:**
- Consumes: Task 2's `launch_config` / `set_launch_option` commands.
- Produces: `state.launch: LaunchConfig`; three Settings toggles wired to `setLaunchOption`.

- [ ] **Step 1: `src/ipc/types.ts`**

Add (near `ServicesFlags`):

```ts
export interface LaunchConfig {
  start_on_login: boolean;
  start_minimized: boolean;
  autostart_services: boolean;
}
```

- [ ] **Step 2: `src/ipc/commands.ts`**

Add `LaunchConfig` to the `import type { ... } from "./types";` line, and add the wrappers (mirroring `serviceFlags`/`setServiceEnabled`):

```ts
export const launchConfig = (): Promise<LaunchConfig> => invoke<LaunchConfig>("launch_config");

export const setLaunchOption = (key: string, enabled: boolean): Promise<LaunchConfig> =>
  invoke<LaunchConfig>("set_launch_option", { key, enabled });
```

- [ ] **Step 3: `src/state.ts`**

Add `LaunchConfig` to the `import type { ... } from "./ipc/types";` line. Add the field to the `AppState` interface (near `serviceFlags`):

```ts
  launch: LaunchConfig;
```

Initialize it in the exported `state` object (near `serviceFlags: {...}`):

```ts
  launch: { start_on_login: false, start_minimized: false, autostart_services: false },
```

- [ ] **Step 4: `src/ui/render.ts`**

Add `launchConfig` to the existing `import { ... } from "../ipc/commands";` line. Add the loader (next to `loadServiceFlags`):

```ts
export async function loadLaunchConfig(): Promise<void> {
  try {
    const c = await launchConfig();
    if (c && typeof c === "object") state.launch = c;
  } catch {
    /* keep defaults */
  }
}
```

- [ ] **Step 5: `src/ui/views/settings.ts` — three toggles + handler**

Replace the existing "Start on login" placeholder row:

```ts
    '<div class="set-row"><div class="grow"><div class="t">Start on login</div><div class="h">Autostart in system tray — coming soon</div></div>' +
    '<span class="toggle-off"><span class="knob"></span></span></div>' +
```

with three real toggle rows:

```ts
    launchRow("start_on_login", "Start on login", "Launch Laralux when you log in") +
    launchRow("start_minimized", "Start minimized to tray", "Launch hidden — open from the tray icon") +
    launchRow("autostart_services", "Auto-start services on launch", "Start the stack automatically when the app opens") +
```

Add the `launchRow` helper + the handler to `settings.ts`. Add the imports `setLaunchOption` (from `"../../ipc/commands"`) and `toast` (from `"../toast"`):

```ts
function launchRow(key: string, title: string, hint: string): string {
  const on = !!(state.launch as unknown as Record<string, boolean>)[key];
  return '<div class="set-row"><div class="grow"><div class="t">' + title + '</div><div class="h">' + hint + "</div></div>" +
    '<button class="' + (on ? "toggle-on" : "toggle-off") + '" data-action="launch-option" data-key="' + key + '" aria-pressed="' + on + '"><span class="knob"></span></button></div>';
}

export async function toggleLaunchOption(key: string): Promise<void> {
  const cur = state.launch as unknown as Record<string, boolean>;
  const next = !cur[key];
  state.launch = { ...state.launch, [key]: next };
  render();
  try {
    state.launch = await setLaunchOption(key, next);
  } catch (e) {
    state.launch = { ...state.launch, [key]: !next };
    toast({ type: "error", title: "Couldn't change launch setting", msg: String(e) });
  }
  render();
}
```

(The existing `settings.ts` already imports `state`, `render`, and from `"../../ipc/commands"`; add `setLaunchOption` to that import and add `toast`.)

- [ ] **Step 6: `src/ui/events.ts` — dispatch the action**

Import `toggleLaunchOption` from the settings view (where `toggleDark`/`toggleServiceEnabled` are imported), and add a case alongside `svc-enable`:

```ts
    else if (a === "launch-option") toggleLaunchOption(el.getAttribute("data-key")!);
```

- [ ] **Step 7: `src/main.ts` — load on boot**

Add `loadLaunchConfig` to the `import { ... } from "./ui/render";` line, and call it in the boot IIFE alongside `loadServiceFlags`:

```ts
(async () => {
  await loadServiceFlags();
  await loadLaunchConfig();
  render();
  refresh();
  loadProcCounts();
})();
```

- [ ] **Step 8: Build the frontend**

Run: `npm run build`
Expected: `✓ built` with no TypeScript errors.

- [ ] **Step 9: Commit**

```bash
git add src/ipc/types.ts src/ipc/commands.ts src/state.ts src/ui/render.ts src/ui/views/settings.ts src/ui/events.ts src/main.ts
git commit -m "feat(ui): launch-behavior settings (start on login / minimized / auto-start)"
```

---

## Self-Review

- **Spec coverage:** §2 LaunchConfig → Task 1; §3 autostart.rs → Task 1; §4 startup wiring (window + autostart_services) + tauri.conf visible → Task 3; §5 commands → Task 2; §6 frontend → Task 4. §8 error handling realized in `set_launch_option` (unknown-key Err, current_exe/IO Err → frontend revert), `disable_autostart` idempotency (Task 1), the non-fatal window show + the `starting`-guarded thread (Task 3), and the optimistic-revert handler (Task 4). §9 tests embedded in Task 1; build/npm gates in Tasks 2-4; runtime behavior is the user's manual smoke.
- **Placeholder scan:** none — every code step is complete. The `state.launch as unknown as Record<string,boolean>` cast indexes by the dynamic key safely (the three keys match `LaunchConfig`).
- **Type/identifier consistency:** the three keys `start_on_login` / `start_minimized` / `autostart_services` are identical across `LaunchConfig` (Rust, config.rs), the `set_launch_option` match arms, the TS `LaunchConfig` interface, the `data-key` values in `settings.ts`, and the `toggleLaunchOption` handler. `launch_config`/`set_launch_option` command names match between `main.rs` registration, `commands.rs` definitions, and `commands.ts` wrappers. `loadLaunchConfig` (render.ts) is imported and called in `main.ts`. `enable_autostart`/`disable_autostart`/`LaunchConfig` are exported from lib.rs (Task 1) and consumed by commands.rs (Task 2).
- **Compile order:** Task 2 depends on Task 1 (LaunchConfig + autostart fns exported); Task 3 depends on Task 1 (`Config.launch`) and is independent of Task 2's commands (both edit `main.rs` in different regions — do Task 2 then Task 3); Task 4 depends on Task 2's commands. Each task's gate (core test / desktop build / npm build) is green at its boundary.
