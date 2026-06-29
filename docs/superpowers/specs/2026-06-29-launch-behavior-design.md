# Laralux — App launch behavior (autostart / minimized / auto-start services)

**Date:** 2026-06-29
**Status:** Design (approved for spec).
**Goal:** Add three independent, persisted launch-behavior settings, each applied on **every** app
launch: (1) **Start on login** (launch the app when the user logs in), (2) **Start minimized to
tray** (launch without showing the window), and (3) **Auto-start services on launch** (run the full
stack start when the app opens).

---

## 1. Context & current state

- **Config:** `core/src/config.rs::Config` aggregates feature sections (`ServicesConfig`,
  `proc_autostart: BTreeSet`, `php_ini`, …), each `#[serde(default)]` so old config files load. The
  new settings follow this pattern as a `LaunchConfig` struct.
- **Window/tray:** `src-tauri/tauri.conf.json` defines one window (`label: "main"`,
  `visible: true`). `main.rs` builds a tray and, on `WindowEvent::CloseRequested`, calls
  `api.prevent_close()` + `window.hide()` — so the app already lives in the tray and closing only
  hides it. The tray menu has a **Dashboard** item that `show()`s + focuses the window (the recovery
  path if the window starts hidden).
- **Startup:** `build_state()` loads `Config` and builds the orchestrator; `main.rs` `.setup(...)`
  wires the tray + monitor threads. There is no per-launch window/service automation today.
- **Full start:** `commands::run_full_start(&state) -> Vec<String>` is the shared Start-All sequence
  (sync hosts/cert/DNS + setcap + `start_all`); each privileged step **self-skips when nothing
  changed**, and it is guarded by the `state.starting: AtomicBool`. The UI Start-All and the tray
  toggle both call it.
- **Settings UI:** `src/ui/views/settings.ts` already renders a **"Start on login"** row as a static
  `toggle-off` labelled "coming soon"; this design replaces that placeholder with a working toggle
  and adds two more. The view already uses an optimistic-update + revert pattern
  (`toggleServiceEnabled` in the same file) to mirror.
- **No-apt / XDG:** autostart uses the freedesktop **XDG autostart** spec — a `.desktop` file in
  `~/.config/autostart/` — not a systemd unit (lighter, desktop-environment-agnostic, no root).

## 2. Settings model — `core/src/config.rs`

- New `LaunchConfig { start_on_login: bool, start_minimized: bool, autostart_services: bool }`,
  deriving the same traits as `ServicesConfig`, `Default` = all `false`.
- `Config` gains `#[serde(default)] pub launch: LaunchConfig`. Old config files without the field
  deserialize to the all-false default.
- Exported from `core/src/lib.rs` (alongside `ServicesConfig`).

## 3. Autostart entry — `core/src/autostart.rs` (zero Tauri deps)

Manage the XDG autostart `.desktop` file. The executable path is supplied by the caller (the desktop
layer passes `std::env::current_exe()`), so core stays Tauri-free and path-agnostic — it works for
both the dev binary (`target/.../laralux-desktop`) and the packaged binary (`/usr/bin/laralux`).

- `pub fn autostart_path() -> PathBuf`: `$XDG_CONFIG_HOME/autostart/laralux.desktop`, falling back to
  `~/.config/autostart/laralux.desktop` when `XDG_CONFIG_HOME` is unset/empty.
- `pub fn enable_autostart(exec_path: &Path) -> std::io::Result<()>`: create the parent dir and write
  the `.desktop` entry:
  ```
  [Desktop Entry]
  Type=Application
  Name=Laralux
  Exec=<exec_path>
  Icon=laralux
  Terminal=false
  X-GNOME-Autostart-enabled=true
  Comment=Local web-development environment manager
  ```
- `pub fn disable_autostart() -> std::io::Result<()>`: remove the file (a missing file is success —
  idempotent).
- `pub fn is_autostart_enabled() -> bool`: the file exists.
- The `.desktop` carries **no special flags** — the three behaviors are config-driven and read on
  every launch, so the autostart entry just relaunches the app normally.
- **Known limitation (documented, out of scope):** the `Exec` path is captured when the toggle is
  enabled; if the binary later moves (e.g. an unusual reinstall path change) the entry can go stale.
  For packaged installs `/usr/bin/laralux` is stable. Re-toggling rewrites it.

## 4. Startup wiring — `src-tauri/src/main.rs` + `tauri.conf.json`

- `tauri.conf.json`: the main window becomes **`visible: false`** so the app controls first paint
  (no flash when starting minimized).
