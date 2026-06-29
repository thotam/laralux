# Changelog

All notable changes to Laralux are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/) and this project adheres to
[Semantic Versioning](https://semver.org/).

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

[0.2.0]: https://github.com/thotam/laralux/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/thotam/laralux/releases/tag/v0.1.0
