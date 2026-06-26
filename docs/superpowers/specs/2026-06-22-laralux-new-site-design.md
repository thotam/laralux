# Laralux — New Site / Quick App Creation (Phase 2, Slice 1) Design Spec

**Date:** 2026-06-22
**Status:** Design approved, pending spec review
**Goal:** A "New site" action that scaffolds a project in `~/laralux/www/` (Blank / Laravel / WordPress), optionally auto-creates a matching MySQL database, then makes it reachable at `https://<name>.dev`.

This is **Slice 1** of the Phase-2 "site management" work. Two follow-up slices are out of scope here and tracked in the backlog:
- **Slice 2 — Site registry + add existing folder** (register a site pointing at a directory outside `www/`).
- **Slice 3 — Reverse proxy sites** (a site that `proxy_pass`es to a port instead of serving PHP).

---

## 1. Context & current state

Today a "site" is purely auto-discovered: `core::sites::scan_sites` lists immediate subdirectories of `~/laralux/www/`, each becoming `Site { name, root, hostname }` with `document_root()` preferring `<root>/public`. `core::sync::sync_sites` generates a per-site HTTPS vhost + mkcert cert + `/etc/hosts` entry. The GUI `stack_start_all` already runs `sync_sites` before starting nginx.

This slice adds **creation**: produce files under `www/` (which the existing scan/sync then serve). Because new files land in `www/`, **no change to the site model/registry is needed** for Slice 1.

The redesigned dashboard already ships a disabled **"New site (+)"** button (header + empty-state, "Coming soon"). This slice enables it and adds a creation modal.

## 2. Approach (chosen: A — typed templates with testable seams)

Template logic lives in Rust (`core::scaffold`). External tools are invoked behind trait seams so the logic is unit-tested with fakes (no network/tools in tests); only the real run shells out. Rejected: B (Laralux-style configurable command strings — deferred as a future "custom template"), C (Blank-only — user wants all three).

## 3. Architecture & components

### 3.1 `core/src/scaffold.rs` (new)

