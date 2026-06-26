# Laralux — Design Spec

**Ngày:** 2026-06-18
**Trạng thái:** Đã duyệt thiết kế, chờ review spec
**Mục tiêu:** Xây dựng app quản lý môi trường dev cho Linux tương tự Laralux (Windows), bám theo stack thực tế người dùng đang dùng: Nginx + PHP-FPM + MariaDB + Redis + Node + Mailpit, pretty URL `*.dev`, chủ yếu phục vụ project Laravel.

---

## 1. Bối cảnh

Laralux (Windows, viết bằng Delphi) là app GUI chạy ở system tray, tự điều phối service (không dùng Windows services), tự sinh virtual host + pretty URL (`app.test`), tạo project 1-click (Laravel/WordPress), cấp SSL local, mở terminal/DB client. Triết lý: nhẹ (<6MB binary, ~4–10MB RAM), portable, isolated.

Người dùng đã chuyển sang **Ubuntu 26.04 LTS** và muốn một app tương đương cho Linux. Stack thực tế (trích từ `laralux.ini` Windows):

- Nginx (port 80/443, SSL bật), upstream PHP-FPM
- PHP 8.4
- MariaDB
- Redis
- Node (qua nvm)
- Mailpit
- Hostname format: `{name}.dev`

## 2. Quyết định kiến trúc (đã chốt với người dùng)

| Hạng mục | Quyết định |
|---|---|
| Quản lý stack | **Native** — dùng binary cài qua apt, KHÔNG dùng Docker, KHÔNG bundle binary portable |
| Điều phối service | **Phương án A** — app tự spawn/quản lý process con; config + data riêng trong `~/laralux/`, không đụng `/etc` hệ thống |
| Giao diện | **GUI desktop + system tray** |
| Framework | **Tauri** (Rust backend + frontend web) — nhẹ, hợp triết lý Laralux |
| Quyền hệ thống | **polkit/sudo khi cần** + `setcap` cho nginx để chạy hằng ngày không cần mật khẩu |
| Pretty URL | `*.dev` (cấu hình được trong `laralux.toml`, mặc định `dev`) |
| SSL | **Auto mkcert** (bắt buộc, vì `.dev` preload HSTS → buộc HTTPS) — nằm trong MVP |
| Phạm vi | Full parity Laralux, chia 3 phase; MVP dùng được hằng ngày |

**Môi trường mục tiêu:** Ubuntu 26.04 (apt + systemd), Rust/cargo 1.96, Node 24 (nvm).

## 3. Kiến trúc tổng thể

```
┌─────────────────────────────────────────────┐
│  Tauri App (GUI + tray)                       │
│  ┌─────────────┐      ┌────────────────────┐ │
│  │ Frontend    │◄────►│ Rust backend (core)│ │
│  │ web (UI)    │ IPC  │  - orchestrator    │ │
│  └─────────────┘      │  - config gen      │ │
│                       │  - privileged ops  │ │
│                       └─────────┬──────────┘ │
└─────────────────────────────────┼────────────┘
                                   │ spawn / signal
        ┌──────────┬───────────┬───┴────┬─────────┐
        ▼          ▼           ▼        ▼         ▼
     nginx     php-fpm     mariadb    redis    mailpit
   (port 80/443, config + data trong ~/laralux, KHÔNG đụng /etc)
```

### Layout thư mục
```
~/laralux/
  www/                # projects (mỗi folder = 1 site)
  etc/                # config do app sinh ra
    nginx/            #   nginx.conf + sites/<name>.conf
    php/<ver>/        #   php.ini + php-fpm pool
    mariadb/          #   my.cnf
  data/               # mariadb datadir, redis dump
  bin/                # symlink tới binary apt + version do app quản lý
  log/                # log mỗi service
  tmp/                # sock, pid
  ssl/                # cert mkcert mỗi site
  laralux.toml        # cấu hình app (thay laralux.ini)
```

### Module (Rust)
- `core::orchestrator` — vòng đời process (start/stop/restart/health), độc lập GUI.
- `core::services` — mỗi service (nginx, php, mariadb, redis, mailpit) là 1 trait impl: sinh config, lệnh chạy, health-check, thứ tự phụ thuộc.
- `core::sites` — quét `www/`, sinh vhost, quản lý `*.dev`, cấp cert.
- `core::privileged` — gom MỌI thao tác cần root vào một polkit helper (bind 80/443, sửa `/etc/hosts`, `apt install`, `setcap`).
- `core::pkg` — cài/đổi version qua apt + ppa:ondrej/php, nvm, tải mailpit/mkcert.
- `ui` (frontend) — chỉ gọi IPC, không chứa logic nghiệp vụ.

**Nguyên tắc:** logic nghiệp vụ nằm hết trong `core` (test được không cần GUI); `privileged` cô lập bề mặt quyền cao vào một chỗ kiểm soát được; phần còn lại chạy quyền user.

## 4. Điều phối service & vòng đời

