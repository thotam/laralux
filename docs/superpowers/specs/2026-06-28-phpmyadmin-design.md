# Laralux — phpMyAdmin (global web DB admin)

**Date:** 2026-06-28
**Status:** DEFERRED — design approved, but implementation postponed until phpMyAdmin 6.0 ships
(5.2.3 officially supports only PHP <8.4 while Laralux runs PHP 8.4; revisit when 6.0 — which targets
modern PHP — is released as a downloadable build). Beekeeper Studio (native client) is being built
first instead.
**Goal:** Bundle phpMyAdmin as a **global** database-management app — served by the existing
nginx + php-fpm stack on a dedicated localhost port, opened from the Dashboard and the tray. It is
NOT a per-site action and does NOT appear in the Sites list.

First of two DB-management sub-projects. The second (separate spec/plan) adds **Beekeeper Studio**
as a portable native client (MariaDB + Redis) launched from the tray/dashboard.

---

## 1. Context & current state

- Sites are served by nginx vhosts: `core/src/service/nginx.rs` writes `etc/nginx/nginx.conf`, whose
  `http {}` block ends with `include <etc>/nginx/sites/*.conf;`. Per-site vhosts are written into
  `etc/nginx/sites/<name>.conf` by `sync_sites` (`core/src/sync.rs`), which only *writes* files (it
  does not wipe/prune the dir). php-fpm listens on a unix socket `~/laralux/tmp/php-fpm.sock`
  (`PhpFpmService::socket_path`).
- MariaDB listens on `127.0.0.1:3306` (and `/tmp/mysql.sock`), root has no password
  (`auth-root-authentication-method=normal` from setup).
- Downloadable, no-apt tools follow a `*_static.rs` pattern (download a tarball into `~/laralux`,
  install). Setup installs components (`core/src/setup.rs` `Component` enum + `run_setup`).
- The tray menu (`src-tauri/src/main.rs`) has Start All / Stop All / Dashboard / Quit. The frontend
  opens URLs via `@tauri-apps/plugin-opener` (`openExternal` / `openUrl`).
- The static PHP build already ships the extensions phpMyAdmin needs — `php -m` shows `mysqli`,
  `mysqlnd`, `mbstring`, `session`, `json`/Core, `openssl`, `zlib`, `zip`, `hash` — so phpMyAdmin runs
  on it with no extra build work.

## 2. Approach

Treat phpMyAdmin as a **bundled web app on its own localhost port**, not a `*.dev` site:
- Download phpMyAdmin into `~/laralux/apps/phpmyadmin/` and write its `config.inc.php` pointing at
  the managed MariaDB with frictionless root auto-login (local dev).
- Add ONE built-in nginx server block, `etc/nginx/sites/_phpmyadmin.conf`, listening on a fixed
  loopback port (`127.0.0.1:9001`) with the phpMyAdmin dir as root and the php-fpm socket upstream.
  The `*.conf` is picked up by the existing `include sites/*.conf`; the `_` prefix keeps it distinct
  from user sites and `sync_sites` never deletes it. No `/etc/hosts` edit, no TLS cert (plain
  loopback HTTP is fine for a local tool).
- Expose a single global "Open phpMyAdmin" entry on the Dashboard and in the tray that opens
  `http://127.0.0.1:9001`.

This reuses nginx + php-fpm + mariadb wholesale; the only new server config is one static block on
an unprivileged port (no setcap needed).

## 3. Architecture & components

### 3.1 `core/src/phpmyadmin.rs` (new)
- `pub const PHPMYADMIN_PORT: u16 = 9001;`
- `pub const PHPMYADMIN_VERSION: &str` — a pinned phpMyAdmin release; download URL
  `https://files.phpmyadmin.net/phpMyAdmin/<ver>/phpMyAdmin-<ver>-english.tar.gz`.
- `pub fn install_dir(paths) -> PathBuf` → `paths.root().join("apps/phpmyadmin")`.
- `pub fn is_installed(paths) -> bool` → `install_dir/index.php` exists.
- `pub fn url() -> String` → `format!("http://127.0.0.1:{PHPMYADMIN_PORT}")`.
- `pub fn install_phpmyadmin(paths, downloader, runner, sink) -> Result<String, PhpMyAdminError>`:
  download the tarball into `tmp/`, extract (`tar -xzf`) into `install_dir` (strip the top-level
  `phpMyAdmin-…/` dir), then `write_config_inc(paths)` and `write_vhost(paths, php_socket)`. Idempotent
  (skips download if `is_installed`). Returns the version.
- `pub fn write_config_inc(paths) -> std::io::Result<()>`: write `install_dir/config.inc.php` with a
  random 32-char `blowfish_secret`, `$cfg['Servers'][1]['host']='127.0.0.1'`, `port='3306'`,
  `auth_type='config'`, `user='root'`, `password=''`, and `$cfg['Servers'][1]['AllowNoPassword']=true`
  → one-click login as root. Also `$cfg['TempDir']` under `~/laralux/tmp/phpmyadmin` so phpMyAdmin
  has a writable temp/cache dir.
- `pub fn write_vhost(paths, php_socket: &Path) -> std::io::Result<()>`: write
  `etc/nginx/sites/_phpmyadmin.conf` — a `server { listen 127.0.0.1:9001; server_name localhost;
  root <install_dir>; index index.php; location / { try_files $uri $uri/ /index.php?$query_string; }
  location ~ \.php$ { include <etc>/nginx/fastcgi_params; fastcgi_pass unix:<php_socket>;
  fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name; } }`.
