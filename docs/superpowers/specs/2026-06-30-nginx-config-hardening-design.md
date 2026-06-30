# Spec: Nginx config hardening (mime.types + P1/P2 fixes)

Date: 2026-06-30
Source report: `NGINX-FIX.md` (root of repo)

## Problem

Laralux generates nginx config from Rust, but the generated `http {}` block sets
`default_type application/octet-stream;` **without** `include mime.types;`, and
laralux ships no `mime.types`. nginx therefore has no extension→MIME table, so
every static asset (`.css`, `.js`, `.woff2`, `.svg`, …) is served as
`application/octet-stream`. Browsers with strict MIME checking then refuse to
apply stylesheets and refuse to execute `<script type="module">`, so every site
served through laralux renders as unstyled HTML with no working JS. This is a
config bug affecting all sites, not an app bug.

The same report lists correctness/security gaps (P1) and performance/hardening
improvements (P2) that should ship together.

## Goal

Fix the MIME bug (P0) and apply the P1 and P2 improvements from `NGINX-FIX.md`,
entirely inside the Rust config generators — no new runtime deps, no `apt`, no
shipped sidecar files.

## Scope decisions (approved)

- **Scope:** P0 + P1 + P2 in one change.
- **mime.types provisioning:** embed as a Rust string constant and write it at
  `write_config` time — mirrors how `fastcgi_params` is already produced. No
  packaged sidecar file, no download.
- **Laravel detection for the `/build/` cache:** structural — emit the block only
  when `<site root>/artisan` exists. Works for both template-created and
  hand-linked Laravel sites; consistent with the existing `document_root()`
  heuristic that prefers `public/`. The `SiteTemplate` kind is NOT persisted, so
  it cannot be used here.
- **httpoxy fix:** add `fastcgi_param HTTP_PROXY "";` inside `fastcgi_params` so
  it covers the default server and every site PHP location from one place.

## Design

### File 1 — `core/src/service/nginx.rs`, `write_config` (global `http {}` + default server)

`http {}` block additions (order matters — `include mime.types` must precede
`default_type` and any server/include):

- **P0** write a new `mime.types` file (content = Appendix A of the report,
  embedded as a Rust const) and add `include <nginx_etc>/mime.types;` before
  `default_type application/octet-stream;` (kept as fallback).
- **P1** `charset utf-8;`.
- **P2** `sendfile on; tcp_nopush on; tcp_nodelay on; keepalive_timeout 65;
  server_tokens off;`.
- **P2** gzip block: `gzip on; gzip_vary on; gzip_comp_level 5;
  gzip_min_length 256;` + a `gzip_types` list (text/plain, text/css,
  application/javascript, application/json, image/svg+xml, application/xml,
  font/ttf, font/otf).

`fastcgi_params` (replace the current 11-line minimal file with the full
Appendix B set):

- Adds `REDIRECT_STATUS 200;`, `REQUEST_SCHEME`, `HTTPS $https if_not_empty`,
  `SERVER_SOFTWARE`, `SERVER_PORT`, `SERVER_ADDR`, `REMOTE_PORT`, `SCRIPT_NAME`,
  `CONTENT_*`, etc.
- **P1 httpoxy** also adds `fastcgi_param HTTP_PROXY "";`.

Default localhost server block:

- **P1** add `location ~ /\.(?!well-known).* { deny all; }` (its root is `www/`,
  the parent of all sites).

### File 2 — `core/src/sites.rs`, `vhost_config` (per-site `sites/*.conf`)

Applies to the `listen 443 ssl` block in BOTH the proxy and non-proxy variants
unless noted:

- **P2** `http2 on;` immediately after `listen 443 ssl;` (nginx ≥ 1.25.1
  directive; running 1.31.2, built `--with-http_v2_module`).
- **P2** SSL hardening: `ssl_protocols TLSv1.2 TLSv1.3;
  ssl_ciphers HIGH:!aNULL:!MD5; ssl_session_cache shared:SSL:10m;
  ssl_session_timeout 10m;`.
- **P1** dotfile deny `location ~ /\.(?!well-known).* { deny all; }` — non-proxy
  variant only (proxy sites have no filesystem root).
- **P2, Laravel only** `location ^~ /build/ { expires 1y;
  add_header Cache-Control "public, immutable"; try_files $uri =404; }` — emitted
  only when `<site root>/artisan` is a file. Non-proxy variant only.

httpoxy needs no per-site change (handled in `fastcgi_params`).

## Testing

Unit tests (assert generated strings):

- `nginx.rs`: `nginx.conf` contains `include …/mime.types`, `charset utf-8`,
  `sendfile on`, `server_tokens off`, `gzip on`; the `mime.types` file is written
  and contains `text/css                              css;` and
  `application/javascript                js mjs;`; the `fastcgi_params` file
  contains `REDIRECT_STATUS`, `REQUEST_SCHEME`, `HTTPS`, `SERVER_PORT`, and
  `HTTP_PROXY ""`.
- `sites.rs` `vhost_config`: contains `http2 on;`,
  `ssl_protocols TLSv1.2 TLSv1.3;`, and the dotfile-deny location; the `/build/`
  block is PRESENT when an `artisan` file exists in the site root and ABSENT when
  it does not (two cases). Proxy variant has `http2 on;` + ssl hardening but no
  dotfile/build block.
- Keep existing tests passing (websocket map, sites include, fastcgi_pass wiring,
  https redirect).

Manual verification (post-implementation, on the running stack):

- Regenerate config, reload nginx, then
  `curl -skI https://<site>/build/assets/<hashed>.css | grep -i content-type`
  → expect `Content-Type: text/css`.

## Out of scope (YAGNI)

HTTP/3, brotli, rate limiting, OCSP stapling, persisting `SiteTemplate` in
`sites.toml`, and any refactor beyond the two generator functions.
