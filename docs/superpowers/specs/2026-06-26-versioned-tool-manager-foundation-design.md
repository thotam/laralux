# Laralux — Versioned Tool Manager (Foundation) + Setup modals + /usr/local/bin symlinks

**Date:** 2026-06-26
**Status:** Design (approved for spec).
**Goal:** Give every app in Setup a per-app modal to (1) choose its version and (2) toggle a
`/usr/local/bin` symlink so its CLI is available system-wide. Move PHP version management from
Settings into the PHP modal in Setup. Replace the Settings "Terminal integration" toggle with the
per-app symlink mechanism.

This is the **foundation** sub-project of a larger effort ("all tools multi-version"). It builds the
uniform framework + UI + symlink manager and ships **PHP** as the only multi-version tool today;
every other tool exposes its single pinned version. Per-tool multi-version catalogs (nginx, mariadb,
redis, mailpit, mkcert, composer) are **follow-on sub-projects**, each its own spec/plan, that simply
fill in `available_versions` + a version-parameterized installer behind the seam defined here.

---

## 1. Context & current state

- **PHP is the only multi-version tool.** `php_versions.rs` exposes `KNOWN_PHP_VERSIONS` (8.0–8.5),
  `php_versions(paths, active) -> Vec<PhpVersionInfo{version,installed,active}>`, install via
  `install_php_static`, switch via `Orchestrator::replace_php_version` (+ `ensure_active_php_cli`).
- **Every other tool is pinned to one version** (a constant in its `*_static.rs`); no catalog, no
  version-parameterized installer.
- `config.versions: BTreeMap<String,String>` (tool→version) is the source of truth for the
  `bin/<tool>/current` symlinks (`apply_versions`). `config.shell_integration: bool` drives the
  Settings "Terminal integration" toggle (`enable_shell_path`/`disable_shell_path` in `shell_env.rs`,
  commands `terminal_integration_status`/`set_terminal_integration`).
- **Setup view** (`dist/app.js`) lists 7 components (Nginx, PHP, MariaDB, Redis, mkcert, Mailpit,
  Composer) with present/missing tags + an "Install missing" bulk action + a post-run report. No
  per-app version selection.
- **Settings view** holds the PHP version card and the Terminal-integration toggle.
- mkcert/mariadb/composer CLI spawns were just fixed to resolve through `managed_bin_dirs`
  (no-apt); `bin/<tool>/current/<cli>` paths are stable and absolute-resolvable.

## 2. Approach

Introduce a small **tool registry** in `core` that describes each managed tool uniformly, so the UI,
the version commands, and the symlink manager are generic — PHP's existing logic becomes one
implementation behind the seam, and later sub-projects add catalogs without touching the UI.

A tool's terminal CLI binary (if any) drives both the symlink toggle's presence and the symlink
source. Switching a version reuses the generalized "stop → repoint `current` → start if it was
running" logic (today's `replace_php_version`). Symlinks point at `bin/<tool>/current/<cli>`, so a
version switch is picked up automatically with no re-linking.

## 3. Architecture & components

### 3.1 `core/src/tools.rs` (new) — the registry
- `pub enum ManagedTool { Php, Nginx, Mariadb, Redis, Mailpit, Mkcert, Composer }` with
  `ALL: [ManagedTool; 7]`.
- `pub struct ToolInfo { pub key: &'static str, pub display: &'static str, pub cli_binary: Option<&'static str>, pub service_kind: Option<ServiceKind> }`.
- `pub fn info(tool) -> ToolInfo`. CLI mapping:
  - Php→`Some("php")`, Composer→`Some("composer")`, Mariadb→`Some("mariadb")`,
    Mkcert→`Some("mkcert")`, Redis→`Some("redis-cli")`, Nginx→`Some("nginx")`, Mailpit→`None`.
  - `service_kind`: Nginx/Mariadb/Redis/Mailpit→`Some(...)`; Php→`Some(PhpFpm)`; Mkcert/Composer→`None`.
- `pub fn from_key(&str) -> Option<ManagedTool>` and `key(tool) -> &'static str` for the IPC layer.
- `pub struct ToolVersion { pub version: String, pub installed: bool, pub active: bool }`.
- `pub fn available_versions(tool, paths) -> Vec<ToolVersion>`:
  - Php → reuse `php_versions(paths, active)` mapped into `ToolVersion`.
  - Others → a single entry: the tool's pinned constant version, `installed` = the tool's binary
    exists under `bin/<tool>/current`, `active` = true.