- `#[derive(thiserror::Error)] pub enum PhpMyAdminError { Download(String), Extract(String), Io(#[from] std::io::Error) }`.
- Export from `lib.rs`.

### 3.2 nginx integration — keep the vhost alive across restarts
- `PhpFpmService` already knows the socket; the cleanest re-ensure point is `NginxService::write_config`
  (runs on every nginx start): after writing `nginx.conf`, if `phpmyadmin::is_installed(paths)`, call
  `phpmyadmin::write_vhost(paths, &php_socket)` so the `_phpmyadmin.conf` block always exists for a
  running nginx. (Install also writes it + reloads nginx so it works without a restart.)

### 3.3 Setup install — `core/src/setup.rs`
- Add `Component::Phpmyadmin` to the enum + `ALL` + `label()` ("phpmyadmin") + `detect_binary`/`detect`
  (present = `phpmyadmin::is_installed`) + `apt_packages_for` (empty). In `run_setup`, when missing,
  call `phpmyadmin::install_phpmyadmin(...)` and record it in `SetupReport` (`phpmyadmin_fetched: bool`).
  It thus installs with "Install missing" and shows in the Setup list like other components.

### 3.4 Desktop — command + tray
- `src-tauri/src/commands.rs`: `#[tauri::command] pub fn phpmyadmin_status() -> { installed: bool, url: String }`
  (installed via `is_installed`, url via `url()`); the frontend uses it to enable the button and to
  get the URL. Opening is done frontend-side via the opener plugin. (No new privileged op.)
- `src-tauri/src/main.rs` tray: add a "phpMyAdmin" menu item that, when installed, opens
  `http://127.0.0.1:9001` (via the opener plugin / a small open call), else focuses the window on
  Setup. Register `phpmyadmin_status` in the invoke handler.

### 3.5 Frontend
- `src/ipc/commands.ts`: `phpmyadminStatus()` → `invoke("phpmyadmin_status")`; reuse `openExternal`.
- `src/ipc/types.ts`: `PhpMyAdminStatus { installed: boolean; url: string }`.
- Dashboard (`src/ui/views/dashboard.ts`): a global **"Open phpMyAdmin"** button in the
  header/quick-actions area. On click: if installed → `openExternal(url)`; else toast "Install
  phpMyAdmin from Setup" (and/or navigate to Setup). The Setup view already lists components, so the
  new `Phpmyadmin` component appears there to install.

## 4. Data flow
1. Setup → Install missing (or a per-component install) downloads phpMyAdmin, writes `config.inc.php`
   + `_phpmyadmin.conf`, and reloads nginx.
2. nginx serves phpMyAdmin at `http://127.0.0.1:9001` (php-fpm executes `index.php`; MariaDB on 3306).
3. Dashboard/tray "Open phpMyAdmin" → opener opens the URL in the browser → auto-login as root.
4. On every nginx start, `write_config` re-ensures `_phpmyadmin.conf` when phpMyAdmin is installed.

## 5. Behavior & error handling
- Auto-login is intentional for a local dev DB (root/no-password on loopback). phpMyAdmin is bound to
  `127.0.0.1` only — not exposed off-machine.
- "Open phpMyAdmin" requires nginx + php-fpm + mariadb running; if the page errors, it's because the
  stack is stopped — the user starts the stack (same as any site). The button is enabled only when
  `installed`; otherwise it points the user to Setup.
- Install is best-effort with the existing progress/toast machinery; a failed download surfaces in the
  Setup report errors, like other components.
- `_phpmyadmin.conf` survives `sync_sites` (which only writes `<site>.conf`); the `_` prefix avoids
  collision with a user site literally named "phpmyadmin".

## 6. Testing (TDD where it applies)
- `core/src/phpmyadmin.rs`: `url()` returns `http://127.0.0.1:9001`; `is_installed` false on a fresh
  root and true after seeding `apps/phpmyadmin/index.php`; `write_config_inc` produces a file
  containing `AllowNoPassword`, `'host'` `127.0.0.1`, `auth_type` `config`; `write_vhost` produces a
  block containing `listen 127.0.0.1:9001`, the install dir as `root`, and `fastcgi_pass unix:` with
  the php-fpm socket. (Real download not unit-tested, like other `*_static` installers.)
- `setup.rs`: `Component::Phpmyadmin` present in `ALL` (count updated); `detect` reflects
  `is_installed`.
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke (needs a display + a running stack): Setup installs phpMyAdmin; Dashboard "Open
  phpMyAdmin" opens `http://127.0.0.1:9001` and lands logged-in on the databases list; it is NOT in
  the Sites list and there is no per-site DB button; the tray "phpMyAdmin" item opens the same URL.

## 7. Out of scope / backlog
- Beekeeper Studio native client (the next sub-project).
- Adminer (lighter single-file alternative) — not bundled now.
- Cookie/login auth or per-user DB credentials (local dev uses root auto-login).
- A `*.dev` pretty URL or TLS for phpMyAdmin (loopback port is sufficient).
- Pruning stale user-site vhosts in `sync_sites` (pre-existing behavior, unrelated).
