# Changelog

All notable changes to Laralux are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/) and this project adheres to
[Semantic Versioning](https://semver.org/).

## [0.4.1] - 2026-06-30

### Fixed
- CoreDNS no longer crashes on startup (tray icon stuck red): the bundled local
  resolver bound UDP `127.0.0.1:5353`, which collides with `avahi-daemon`'s
  mDNS listener on `0.0.0.0:5353` (present by default on most desktops), so
  CoreDNS exited immediately with "address already in use" and the whole stack
  showed as crashed. It now binds a dedicated port (15353) and falls back to the
  next free port if that is taken, so a one-off conflict can't crash-loop it.
- `*.dev` HTTPS no longer shows `ERR_CERT_AUTHORITY_INVALID` in Chrome/Chromium
  when the browser is first launched after setup: the mkcert CA could only be
  registered in NSS databases that already existed, so a browser whose
  `~/.pki/nssdb` was created later never trusted the CA. Setup now pre-seeds an
  empty Chromium NSS database (using the bundled `certutil`, still no `apt`) so
  the CA is always installed and the browser reuses it.

## [0.4.0] - 2026-06-30

### Fixed
- Setup no longer ends with "disable system services: pkexec command failed":
  the distro nginx/mariadb/redis systemd units are now disabled individually and
  any that aren't installed are skipped, instead of failing the whole batch (the
  units don't exist on a clean no-apt system, so this previously failed almost
  every time).
- Setup no longer ends with "mkcert -install (system) failed": the mkcert CA is
  now installed into the system trust store under privilege escalation with
  `CAROOT` pinned to the user's CA, instead of running mkcert unprivileged (which
  could not write the system store and had no TTY for its internal `sudo`).

### Changed
- Setup now performs its privileged steps (disable distro services, install the
  mkcert system CA, grant nginx the low-port bind capability) under a single
  authorization prompt instead of one prompt per step.

## [0.3.0] - 2026-06-29

### Fixed
- Single instance: opening Laralux while it is already running now focuses the
  existing window instead of spawning a duplicate process (multiple instances
  previously fought over the same ports and crashed services).
- App icon now resolves in the dock/taskbar and app grid: the icon ships at
  standard sizes (32/128/256/512) so it lands in recognized hicolor
  directories, instead of a single non-standard 671×671 size that desktop
  environments ignored.

## [0.2.0] - 2026-06-29

### Added
- Sidebar brand logo now uses the Laralux "L" app icon for consistent branding.
- `scripts/install-dev-desktop.sh` registers a desktop entry so the app icon
  appears in the GNOME/Wayland dock/taskbar when running the dev build.

### Fixed
- App icon now shows in the GNOME/Wayland dock/taskbar: the desktop entry's
  `StartupWMClass` matches the running window's app_id (the executable basename),
  so the compositor associates the window with the entry's icon and name.
- Packaging: the Debian desktop entry and hicolor icon are named to match the
  bundle identifier (`com.laralux.linux`), so the icon also resolves on
  installed systems.

## [0.1.0] - 2026-06-29

Initial release.

### Added
- Native Linux system-tray + GUI manager (Tauri 2) for a local web-development
  stack, with realtime service status and one-click Start All / Stop All.
- Managed services downloaded as no-apt static binaries into `~/laralux`:
  nginx, PHP-FPM, MariaDB, PostgreSQL, MongoDB, Redis, Mailpit, and CoreDNS
  (PostgreSQL/MongoDB are opt-in via Settings).
- Pretty `*.dev` HTTPS with automatic mkcert SSL; multi-version tool switching.
- Sites management: create from templates, link existing folders, reverse
  proxy, and custom domains.
- Per-site Procfile process runner for background workers, with a per-site
  autostart flag.
- Bundled DbGate database client (MariaDB / PostgreSQL / MongoDB / Redis).
- App launch behavior: start on login, start minimized to tray, and auto-start
  services on launch.
- Debian packaging (`debian/` source package) and a GitHub Actions release
  workflow that builds the `.deb` and publishes a GitHub Release on `v*` tags.

[0.4.1]: https://github.com/thotam/laralux/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/thotam/laralux/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/thotam/laralux/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/thotam/laralux/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/thotam/laralux/releases/tag/v0.1.0