- `pub fn install_version(tool, paths, version, downloader, runner, sink) -> Result<String, ToolError>`:
  dispatch — Php→`install_php_static`; others→their existing `install_*` (version arg ignored for
  now, returns the pinned version). `ToolError` wraps the per-tool errors (thiserror `#[from]` where
  practical, else `Other(String)`).
- `pub fn cli_path(tool, paths) -> Option<PathBuf>`: `cli_binary.map(|b| bin/<tool>/current/b)`.

`switch_version` is **not** a new core fn; the orchestrator gains a generic `replace_version` (below)
and PHP keeps its CLI extra in the command layer.

### 3.2 `core/src/orchestrator.rs` — generalize version switch
- Generalize `replace_php_version` into
  `pub fn replace_version(&mut self, kind: ServiceKind, tool: &str, version: &str) -> Result<bool, ServiceError>`:
  stop `kind` if running → reap that tool's orphans (`reap(bin()/tool, tracked)`) → resolve full +
  `set_current(tool, full)` → start if it had been running. Keep `replace_php_version` as a thin
  wrapper calling `replace_version(PhpFpm, "php", v)` (so existing tests/callers stay valid).
- For tools with `service_kind == None` (mkcert/composer) the command layer just calls `set_current`
  (no process), so no orchestrator path is needed.

### 3.3 `core/src/symlinks.rs` (new) — /usr/local/bin manager
- `pub const SYSTEM_BIN_DIR: &str = "/usr/local/bin";`
- `pub fn system_link_path(tool) -> Option<PathBuf>`: `info(tool).cli_binary.map(|b| /usr/local/bin/b)`.
- `pub fn link_tool(paths, tool, privileged) -> Result<(), SymlinkError>`: resolve absolute
  `cli_path` (err `NotInstalled` if missing) → `privileged.create_symlink(&src, &dst)`.
- `pub fn unlink_tool(tool, privileged) -> Result<(), SymlinkError>`: `privileged.remove_symlink(&dst)`.
- The "linked" set lives in `config.symlinks` (see §3.5); this module performs the filesystem effect,
  the command layer persists config.

### 3.4 `core/src/privileged.rs` — symlink ops
- Trait `Privileged` gains:
  - `fn create_symlink(&self, src: &Path, dst: &Path) -> Result<(), PrivError>;`
  - `fn remove_symlink(&self, dst: &Path) -> Result<(), PrivError>;`
- Sudo/Pkexec impls: `ln -sfn <src> <dst>` and `rm -f <dst>` via the existing escalator. Free argv
  builders `ln_symlink_argv`/`rm_argv` are unit-tested (exact argv). `FakePrivileged` records the
  created/removed links for assertions (`symlinks_created()/symlinks_removed()`).

### 3.5 `core/src/config.rs`
- Add `#[serde(default)] pub symlinks: BTreeSet<String>` — tool keys currently linked into
  `/usr/local/bin`.
- **Remove** `shell_integration` (the terminal-integration feature is removed). Old configs with the
  field still load (serde ignores unknown fields). `normalize()` unchanged otherwise.

### 3.6 `src-tauri/src/commands.rs` — IPC
- New, tool-generic commands (all take a `tool: String` key, validated via `from_key`):
  - `tool_versions(tool) -> Vec<ToolVersion>`.
  - `install_tool_version(tool, version) -> Vec<ToolVersion>` (async, progress events; PHP reuses
    `install_php_static`, then `apply_versions` to keep `current` per config).
  - `set_tool_version(tool, version) -> Vec<ServiceStatus>`: persist `config.versions[tool]` →
    `orchestrator.replace_version(kind, tool, version)` when the tool is a service (PHP also
    `ensure_active_php_cli`) else `set_current`; if the tool is symlinked, the symlink already points
    at `current` so nothing else is needed.
  - `tool_symlinks() -> Vec<String>` (the configured set) and
    `set_tool_symlink(tool, enabled) -> Vec<String>`: create/remove `/usr/local/bin/<cli>` via
    `PkexecPrivileged`, update `config.symlinks`, return the new set. Errors (pkexec cancelled,
    not installed) propagate as `Err(String)` so the UI can revert the toggle and toast.