- In `.setup(...)`, after the tray is built, read the launch flags (from the loaded `Config`):
  - **Window:** if `start_minimized` is false → `window.show()` + focus; if true → leave it hidden
    (the user opens it from the tray's Dashboard item). On any show error, log and continue (the tray
    remains the recovery path).
  - **Services:** if `autostart_services` is true → spawn a thread that runs the same
    `run_full_start(&state)` the UI/tray use, guarded by the existing `state.starting` flag (so it
    can't double-fire). **Caveat (expected):** if hosts/cert/DNS changed since last run, the
    privileged steps prompt via pkexec at launch; an unchanged restart self-skips and never prompts.
- `build_state()` already loads `Config`; expose the `LaunchConfig` to `.setup` (e.g. store it on
  `AppState` or re-read the config in setup) so the flags are available without re-parsing twice.

## 5. Commands — `src-tauri/src/commands.rs` + `main.rs` registration

- `#[tauri::command] launch_config(state) -> Result<LaunchConfig, String>`: load `Config`, return its
  `launch`.
- `#[tauri::command] set_launch_option(state, key: String, enabled: bool) -> Result<LaunchConfig, String>`:
  load `Config`, set the matching flag (`"start_on_login" | "start_minimized" | "autostart_services"`;
  unknown key → `Err`), save, and — **only** for `start_on_login` — call
  `enable_autostart(&std::env::current_exe()?)` or `disable_autostart()` so the `.desktop` file is
  written/removed immediately. Return the updated `LaunchConfig`. (Mirrors `set_service_enabled`'s
  load → mutate → side-effect → return shape.)
- Register both in `main.rs`'s `generate_handler!`.

## 6. Frontend — IPC + state + Settings

- `src/ipc/types.ts`: `LaunchConfig { start_on_login: boolean; start_minimized: boolean; autostart_services: boolean }`.
- `src/ipc/commands.ts`: `launchConfig(): Promise<LaunchConfig>`, `setLaunchOption(key, enabled): Promise<LaunchConfig>`.
- `src/state.ts`: `launch: LaunchConfig` (default all false); load it on boot (the `main.ts` boot
  IIFE, alongside the existing `loadServiceFlags`/`loadProcCounts`).
- `src/ui/views/settings.ts`: replace the static "Start on login" row with a real toggle, and add two
  rows — **"Start minimized to tray"** and **"Auto-start services on launch"**. Each toggle uses the
  existing optimistic-update-then-revert pattern (`data-action="launch-option"
  data-key="start_on_login|start_minimized|autostart_services"`), calling `setLaunchOption` and
  reverting on error (like `toggleServiceEnabled`). `src/ui/events.ts` dispatches the new action.

## 7. Data flow
1. Settings → toggle "Start on login" on → `setLaunchOption("start_on_login", true)` → config saved +
   `~/.config/autostart/laralux.desktop` written with `Exec=<current exe>` → next login launches the app.
2. Toggle "Start minimized" on → saved. Next launch: the window stays hidden; the tray icon is the
   entry point (Dashboard shows it).
3. Toggle "Auto-start services" on → saved. Next launch: `run_full_start` runs in a thread; services
   come up (privileged steps prompt only if something changed).
4. Toggle any off → flag cleared (and for start-on-login the `.desktop` is removed). Optimistic UI
   updates immediately and reverts if the command errors.

## 8. Error handling
- `set_launch_option` autostart write/remove failure (or `current_exe()` failure) → `Err(String)` →
  the frontend toggle reverts + shows a toast.
- `start_minimized` window show/hide error at startup → logged, non-fatal (tray Dashboard recovers).
- `autostart_services` failures surface through the orchestrator's normal `Crashed`/error path and
  the 1s monitor (same as a manual Start-All); the pkexec prompt on a changed environment is expected.
- Unknown `key` in `set_launch_option` → `Err` (defensive; the UI only sends the three known keys).
- `disable_autostart` on an already-absent file is success (idempotent).

## 9. Testing (TDD where it applies)
- `autostart.rs` (unit, via a temp `HOME`/`XDG_CONFIG_HOME`): `autostart_path()` honors
  `XDG_CONFIG_HOME` and falls back to `~/.config`; `enable_autostart(path)` writes a file whose
  contents include `Exec=<path>`, `Name=Laralux`, and `Type=Application`; `disable_autostart()`
  removes it and is idempotent when absent; `is_autostart_enabled()` reflects presence.
- `config.rs`: `LaunchConfig` defaults all-false; `Config.launch` roundtrips through TOML; an old
  config without the field loads to the default.
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke (needs a display + login session): toggle each option; confirm the `.desktop` file is
  created/removed under `~/.config/autostart/`; restart the app with "start minimized" on (it stays in
  the tray, opens from Dashboard); restart with "auto-start services" on (the stack comes up); log out
  and back in with "start on login" on (the app launches).

## 10. Out of scope / backlog
- A systemd **user service** alternative to XDG autostart.
- CLI flags (`--minimized`, etc.) — behaviors are config-driven instead.
- Auto-repairing the autostart `Exec` path if the binary moves after the toggle was enabled.
- Per-desktop-environment quirks beyond the standard XDG autostart + `X-GNOME-Autostart-enabled` key.
- A delay/"wait for network" option before auto-starting services.