- Trạng thái mỗi service: `Stopped → Starting → Running → Stopping`, kèm `Crashed`.
- Health-check: nginx `nginx -t` + probe port; mariadb `mysqladmin ping`; redis `PING`; php-fpm probe socket; mailpit probe port.
- App spawn process con, giữ PID trong `tmp/`, log riêng vào `log/`. App thoát → dừng sạch mọi process (không để mồ côi).
- "Start All" tôn trọng thứ tự phụ thuộc: mariadb/redis → php-fpm → nginx.

### Config do app sinh (không đụng /etc)
- **nginx:** `nginx -p ~/laralux/etc/nginx -c .../nginx.conf`; mỗi site 1 file trong `etc/nginx/sites/`.
- **php-fpm:** pool riêng, socket trong `tmp/`, chọn version qua binary `/usr/sbin/php-fpm<ver>`.
- **mariadb:** `--defaults-file=~/laralux/etc/mariadb/my.cnf --datadir=~/laralux/data/mariadb`; lần đầu tự `mariadb-install-db`.
- **redis / mailpit:** config tối giản trong `etc/`.

## 5. Mô hình quyền

Gom vào `core::privileged`, một polkit action + helper nhỏ. Thao tác cần root chỉ gồm:
1. **Bind port 80/443** — `setcap cap_net_bind_service` cho binary nginx (cấp 1 lần) → sau đó nginx chạy quyền user, **không cần sudo mỗi lần start**.
2. **Sửa `/etc/hosts`** thêm `*.dev` (khi chưa bật dnsmasq).
3. **`apt install`** khi cài stack/version.
4. **`setcap`** lúc setup.

→ Chạy/dừng stack hằng ngày **không cần nhập mật khẩu**; chỉ cần lúc setup hoặc cài thêm.

## 6. Pretty URLs & SSL

- Mỗi folder trong `www/` → 1 site `<name>.dev`.
- Mặc định: thêm dòng vào `/etc/hosts` cho mỗi site.
- **SSL bắt buộc** (`.dev` preload HSTS): app cài mkcert CA (1 lần), cấp cert mỗi site vào `ssl/`, nginx phục vụ HTTPS 443. `https://<name>.dev` chạy ngay.
- TLD cấu hình trong `laralux.toml` (mặc định `dev`).
- Nâng cao (Phase 3): dnsmasq wildcard trong `/etc/NetworkManager/dnsmasq.d/` để khỏi sửa hosts từng site.

## 7. Phân phase

### Phase 1 — MVP (dùng được hằng ngày)
- Tray + cửa sổ chính: Start/Stop All, trạng thái từng service.
- Orchestrator + 5 service: nginx, php-fpm (8.4), mariadb, redis, mailpit.
- Quét `www/` → auto vhost + `*.dev` (sửa `/etc/hosts`) + **auto SSL mkcert**.
- `setcap` cho nginx.
- Wizard setup lần đầu: cài stack qua apt, khởi tạo datadir, cài mkcert CA.

### Phase 2 — Version & quick app
- Cài/đổi version PHP (7.4–8.4 qua ppa:ondrej/php), Node (nvm), MariaDB.
- PHP quick settings (xdebug, memory_limit, upload_max_filesize, post_max_size, max_execution_time).
- Quick app 1-click: Laravel (`composer create-project`), WordPress.
- Nút mở: terminal tại site, DB client, thư mục project.

### Phase 3 — Parity đầy đủ
- dnsmasq wildcard.
- PostgreSQL, MongoDB, Memcached.
- Procfile (chạy app tùy chỉnh khi start).
- Share ra ngoài (ngrok/cloudflared), auto-update, đa profile.

### Stack version mặc định
nginx (apt) · PHP 8.4 (ppa:ondrej/php, đa version) · MariaDB (apt) · redis-server (apt) · mailpit (GitHub release binary) · mkcert (apt/binary) · composer.

## 8. Xử lý lỗi

- **Preflight check** mỗi lần start: binary tồn tại? port trống? config valid? datadir khởi tạo chưa? → báo lỗi cụ thể thay vì fail mơ hồ.
- **Port bận:** phát hiện, chỉ rõ PID/tiến trình đang giữ, gợi ý xử lý.
- **Process crash:** đánh dấu `Crashed`, hiện link log, KHÔNG tự loop restart vô hạn.
- **Polkit bị từ chối:** rollback sạch, báo rõ thao tác cần quyền và lý do.
- Mọi lỗi: log chi tiết trong `~/laralux/log/` + thông điệp ngắn trên UI.

## 9. Chiến lược kiểm thử

- `core` tách rời GUI → unit test thuần Rust: sinh config (so output mong đợi), parse `www/`, logic version/port, dựng dòng `/etc/hosts`.
- Integration test orchestrator: start/stop "fake binary" để kiểm vòng đời, health-check, dọn process.
- Privileged & apt: bọc sau trait → mock trong CI, không cần root.
- Smoke test thủ công (checklist): start stack → mở `https://app.dev` → thấy phpinfo.
- Theo **TDD**: viết test trước cho từng module core khi implement.

## 10. Ngoài phạm vi (YAGNI)

- Không Docker, không bundle binary portable.
- Không hỗ trợ distro khác Ubuntu/Debian ở Phase 1 (apt-only).
- Không Apache ở MVP (chỉ nginx; Apache có thể thêm sau nếu cần).
- Không Windows/macOS.