- `enum SiteTemplate { Blank, Laravel, Wordpress }` — derives `serde::Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug`.
- `enum ScaffoldError` (thiserror): `InvalidName(String)`, `AlreadyExists(String)`, `ToolMissing(String)`, `Download(String)`, `Command(String)`, `Db(String)`, `Io(std::io::Error)`.
- `struct CreateReport { site_name: String, hostname: String, template: SiteTemplate, database_created: bool, warnings: Vec<String> }` — serde `Serialize`.
- **Seams:**
  - reuse existing `core::setup::Downloader` (real `CurlDownloader`, `FakeDownloader`) for the WordPress tarball.
  - new `trait CommandRunner: Send + Sync { fn run(&self, program: &str, args: &[String], cwd: Option<&std::path::Path>) -> Result<(), ScaffoldError>; }` with `RealCommandRunner` (std::process; non-zero exit → `Command`) and `FakeCommandRunner` (records `(program, args, cwd)` invocations; not `#[cfg(test)]`-gated so other modules' tests reuse it).
- **Pure generators (unit-tested):**
  - `validate_site_name(name: &str) -> Result<(), ScaffoldError>` — must match `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$`, length 1–63 (a valid DNS label). Rejects empty, uppercase, spaces, leading/trailing `-`.
  - `blank_index(site_name: &str) -> String` — see §4.
  - `wp_config(db_name, db_user, db_pass, db_host, salts: &str) -> String`.
  - `wp_salts() -> String` — 8 WordPress keys/salts, each 64 random printable chars generated locally (offline; no call to wordpress.org).
  - `create_database_sql(name: &str) -> String` → ``CREATE DATABASE IF NOT EXISTS `<name>` ``  (name already validated; wrapped in backticks).
  - `laravel_create_argv(target_dir: &str) -> Vec<String>` → `["create-project", "laravel/laravel", "<target_dir>"]` (program is `composer`).
- **Orchestrator:**
  `create_site(paths: &LaraluxPaths, name: &str, template: SiteTemplate, mariadb_running: bool, runner: &dyn CommandRunner, downloader: &dyn Downloader) -> Result<CreateReport, ScaffoldError>`
  Steps:
  1. `validate_site_name(name)`; compute `dir = paths.www().join(name)`; if it exists → `AlreadyExists`.
  2. Build files per template (§5). On any error after the dir was created, **remove `dir`** (rollback) before returning.
  3. Auto-DB (§5.4) — only if `mariadb_running`; failures become `warnings` (WordPress adds an emphatic warning when skipped).
  4. Return `CreateReport`.

### 3.2 IPC (Tauri) — `src-tauri/src/commands.rs`

- `create_site({ name: string, template: SiteTemplate }) -> CreateReport` (`#[tauri::command]`):
  1. Determine `mariadb_running` from the orchestrator's current `ServiceState` for `Mariadb`.
  2. Call `core::scaffold::create_site(...)` with `RealCommandRunner` + `CurlDownloader`.
  3. On success, run `sync_sites(...)` (vhost + cert + `/etc/hosts`, via `PkexecPrivileged` — same path as `stack_start_all`), then **reload nginx if it is Running** (orchestrator `stop(Nginx)` + `start(Nginx)`), so the new vhost is served immediately.
  4. Return the `CreateReport` (UI toasts + refreshes).
  Errors map to `Err(String)` for the frontend toast.

### 3.3 Setup wizard — add `composer`

Laravel requires composer. Add **`Composer`** to `core::setup::Component` (apt package `composer`; detection binary `composer`). It then appears in the Setup checklist and is installed by "Install missing". (Update `Component::ALL`, `detect`/`detect_binary`, `apt_packages_for`, `label`.)

### 3.4 Frontend — `dist/` (modal + wiring)

- Enable the existing **"New site (+)"** button; on click open a **modal** built with the redesign's tokens/components.
- Form: **Name** input (realtime validation, inline error; same rule as `validate_site_name`), **Type** selector (Blank / Laravel / WordPress), live preview `→ https://<name>.dev`.
- Submit: disable form, spinner, button text "Creating… (this can take a minute)"; call `invoke("create_site", { name, template })`.
- Success: success **toast** (e.g. "Created <name> · database created · https://<name>.dev"), close modal, refresh sites.
- Error: error **toast** with detail (e.g. "composer not installed — run Setup"); keep modal open for correction. **No `alert()`** — use the toast system.
- Accessibility: modal focus-trap, Esc to close, `:focus-visible` rings, labels for input/radio, `prefers-reduced-motion` respected.

## 4. Blank `index.php` content (welcome page)

`blank_index(site_name)` returns a self-contained HTML+PHP page (no external assets, inline CSS) that:
- Shows "🚀 `<site_name>` — powered by Laralux" and the current host.
- Shows basic PHP info via PHP: `phpversion()`, `PHP_SAPI`, `$_SERVER['SERVER_SOFTWARE']`, `$_SERVER['DOCUMENT_ROOT']`, HTTPS on/off.
- Lists a quick extension check using `extension_loaded()` for: `pdo_mysql`, `redis`, `curl`, `mbstring`, `gd` — each rendered ✓/✗ (icon + text, not color-only).
- Links to Mailpit (`http://localhost:8025`).
- Supports `?phpinfo` query: when present, render full `phpinfo()`; otherwise the curated welcome page (with a "View full phpinfo" link).

(Courtesy, not code: after build, the throwaway `~/laralux/www/demo/index.php` — currently raw `phpinfo()` — will be overwritten with this welcome page for consistency. It is runtime data under `~/laralux/www`, not part of the repo.)

## 5. Per-template behavior

### 5.1 Blank
`mkdir www/<name>`; write `index.php` = `blank_index(name)`. Docroot = `www/<name>`.

### 5.2 Laravel
`composer create-project laravel/laravel www/<name>` (CommandRunner program `composer`, args from `laravel_create_argv`, cwd `www/`). Then edit `www/<name>/.env`: set `DB_DATABASE=<name>`, `DB_USERNAME=root`, `DB_PASSWORD=` (empty), `DB_HOST=127.0.0.1`. Docroot resolves to `public/` automatically via the existing `Site::document_root`. Requires `composer` (else `ToolMissing`).

### 5.3 WordPress
Download `https://wordpress.org/latest.tar.gz` to `tmp/` (Downloader). Extract into `www/<name>` stripping the leading `wordpress/` directory (`tar -xzf <tarball> -C www/<name> --strip-components=1`). Write `wp-config.php` with `DB_NAME=<name>`, `DB_USER=root`, `DB_PASSWORD=''`, `DB_HOST=127.0.0.1`, and locally-generated salts (`wp_salts()`). Docroot = `www/<name>`.

### 5.4 Auto-database (enabled)
When MariaDB is running, run the mariadb client to execute `create_database_sql(name)` as root with no password (the project's dev default from `--auth-root-authentication-method=normal`), over `127.0.0.1`. Sets `database_created = true`.
- WordPress: DB required — if MariaDB is not running, still create files but add warning "start MariaDB, then create database `<name>`".
- Blank/Laravel: best-effort — if MariaDB is not running, skip with a light warning.
- The database name equals the (already validated) site name and is backtick-quoted (no injection surface).

## 6. Error handling

- Validate the name before any filesystem/network work; surface errors in the modal pre-submit.
- Missing tool (composer) → `ToolMissing` with guidance to run Setup.
- Network/command failures (composer, WordPress download, tar) → `Download`/`Command` with the captured stderr; **roll back** the partially-created `www/<name>` directory so no junk remains.
- Auto-DB failures are non-fatal (collected into `warnings`), with WordPress emphasizing the DB is required.
- All surfaced via toasts; never `alert()`.

## 7. Testing (TDD; fakes only, no network/tools)

- `validate_site_name`: accepts `blog`, `shop-api`; rejects ``, `Blog`, `a b`, `-x`, `x-`, an over-long name; rejects when `www/<name>` exists.
- `blank_index(name)` contains the site name and `phpversion(`; `wp_config(...)` contains `DB_NAME` = name and the salts block; `create_database_sql("a-b")` == ``CREATE DATABASE IF NOT EXISTS `a-b` ``.
- `laravel_create_argv("www/blog")` == `["create-project","laravel/laravel","www/blog"]`.
- `wp_salts()` returns 8 distinct key lines, each sufficiently long.
- `create_site` with `FakeCommandRunner` + `FakeDownloader` (temp `www/`):
  - Blank → `www/<name>/index.php` exists with welcome content; no runner calls.
  - Laravel → runner invoked with composer create-project; `.env` edited; (DB sql issued when `mariadb_running`).
  - WordPress → downloader fetched the WP URL; runner invoked for tar extract; `wp-config.php` written; DB sql issued / warning when not running.
  - `AlreadyExists` when the dir exists; rollback removes the dir on a simulated mid-create failure.
- Setup: `Composer` appears in `detect`/`apt_packages_for` (apt `composer`), tests stay hermetic.

## 8. Out of scope (backlog)

- Site registry / add existing folder (Slice 2); reverse proxy (Slice 3).
- Live streamed progress for long creates (v1 uses a blocking call + spinner; Tauri-events streaming is a future enhancement).
- Custom/configurable templates (approach B).
- Choosing PHP/Node version per site, PHP quick-settings, open terminal/DB/folder (other Phase-2 items, separate specs).
