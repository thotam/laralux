# Chặn thực thi dưới `.well-known` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development hoặc superpowers:executing-plans. Các step dùng checkbox (`- [ ]`).

**Goal:** File thả vào `.well-known` (ở gốc **và** lồng ở mọi độ sâu) không bao giờ được thực thi hay bị trả về dạng source, trong khi ACME challenge và OAuth discovery vẫn hoạt động bình thường.

**Architecture:** Thêm một hằng chuỗi dùng chung trong `core/src/sites.rs`, nhúng vào hai nhánh sinh PHP handler (`vhost_config` block local và `public_vhost_block` block public). Nhánh proxy không đụng tới vì không có `root`/PHP.

**Tech Stack:** Rust (`cargo test`). Không đụng frontend.

## Global Constraints

- KHÔNG thêm `Co-Authored-By` hay chữ ký "Generated with Claude"/🤖 vào commit.
- Guard gồm **hai** rule, cả hai bắt buộc:
  1. `location ^~ /.well-known/` với **hai deny lồng bên trong** (dotfile + đuôi thực thi).
  2. `location ~ /\.well-known/.*\.(php|phar|phtml)$` — regex **KHÔNG neo `^`**.
- Bỏ bất kỳ phần nào trong ba phần trên đều tái sinh một lỗ hổng đã đo được:
  - thiếu deny dotfile lồng → `/.well-known/.env` từ 403 thành **200 lộ nội dung**;
  - thiếu rule regex không neo → `/mcp/.well-known/x.php` **thực thi** (đo được `EXECUTED-NESTED`).
- Không đụng nhánh proxy; không đụng route OAuth phía Laravel.
- ACME token tĩnh và OAuth discovery phải tiếp tục hoạt động.

---

### Task 1: Guard `.well-known` cho cả block local và public

**Files:**
- Modify: `core/src/sites.rs`
- Test: `core/src/sites.rs` (module `#[cfg(test)]` sẵn có)