- **Remove** `terminal_integration_status` / `set_terminal_integration`; drop them from the
  `invoke_handler` and any tray wiring. Keep `php_versions`/`install_php_version`/`set_php_version`
  only if still referenced; otherwise remove in favor of the generic commands (UI switches over).
- `build_state` unchanged except it no longer reads `shell_integration`.

### 3.7 `dist/app.js` + styles — UI
- **Setup list rows become buttons** that open a per-app modal (`data-action="open-tool"`,
  `data-tool="php"`). Each row keeps its status tag and gains a small current-version chip.
- **Modal** (new component) renders for the focused tool:
  - Header: icon + display name + close.
  - "Versions" section: one row per `tool_versions` entry — `Active` badge / `Use` / `Install`
    (identical states to today's PHP card; inline ring on the busy row).
  - Divider, then a symlink toggle row shown only when `info.cli_binary` exists: "In terminal
    (`/usr/local/bin`)" with an On/Off switch and the `<cli>` name; disabled until the tool is
    installed.
  - Footer: Close.
- **Settings view**: remove the PHP version card and the Terminal-integration row. (Appearance, TLD,
  sites dir, Start-on-login placeholder remain.)
- The Setup post-run report and "Install missing" bulk action are unchanged.

### 3.8 Removals
- Delete `shell_env.rs` (`enable_shell_path`/`disable_shell_path`) and its `lib.rs` re-exports;
  remove the desktop terminal-integration commands and the Settings UI for it. `install_composer` /
  `ensure_active_php_cli` stay (still used by setup and `set_tool_version` for PHP).

## 4. Data flow
1. Setup renders rows from `setup_status` (present/missing) + a current-version chip from config.
2. Click row → load modal via `tool_versions(tool)` + `tool_symlinks()`.
3. Install → `install_tool_version` (progress ring), refresh modal.
4. Use → `set_tool_version` → repoint `current` (+restart service if running). Symlink auto-follows.
5. Toggle symlink → `set_tool_symlink(tool, on/off)` → pkexec `ln`/`rm` → persist `config.symlinks`.

## 5. Behavior & error handling
- All actions best-effort with toasts. pkexec cancelled → `Err` surfaced; the toggle reverts to its
  persisted state. A symlink for a not-yet-installed tool is refused (`NotInstalled`) and the toggle
  is disabled in the UI until installed.
- Switching a running service's version stops it (SIGTERM→wait→SIGKILL via the hardened `stop`),
  reaps that tool's orphans, repoints `current`, and restarts — reusing the just-built reaper.
- `/usr/local/bin/<cli>` is an absolute symlink to `bin/<tool>/current/<cli>`, so version switches
  need no re-link; removing a tool's symlink only `rm`s the link, never the target.

## 6. Testing (TDD)
- `tools.rs`: `cli_binary` mapping per tool; `available_versions` (PHP multi vs single-version tool);
  `cli_path` composition; `from_key`/`key` round-trip.
- `symlinks.rs`: `system_link_path` per tool (None for mailpit); `link_tool` calls
  `create_symlink(src=bin/<tool>/current/<cli>, dst=/usr/local/bin/<cli>)` and returns `NotInstalled`
  when the cli is absent (FakePrivileged).
- `privileged.rs`: `ln_symlink_argv`/`rm_argv` exact argv; FakePrivileged records links.
- `orchestrator.rs`: `replace_version` restarts a running service and is a no-op-start when stopped
  (generalize the existing `replace_php_version` tests; keep the PHP wrapper tests).
- `config.rs`: `symlinks` de/serializes; an old config without it loads with an empty set; a config
  with a stale `shell_integration` key still loads.
- Live: pkexec symlink create/remove; PHP install/switch (as today).
- `cargo test -p laralux-core`; `cargo build -p laralux-desktop && -p laraluxctl`.

## 7. Out of scope (this sub-project) / backlog
- Real multi-version catalogs + version-parameterized installers for nginx, mariadb, redis, mailpit,
  mkcert, composer — each a follow-on sub-project behind the `available_versions`/`install_version`
  seam.
- `~/.local/bin` (no-root) symlink scope — only `/usr/local/bin` now.
- A symlink for the Mailpit daemon (no terminal CLI) — excluded by design.
- Uninstalling a tool version.