**Interfaces:**
- Produces: hằng private `WELL_KNOWN_GUARD: &str` trong `core/src/sites.rs`, được nhúng vào chuỗi vhost của cả nhánh PHP local lẫn nhánh PHP public.

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/sites.rs`:

```rust
    #[test]
    fn php_vhost_blocks_execution_under_well_known() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("app").join("public")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        let mut site = scan_sites(&paths, "dev").unwrap().into_iter().find(|s| s.name == "app").unwrap();
        site.public_domains = vec!["app.example.com".to_string()];

        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("app.dev.pem");
        let key = paths.ssl().join("app.dev-key.pem");
        let conf = site.vhost_config(&paths, &sock, &cert, &key);

        // Prefix block: `^~` stops nginx reaching the `\.php$` handler at all.
        assert_eq!(conf.matches("location ^~ /.well-known/").count(), 2,
            "cả block local lẫn block public đều phải có guard");
        // Nested denies: without the dotfile one, `^~` would expose /.well-known/.env.
        assert_eq!(conf.matches("location ~ ^/\\.well-known/(.*/)?\\. { deny all; }").count(), 2);
        assert_eq!(conf.matches("location ~ \\.(php|phar|phtml)$ { deny all; }").count(), 2);
        // `.well-known` nested at any depth — `^~` is anchored and misses these.
        assert_eq!(conf.matches("location ~ /\\.well-known/.*\\.(php|phar|phtml)$ { deny all; }").count(), 2);
        // The nested rule must NOT be anchored, or /mcp/.well-known/x.php executes.
        assert!(!conf.contains("location ~ ^/\\.well-known/.*\\.(php|phar|phtml)$"),
            "rule cho .well-known lồng không được neo ^");
        // OAuth/ACME still reach Laravel.
        assert!(conf.contains("try_files $uri $uri/ /index.php?$query_string;"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn proxy_vhost_has_no_well_known_guard() {
        let root = temp_root();
        let paths = LaraluxPaths::new(root.clone());
        let route = crate::site_registry::ProxyRoute { path: "/".into(), upstream: "127.0.0.1:3000".into() };
        let mut site = proxy_site("api", vec![route], true);
        site.public_domains = vec!["api.example.com".to_string()];
        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("x.pem");
        let key = paths.ssl().join("x-key.pem");

        let conf = site.vhost_config(&paths, &sock, &cert, &key);
        // A proxy has no root and no PHP handler — there is nothing to protect,
        // and a guard here would only add a confusing dead rule.
        assert!(!conf.contains(".well-known"), "nhánh proxy không được có guard");
    }
```

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core sites::tests::php_vhost_blocks_execution_under_well_known`
Expected: FAIL — vhost hiện chưa có chuỗi `location ^~ /.well-known/`.

- [ ] **Step 3: Cài đặt**

Trong `core/src/sites.rs`, thêm hằng ở cấp module (đặt gần đầu file, cạnh các khai báo khác):

```rust
/// Serve `.well-known`, never execute it. ACME clients and OAuth libraries write
/// into this directory, and write access there must not imply code execution.
///
/// Three parts, all load-bearing:
/// - `^~` makes nginx skip the regex locations below, so the `\.php$` handler can
///   never reach anything here — the property is structural, not a matter of
///   declaration order or of the extension list being exhaustive.
/// - The nested dotfile deny restores what `^~` would otherwise bypass: without
///   it `/.well-known/.env` goes from 403 to 200 with its contents.
/// - The trailing rule is deliberately NOT anchored, because `^~` only matches at
///   the start: `/{tenant}/.well-known/…` (the shape OIDC discovery uses) would
///   otherwise still execute.
const WELL_KNOWN_GUARD: &str = "\x20 location ^~ /.well-known/ {\n\
     \x20   location ~ ^/\\.well-known/(.*/)?\\. { deny all; }\n\
     \x20   location ~ \\.(php|phar|phtml)$ { deny all; }\n\
     \x20   try_files $uri $uri/ /index.php?$query_string;\n\
     \x20 }\n\
     \x20 location ~ /\\.well-known/.*\\.(php|phar|phtml)$ { deny all; }\n";
```

Rồi nhúng vào **hai** chỗ, ngay sau dòng deny dotfile đã có.

1. Trong `vhost_config`, nhánh PHP (block local) — đổi:

```rust
             \x20 location ~ /\\.(?!well-known).* {{ deny all; }}\n\
             {build_cache}\
```

thành:

```rust
             \x20 location ~ /\\.(?!well-known).* {{ deny all; }}\n\
             {well_known}\
             {build_cache}\
```

và thêm vào danh sách named args của `format!` đó:

```rust
            well_known = WELL_KNOWN_GUARD,
```

2. Trong `public_vhost_block`, nhánh PHP — làm y hệt: thêm `{well_known}\` ngay sau dòng deny dotfile, và thêm `well_known = WELL_KNOWN_GUARD,` vào named args.

**Không** đụng nhánh proxy ở cả hai hàm.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core sites`
Expected: PASS toàn bộ (các test vhost cũ vẫn xanh — guard chỉ thêm dòng, không đổi dòng nào sẵn có).

- [ ] **Step 5: Kiểm chứng bằng nginx thật (bắt buộc)**

Chỉ assert chuỗi là chưa đủ — lỗ hổng lần trước lọt qua đúng vì thiếu bước này. Chạy laralux, để nó sinh lại vhost, rồi đo:

```bash
# nginx phải parse được
~/laralux/bin/nginx/current/nginx -p ~/laralux/etc/nginx -c ~/laralux/etc/nginx/nginx.conf -t

# đặt file dò ở CẢ HAI vị trí trong public/ của một site PHP
mkdir -p public/.well-known public/mcp/.well-known
echo '<?php echo "EXECUTED";' > public/.well-known/probe.php
echo '<?php echo "EXECUTED";' > public/mcp/.well-known/probe.php
printf 'token' > public/.well-known/acme-token
printf 'SECRET' > public/.well-known/.env
```

Kỳ vọng (thay `<site>.dev` bằng site thật):

| Request | Kỳ vọng |
|---------|---------|
| `/.well-known/probe.php` | **403**, body là trang nginx (không phải source) |
| `/mcp/.well-known/probe.php` | **403** |
| `/.well-known/.env` | **403** |
| `/.well-known/acme-token` | **200**, đúng nội dung |
| `/.well-known/oauth-protected-resource` | tới Laravel (header `x-powered-by: PHP`) |
| `/mcp/.well-known/openid-configuration` | tới Laravel |

Nếu bất kỳ dòng nào lệch, **dừng lại và báo** — đừng sửa test cho khớp.

Dọn sạch file dò sau khi đo xong.

- [ ] **Step 6: Commit**

```bash
git add core/src/sites.rs
git commit -m "fix(sites): never execute or expose files under .well-known"
```

---

## Self-Review

**Spec coverage:**
- §Hướng giải quyết (hai rule, cả hai bắt buộc) → Task 1 Step 3. ✅
- §Dòng deny dotfile bắt buộc → assert riêng trong Step 1 + đo ở Step 5. ✅
- §Rule regex không neo cho `.well-known` lồng → assert riêng + assert phủ định (không được có `^`) + đo ở Step 5. ✅
- §Các thành phần (hai chỗ nhúng: local + public) → assert `count() == 2` cho từng dòng. ✅
- §Proxy không được có guard → `proxy_vhost_has_no_well_known_guard`. ✅
- §Kết quả mong đợi (bảng 6 dòng) → Step 5 lặp lại đúng bảng đó. ✅

**Type consistency:** `WELL_KNOWN_GUARD` khai một lần, dùng qua named arg `well_known` ở cả hai `format!` — tên hằng và tên arg nhất quán giữa hai chỗ nhúng.

**Placeholder scan:** Step 5 yêu cầu đo trên nginx thật thay vì chỉ tin assert chuỗi — đây là yêu cầu cụ thể có bảng kỳ vọng, không phải placeholder. Lý do nó bắt buộc: bản thiết kế đầu của spec này qua được mọi assert chuỗi mà vẫn để `/mcp/.well-known/x.php` thực thi.
