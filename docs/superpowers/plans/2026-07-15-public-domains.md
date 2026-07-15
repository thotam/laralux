# Public domains (tên miền thật qua upstream reverse-proxy) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans để triển khai plan này theo từng task. Các step dùng cú pháp checkbox (`- [ ]`) để theo dõi.

**Goal:** Cho phép mỗi site phục vụ một hoặc nhiều tên miền thật qua HTTP-only (không redirect 301, không `/etc/hosts`, không mkcert), để một server public terminate TLS rồi reverse-proxy HTTP xuống thiết bị chạy laralux.

**Architecture:** Thêm khái niệm `public_domains` tách biệt với local domains ở tầng registry và `Site`. `vhost_config` sinh thêm một nginx server block HTTP cho public domains; `nginx.conf` thêm một `map` để suy `HTTPS` từ header `X-Forwarded-Proto`. `sync_sites` không đổi logic — vì `public_domains` là field riêng nên tự động bị loại khỏi hosts + cert. Tầng UI thêm modal "Public domains" và command Tauri tương ứng.

**Tech Stack:** Rust (core lib + Tauri commands, `cargo test`), TypeScript/Vite frontend (không có test harness — verify bằng `npm run build`).

## ⚠️ Cập nhật thiết kế (443) — đọc trước Task 3–6

Bản plan gốc giả định upstream proxy **HTTP:80** xuống device (public block
HTTP-only, không cert, cần map `$http_x_forwarded_proto`). Người dùng làm rõ
upstream proxy **HTTPS:443** (Let's Encrypt terminate ở upstream). Thiết kế đã
pivot — phần code Task 3–6 ĐÃ được cập nhật trong commit
`refactor(sites): serve public domains on 80+443 …`:

- **Task 3/4 (vhost):** public block `listen 80; listen 443 ssl;` dùng cert của
  site, KHÔNG redirect, `fastcgi_param HTTPS on;` (PHP) / `X-Forwarded-Proto
  https` (proxy). `public_vhost_block` nhận thêm `cert`, `key`.
- **Task 5 (map):** ĐÃ REVERT — không cần `$http_x_forwarded_proto` map.
- **Task 6 (sync):** cert SAN giờ GỒM public domains (`ensure_cert` nhận
  `domains + public_domains`); public domains VẪN loại khỏi `/etc/hosts`.

Chi tiết đầy đủ ở spec `docs/superpowers/specs/2026-07-15-public-domains-design.md`.
Task 1, 2, 7, 8, 9, 10 không đổi.

## Global Constraints

- KHÔNG thêm dòng `Co-Authored-By` hay chữ ký "Generated with Claude"/🤖 vào commit.
- Local domains (`.dev`) giữ nguyên hành vi cũ: block `80→301` + `443 ssl` mkcert + `/etc/hosts`.
- Public domain: HTTP-only, KHÔNG redirect, KHÔNG cert, KHÔNG vào `/etc/hosts`.
- Uniqueness toàn cục: một domain string chỉ được thuộc đúng một chỗ (local hoặc public) trên toàn bộ các site.
- `sites.toml` cũ (chưa có `public_domains`) vẫn phải load được (`#[serde(default)]`).
- Domain validation tái dùng `validate_domain` (backend) và `validDomain` (frontend) đã có.

---

### Task 1: Registry — dữ liệu & method cho `public_domains`

**Files:**
- Modify: `core/src/site_registry.rs`
- Test: `core/src/site_registry.rs` (module `#[cfg(test)]` sẵn có)

**Interfaces:**
- Produces:
  - `struct SitePublicDomains { pub name: String, pub domains: Vec<String> }`
  - `SiteRegistry::public_domains_for(&self, name: &str) -> Option<&[String]>`
  - `SiteRegistry::set_public_domains(&mut self, name: &str, domains: &[String]) -> Result<(), RegistryError>`
  - `remove()` giờ cũng xoá entry trong `public_domains`.

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/site_registry.rs`:

```rust
    #[test]
    fn public_domains_set_get_remove_and_cross_uniqueness() {
        let mut reg = SiteRegistry::default();
        // empty bị từ chối
        assert!(matches!(reg.set_public_domains("a", &[]), Err(RegistryError::NoDomains)));
        // invalid bị từ chối
        assert!(matches!(
            reg.set_public_domains("a", &["Bad".to_string()]),
            Err(RegistryError::InvalidDomain(_))
        ));
        // set + get, có normalize/dedupe
        reg.set_public_domains("a", &["  App.Example.COM ".to_string(), "app.example.com".to_string()]).unwrap();
        assert_eq!(reg.public_domains_for("a").unwrap(), &["app.example.com".to_string()]);

        // local domain của site khác không được trùng public domain đã dùng
        assert!(matches!(
            reg.set_domains("b", &["app.example.com".to_string()]),
            Err(RegistryError::DomainTaken(_))
        ));
        // và ngược lại: public domain không được trùng local domain đã dùng
        reg.set_domains("c", &["c.dev".to_string()]).unwrap();
        assert!(matches!(
            reg.set_public_domains("d", &["c.dev".to_string()]),
            Err(RegistryError::DomainTaken(_))
        ));

        // remove xoá cả public domains
        assert!(reg.remove("a"));
        assert!(reg.public_domains_for("a").is_none());
    }

    #[test]
    fn old_sites_toml_without_public_domains_loads() {
        let r = root();
        std::fs::create_dir_all(&r).unwrap();
        let file = r.join("sites.toml");
        std::fs::write(&file, "[[sites]]\nname = \"blog\"\nroot = \"/tmp/blog\"\n").unwrap();
        let reg = SiteRegistry::load(&file).unwrap();
        assert!(reg.public_domains_for("blog").is_none());
        std::fs::remove_dir_all(&r).ok();
    }
```

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core site_registry::tests::public_domains_set_get_remove_and_cross_uniqueness`
Expected: FAIL — không compile (method `set_public_domains` chưa tồn tại).

- [ ] **Step 3: Cài đặt tối thiểu**

Trong `core/src/site_registry.rs`:

1. Thêm struct sau `SiteDomains`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SitePublicDomains {
    pub name: String,
    pub domains: Vec<String>,
}
```

2. Thêm field vào `SiteRegistry` (sau `domains`):

```rust
    #[serde(default)]
    public_domains: Vec<SitePublicDomains>,
```

3. Thêm một helper private để kiểm tra một domain đã bị site KHÁC chiếm chưa (quét cả `domains` lẫn `public_domains`), rồi thêm các method public:

```rust
    /// True nếu `domain` đang được một site có tên khác `skip` sử dụng,
    /// xét cả local domains lẫn public domains.
    fn domain_taken_by_other(&self, skip: &str, domain: &str) -> bool {
        self.domains.iter().any(|d| d.name != skip && d.domains.iter().any(|x| x == domain))
            || self
                .public_domains
                .iter()
                .any(|d| d.name != skip && d.domains.iter().any(|x| x == domain))
    }

    pub fn public_domains_for(&self, name: &str) -> Option<&[String]> {
        self.public_domains.iter().find(|d| d.name == name).map(|d| d.domains.as_slice())
    }

    pub fn set_public_domains(&mut self, name: &str, domains: &[String]) -> Result<(), RegistryError> {
        let mut norm: Vec<String> = Vec::new();
        for d in domains {
            let d = d.trim().to_ascii_lowercase();
            validate_domain(&d)?;
            if !norm.iter().any(|x| x == &d) {
                norm.push(d);
            }
        }
        if norm.is_empty() {
            return Err(RegistryError::NoDomains);
        }
        for d in &norm {
            if self.domain_taken_by_other(name, d) {
                return Err(RegistryError::DomainTaken(d.clone()));
            }
        }
        self.public_domains.retain(|d| d.name != name);
        self.public_domains.push(SitePublicDomains { name: name.to_string(), domains: norm });
        Ok(())
    }
```

4. Sửa `set_domains` để dùng chung uniqueness check (thay vòng lặp cũ chỉ quét `self.domains`). Đổi khối kiểm tra "reject a domain claimed by a different site" thành:

```rust
        // reject a domain claimed by a *different* site (local hoặc public)
        for d in &norm {
            if self.domain_taken_by_other(name, d) {
                return Err(RegistryError::DomainTaken(d.clone()));
            }
        }
```

5. Sửa `remove()` để tính cả `public_domains`:

```rust
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.sites.len() + self.proxies.len() + self.domains.len() + self.public_domains.len();
        self.sites.retain(|s| s.name != name);
        self.proxies.retain(|p| p.name != name);
        self.domains.retain(|d| d.name != name);
        self.public_domains.retain(|d| d.name != name);
        self.sites.len() + self.proxies.len() + self.domains.len() + self.public_domains.len() != before
    }
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core site_registry`
Expected: PASS toàn bộ (kể cả các test cũ như `set_domains_validates_uniqueness_and_roundtrips`).

- [ ] **Step 5: Commit**

```bash
git add core/src/site_registry.rs
git commit -m "feat(registry): add per-site public_domains with cross-list uniqueness"
```

---

### Task 2: Site model — field `public_domains` + `list_all_sites`

**Files:**
- Modify: `core/src/sites.rs`
- Test: `core/src/sites.rs` (module test sẵn có)

**Interfaces:**
- Consumes: `SiteRegistry::public_domains_for` (Task 1).
- Produces: `Site.public_domains: Vec<String>` — mọi builder của `Site` phải set field này (mặc định `Vec::new()`).

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/sites.rs`:

```rust
    #[test]
    fn list_all_populates_public_domains_from_registry() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www").join("demo")).unwrap();
        let paths = LaraluxPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        reg.set_public_domains("demo", &["app.example.com".to_string()]).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let (sites, _w) = list_all_sites(&paths, "dev").unwrap();
        let demo = sites.iter().find(|s| s.name == "demo").unwrap();
        // local domain giữ nguyên
        assert_eq!(demo.domains, vec!["demo.dev".to_string()]);
        // public domain điền từ registry
        assert_eq!(demo.public_domains, vec!["app.example.com".to_string()]);
        std::fs::remove_dir_all(&root).ok();
    }
```

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core sites::tests::list_all_populates_public_domains_from_registry`
Expected: FAIL — `Site` chưa có field `public_domains` (không compile).

- [ ] **Step 3: Cài đặt tối thiểu**

Trong `core/src/sites.rs`:

1. Thêm field vào struct `Site` (sau `proxy`):

```rust
    pub public_domains: Vec<String>,
```

2. Trong `scan_sites`, thêm `public_domains: Vec::new(),` vào literal `Site { … }`.

3. Trong `list_all_sites`, ở cả hai chỗ push `Site` (nhánh Linked và nhánh Proxy) thêm `public_domains: Vec::new(),`.

4. Trong `list_all_sites`, ngay sau vòng lặp áp `domains` override (`for s in sites.iter_mut() { … domains_for … }`), thêm vòng điền public domains:

```rust
    for s in sites.iter_mut() {
        if let Some(pd) = registry.public_domains_for(&s.name) {
            s.public_domains = pd.to_vec();
        }
    }
```

5. Sửa các test cũ đang khởi tạo `Site { … }` bằng struct literal đầy đủ (`proxy_site` helper, `vhost_server_name_lists_all_domains`) để thêm `public_domains: Vec::new(),` — nếu không sẽ lỗi compile "missing field".

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core sites`
Expected: PASS toàn bộ.

- [ ] **Step 5: Commit**

```bash
git add core/src/sites.rs
git commit -m "feat(sites): add public_domains field populated from registry"
```

---

### Task 3: `vhost_config` — public HTTP block cho site PHP

**Files:**
- Modify: `core/src/sites.rs` (method `Site::vhost_config`)
- Test: `core/src/sites.rs`

**Interfaces:**
- Consumes: `Site.public_domains` (Task 2), `$lara_fwd_https` (biến nginx, định nghĩa ở Task 5 — chỉ là tên biến trong output string).
- Produces: output `vhost_config` có thêm một `server { listen 80; … }` khi `public_domains` không rỗng.

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/sites.rs`:

```rust
    #[test]
    fn vhost_php_public_block_is_http_only_no_redirect() {
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

        // block local cũ vẫn còn
        assert!(conf.contains("server_name app.dev;"));
        assert!(conf.contains("listen 443 ssl;"));
        // block public: HTTP-only, có server_name domain thật
        assert!(conf.contains("server_name app.example.com;"));
        // public block KHÔNG redirect và dùng X-Forwarded-Proto cho HTTPS param
        assert!(conf.contains("fastcgi_param HTTPS $lara_fwd_https;"));
        // domain public không được nằm trong block 443 (không cấp cert cho nó)
        // -> đảm bảo server_name 443 chỉ chứa domain local
        assert!(!conf.contains("server_name app.example.com app.dev;"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn vhost_no_public_block_when_public_domains_empty() {
        let root = temp_root();
        let www = root.join("www");
        std::fs::create_dir_all(www.join("app").join("public")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        let site = scan_sites(&paths, "dev").unwrap().into_iter().find(|s| s.name == "app").unwrap();
        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("app.dev.pem");
        let key = paths.ssl().join("app.dev-key.pem");
        let conf = site.vhost_config(&paths, &sock, &cert, &key);
        assert!(!conf.contains("example.com"));
        std::fs::remove_dir_all(&root).ok();
    }
```

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core sites::tests::vhost_php_public_block_is_http_only_no_redirect`
Expected: FAIL — output chưa chứa `server_name app.example.com;`.

- [ ] **Step 3: Cài đặt tối thiểu**

Trong `core/src/sites.rs::vhost_config`, ngay TRƯỚC câu lệnh `format!(…)` cuối cùng (block PHP local), tạo sẵn phần public rồi nối vào kết quả. Cách làm gọn nhất: bọc phần return cuối thành biến `let local = format!(…);` rồi `return format!("{local}{public}")` với `public` build từ helper. Thêm helper method vào `impl Site`:

```rust
    /// Nếu có public domains, sinh một server block HTTP-only (không redirect,
    /// không TLS). HTTPS param suy từ `$lara_fwd_https` (map trong nginx.conf).
    fn public_vhost_block(&self, paths: &LaraluxPaths, php_socket: &std::path::Path) -> String {
        if self.public_domains.is_empty() {
            return String::new();
        }
        let names = self.public_domains.join(" ");
        let alog = paths.log().join(format!("{}-public-access.log", self.name)).display().to_string();
        let elog = paths.log().join(format!("{}-public-error.log", self.name)).display().to_string();

        // Proxy-site: mirror routes over plain HTTP.
        if let Some(spec) = &self.proxy {
            let mut locations = String::new();
            for r in &spec.routes {
                locations.push_str(&format!(
                    "\x20 location {path} {{\n\
                     \x20   proxy_pass http://{up};\n\
                     \x20   proxy_http_version 1.1;\n\
                     \x20   proxy_set_header Host $host;\n\
                     \x20   proxy_set_header X-Real-IP $remote_addr;\n\
                     \x20   proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n\
                     \x20   proxy_set_header X-Forwarded-Proto $scheme;\n",
                    path = r.path,
                    up = r.upstream,
                ));
                if spec.websocket {
                    locations.push_str(
                        "\x20   proxy_set_header Upgrade $http_upgrade;\n\
                         \x20   proxy_set_header Connection $connection_upgrade;\n",
                    );
                }
                locations.push_str("\x20 }\n");
            }
            return format!(
                "server {{\n\
                 \x20 listen 80;\n\
                 \x20 server_name {names};\n\
                 \x20 access_log {alog};\n\
                 \x20 error_log {elog};\n\
                 {locations}\
                 }}\n",
            );
        }

        // PHP site.
        let is_laravel = self.root.join("artisan").is_file();
        let build_cache = if is_laravel {
            "\x20 location ^~ /build/ {\n\
             \x20   expires 1y;\n\
             \x20   add_header Cache-Control \"public, immutable\";\n\
             \x20   try_files $uri =404;\n\
             \x20 }\n"
        } else {
            ""
        };
        format!(
            "server {{\n\
             \x20 listen 80;\n\
             \x20 server_name {names};\n\
             \x20 root {docroot};\n\
             \x20 index index.php index.html;\n\
             \x20 access_log {alog};\n\
             \x20 error_log {elog};\n\
             \x20 location ~ /\\.(?!well-known).* {{ deny all; }}\n\
             {build_cache}\
             \x20 location / {{ try_files $uri $uri/ /index.php?$query_string; }}\n\
             \x20 location ~ \\.php$ {{\n\
             \x20   fastcgi_pass unix:{sock};\n\
             \x20   fastcgi_index index.php;\n\
             \x20   include {nginx_etc}/fastcgi_params;\n\
             \x20   fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n\
             \x20   fastcgi_param HTTPS $lara_fwd_https;\n\
             \x20 }}\n\
             }}\n",
            docroot = self.document_root().display(),
            sock = php_socket.display(),
            nginx_etc = paths.etc_for("nginx").display(),
        )
    }
```

Rồi ở cuối `vhost_config`: trong nhánh proxy (`if let Some(spec) = &self.proxy { … return format!(…) }`) đổi `return format!(…)` thành:

```rust
            let local = format!( /* … giữ nguyên nội dung format! cũ … */ );
            return format!("{local}{}", self.public_vhost_block(paths, php_socket));
```

Và ở block PHP cuối cùng, đổi `format!(…)` thành:

```rust
        let local = format!( /* … giữ nguyên nội dung format! cũ … */ );
        format!("{local}{}", self.public_vhost_block(paths, php_socket))
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core sites`
Expected: PASS toàn bộ (các test vhost cũ vẫn đúng vì local block không đổi).

- [ ] **Step 5: Commit**

```bash
git add core/src/sites.rs
git commit -m "feat(sites): emit HTTP-only public vhost block for public domains"
```

---

### Task 4: `vhost_config` — public block cho proxy-site (test riêng)

**Files:**
- Modify: `core/src/sites.rs` (chỉ thêm test; code đã làm ở Task 3)
- Test: `core/src/sites.rs`

**Interfaces:**
- Consumes: `public_vhost_block` (Task 3), helper `proxy_site` trong module test.

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test:

```rust
    #[test]
    fn proxy_site_public_block_uses_http_proxy_pass() {
        let root = temp_root();
        let paths = LaraluxPaths::new(root.clone());
        let route = crate::site_registry::ProxyRoute { path: "/".into(), upstream: "127.0.0.1:3000".into() };
        let mut site = proxy_site("api", vec![route], true);
        site.public_domains = vec!["api.example.com".to_string()];
        let sock = paths.tmp().join("php-fpm.sock");
        let cert = paths.ssl().join("x.pem");
        let key = paths.ssl().join("x-key.pem");

        let conf = site.vhost_config(&paths, &sock, &cert, &key);
        // block public HTTP cho proxy-site
        assert!(conf.contains("server_name api.example.com;"));
        assert!(conf.contains("proxy_pass http://127.0.0.1:3000;"));
        // ws headers vẫn có (websocket = true)
        assert!(conf.contains("proxy_set_header Upgrade $http_upgrade;"));
        // public block không có fastcgi
        assert!(!conf.contains("fastcgi_param HTTPS $lara_fwd_https;"));
    }
```

Lưu ý: `proxy_site` helper hiện khởi tạo `Site { … }` — Task 2 đã thêm `public_domains: Vec::new()` vào helper này nên `site.public_domains = …` gán được.

- [ ] **Step 2: Chạy test để xác nhận fail/pass**

Run: `cargo test -p laralux-core sites::tests::proxy_site_public_block_uses_http_proxy_pass`
Expected: PASS ngay (code đã có ở Task 3). Nếu FAIL, sửa `public_vhost_block` nhánh proxy cho khớp.

- [ ] **Step 3: Commit**

```bash
git add core/src/sites.rs
git commit -m "test(sites): cover public vhost block for proxy sites"
```

---

### Task 5: `nginx.conf` — thêm `map $http_x_forwarded_proto`

**Files:**
- Modify: `core/src/service/nginx.rs`
- Test: `core/src/service/nginx.rs`

**Interfaces:**
- Produces: biến nginx `$lara_fwd_https` (dùng bởi output của Task 3).

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/service/nginx.rs` (theo mẫu test `map $http_upgrade` ở dòng ~265):

```rust
    #[test]
    fn nginx_conf_has_forwarded_proto_map() {
        let p = test_paths();
        write_conf(&p); // dùng đúng helper mà các test khác trong file này đang dùng để sinh conf
        let conf = std::fs::read_to_string(p.etc_for("nginx").join("nginx.conf")).unwrap();
        assert!(conf.contains("map $http_x_forwarded_proto $lara_fwd_https"));
    }
```

Lưu ý cho người thực thi: xem test `map $http_upgrade $connection_upgrade` ở gần dòng 264-265 để copy đúng cách khởi tạo `paths` và cách gọi hàm sinh conf trong file này (tên helper có thể là `NginxService::new(...).write_config(&p)` hoặc tương tự — dùng đúng pattern đang có, KHÔNG bịa hàm `test_paths`/`write_conf` nếu chúng không tồn tại).

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core service::nginx::tests::nginx_conf_has_forwarded_proto_map`
Expected: FAIL — conf chưa chứa map mới.

- [ ] **Step 3: Cài đặt tối thiểu**

Trong `core/src/service/nginx.rs`, ngay sau dòng `map $http_upgrade $connection_upgrade …` (dòng ~121) thêm một dòng vào chuỗi `format!`:

```rust
             \x20 map $http_x_forwarded_proto $lara_fwd_https {{ default ''; https on; }}\n\
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core service::nginx`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/service/nginx.rs
git commit -m "feat(nginx): add X-Forwarded-Proto -> HTTPS map for public domains"
```

---

### Task 6: Sync — kiểm chứng public domains không lọt vào hosts + cert

**Files:**
- Modify: `core/src/sync.rs` (chỉ thêm test — logic không cần đổi vì `public_domains` là field riêng, `sync_sites` chỉ dùng `site.domains` cho hosts/cert)
- Test: `core/src/sync.rs`

**Interfaces:**
- Consumes: `FakeCertIssuer::requested` (ghi lại các basename+names được cấp cert), `Privileged` fake ghi lại hosts writes.

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/sync.rs` (theo mẫu `sync_splits_explicit_hosts_and_wildcard_bases`):

```rust
    #[test]
    fn public_domains_excluded_from_hosts_and_cert() {
        let r = temp_root();
        std::fs::create_dir_all(r.join("www").join("demo")).unwrap();
        let paths = LaraluxPaths::new(r.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        reg.set_public_domains("demo", &["app.example.com".into()]).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let sock = paths.tmp().join("php.sock");
        let hosts_path = r.join("hosts");
        std::fs::write(&hosts_path, "127.0.0.1 localhost\n").unwrap();
        let issuer = FakeCertIssuer::new(paths.ssl());
        let requested = issuer.requested();
        let priv_ = /* fake Privileged theo đúng mẫu các test khác trong file */ ;

        sync_sites(&paths, "dev", &sock, &hosts_path, &issuer, &priv_).unwrap();

        // cert chỉ cho local domain, không cho public
        let req = requested.lock().unwrap();
        assert!(req.iter().all(|(_, names)| !names.iter().any(|n| n == "app.example.com")));
        assert!(req.iter().any(|(_, names)| names.iter().any(|n| n == "demo.dev")));

        // hosts không chứa domain public
        let writes = priv_.hosts_writes();
        let w = writes.lock().unwrap();
        assert!(w.iter().all(|h| !h.contains("app.example.com")));
        std::fs::remove_dir_all(&r).ok();
    }
```

Lưu ý người thực thi: dùng đúng tên helper tạo `temp_root`, fake `Privileged`, và cách lấy `hosts_writes()` như các test hiện có trong `core/src/sync.rs` (ví dụ `writes_vhosts_certs_and_hosts_block`). Thay đoạn `/* … */` cho khớp.

- [ ] **Step 2: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core sync::tests::public_domains_excluded_from_hosts_and_cert`
Expected: PASS ngay (không cần sửa code sync). Nếu FAIL vì lý do khác (helper), sửa test cho khớp mẫu.

- [ ] **Step 3: Commit**

```bash
git add core/src/sync.rs
git commit -m "test(sync): public domains stay out of /etc/hosts and mkcert"
```

---

### Task 7: Tauri command `set_site_public_domains`

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs` (đăng ký handler)

**Interfaces:**
- Consumes: `SiteRegistry::set_public_domains` (Task 1).
- Produces: command `set_site_public_domains(name, domains) -> Result<SetDomainsResult, String>` (tái dùng struct `SetDomainsResult`).

- [ ] **Step 1: Thêm command**

Trong `src-tauri/src/commands.rs`, thêm ngay sau `set_site_domains` (kết thúc ~dòng 794) một command song song — giống hệt nhưng gọi `registry.set_public_domains`:

```rust
#[tauri::command]
pub async fn set_site_public_domains(
    app: tauri::AppHandle,
    name: String,
    domains: Vec<String>,
) -> Result<SetDomainsResult, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<SetDomainsResult, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry = SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        registry.set_public_domains(&name, &domains).map_err(|e| e.to_string())?;
        registry.save(&state.paths.sites_file()).map_err(|e| e.to_string())?;

        let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
        let issuer = MkcertIssuer::resolved(&state.paths);
        let privileged = PkexecPrivileged;
        let outcome = sync_sites(
            &state.paths, &config.tld, &php_socket,
            std::path::Path::new("/etc/hosts"), &issuer, &privileged,
        );
        let bases = outcome.as_ref().map(|o| o.wildcard_bases.clone()).unwrap_or_default();
        let warnings = apply_wildcard_dns(&state, &bases);
        {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            let _ = orch.reload(ServiceKind::Nginx);
        }
        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        Ok(SetDomainsResult { sites, warnings })
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 2: Đăng ký handler**

Trong `src-tauri/src/main.rs`, thêm `commands::set_site_public_domains,` ngay sau dòng `commands::set_site_domains,` (dòng ~83) trong `generate_handler![ … ]`.

- [ ] **Step 3: Build để xác minh**

Run: `cargo build -p laralux` (hoặc `cargo check --workspace`)
Expected: build thành công, không lỗi.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(tauri): add set_site_public_domains command"
```

---

### Task 8: Frontend — IPC binding

**Files:**
- Modify: `src/ipc/commands.ts`

**Interfaces:**
- Consumes: command `set_site_public_domains` (Task 7), type `SetDomainsResult` (đã có).
- Produces: `setSitePublicDomains(name, domains) -> Promise<SetDomainsResult>`.

- [ ] **Step 1: Thêm binding**

Trong `src/ipc/commands.ts`, ngay sau `setSiteDomains` (~dòng 136):

```ts
/**
 * Set public (real) domains for a site — served HTTP-only for an upstream
 * reverse-proxy. Arg keys: `name`, `domains`.
 * Returns { sites, warnings }.
 */
export const setSitePublicDomains = (
  name: string,
  domains: string[],
): Promise<SetDomainsResult> =>
  invoke<SetDomainsResult>("set_site_public_domains", { name, domains });
```

- [ ] **Step 2: Typecheck**

Run: `npm run build`
Expected: `tsc --noEmit` pass, vite build thành công.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/commands.ts
git commit -m "feat(ipc): add setSitePublicDomains binding"
```

---

### Task 9: Frontend — state + modal + wiring + badge

**Files:**
- Modify: `src/state.ts` (thêm modal kind + slice `sitePublicDomains`)
- Create: `src/ui/modals/publicdomains.ts`
- Modify: `src/ui/render.ts` (route modal mới)
- Modify: `src/ui/views/sites.ts` (row-menu item, badge, `openPublicDomains`/`submitPublicDomains`, `addPublicDomainRow`/`delPublicDomainRow`)
- Modify: `src/ui/events.ts` (handlers `pd-*`, Escape, Tab focus-trap)

**Interfaces:**
- Consumes: `setSitePublicDomains` (Task 8), `SiteDomainsState` type (tái dùng), `Site.public_domains` (field từ backend, đã có trong `Site` type nếu `Site` phản chiếu struct — kiểm tra `src/ipc/types.ts`).

- [ ] **Step 1: State — thêm modal kind + slice**

Trong `src/state.ts`:
1. Thêm `"publicdomains"` vào union `modal` (dòng ~100):

```ts
  modal: null | "newsite" | "linksite" | "proxy" | "domains" | "publicdomains" | "deletesite" | "procs" | ToolModalState;
```

2. Thêm field vào `AppState` (cạnh `siteDomains: SiteDomainsState;`, dòng ~117):

```ts
  sitePublicDomains: SiteDomainsState;
```

3. Khởi tạo giá trị mặc định (cạnh `siteDomains: { … }`, dòng ~152):

```ts
  sitePublicDomains: { name: "", domains: [""], busy: false, error: "" },
```

4. Trong `src/ipc/types.ts`, đảm bảo interface `Site` có `public_domains: string[];` (thêm nếu thiếu — xem cạnh `domains: string[];` dòng ~152). Thêm:

```ts
  public_domains: string[];
```

- [ ] **Step 2: Modal component**

Tạo `src/ui/modals/publicdomains.ts` (mirror `domains.ts`, đổi prefix action sang `pd-`, dùng slice `sitePublicDomains`):

```ts
import { state } from "../../state";
import { esc } from "../util";
import { I } from "../icons";

export function publicDomainsModal(): string {
  const sd = state.sitePublicDomains;
  const hasAny = sd.domains.some((d: string) => d.trim().length > 0);
  const errorHtml = sd.error ? '<div class="ns-error">' + esc(sd.error) + '</div>' : '';
  const d = sd.busy ? ' disabled' : '';
  const rows = sd.domains.map((v: string, i: number) =>
    '<div class="pr-row" data-key="pdom-' + i + '">' +
    '<input class="ns-input" type="text" placeholder="app.example.com" value="' + esc(v) + '" autocomplete="off" spellcheck="false" data-action="pd-input" data-idx="' + i + '"' + d + ' />' +
    (sd.domains.length > 1 ? '<button class="icon-btn sq32" data-action="pd-del" data-idx="' + i + '" aria-label="Remove domain"' + d + '>' + I.close + '</button>' : '') +
    '</div>'
  ).join('');
  const submitLabel = sd.busy
    ? '<span class="spin spinner on-primary"></span>Saving…'
    : 'Save';
  return (
    '<div class="ns-overlay" data-action="pd-overlay-click" role="dialog" aria-modal="true" aria-labelledby="pd-title">' +
    '<div class="ns-card" role="document">' +
    '<div class="ns-head"><h2 class="ns-title" id="pd-title">Public domains — ' + esc(sd.name) + '</h2>' +
    '<button class="icon-btn" data-action="pd-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
    '<div class="ns-body">' +
    '<label class="ns-label">Served HTTP-only for an upstream reverse-proxy (TLS terminated upstream). No local HTTPS, no /etc/hosts.</label>' +
    rows +
    '<button class="link-btn" data-action="pd-add"' + d + '>+ Add domain</button>' +
    errorHtml +
    '</div>' +
    '<div class="ns-foot">' +
    '<button class="btn btn-outline" data-action="pd-close"' + d + '>Cancel</button>' +
    '<button class="btn btn-primary' + (!hasAny || sd.busy ? ' btn-dim' : '') + '" data-action="pd-submit"' + (!hasAny || sd.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
    '</div></div></div>'
  );
}
```

- [ ] **Step 3: Route modal trong render**

Trong `src/ui/render.ts`:
1. Thêm import cạnh import `domainsModal` (dòng ~19):

```ts
import { publicDomainsModal } from "./modals/publicdomains";
```

2. Thêm nhánh route cạnh `state.modal === "domains" ? domainsModal()` (dòng ~187):

```ts
    : state.modal === "publicdomains" ? publicDomainsModal()
```

- [ ] **Step 4: View helpers + row menu + badge**

Trong `src/ui/views/sites.ts`:
1. Thêm import `setSitePublicDomains` vào dòng import từ `../../ipc/commands` (cùng chỗ `setSiteDomains`).
2. Sau `openDomains`/`submitDomains` (~dòng 249-268) thêm bộ helper song song:

```ts
export function openPublicDomains(site: Site): void {
  const ds = (site.public_domains && site.public_domains.length) ? site.public_domains.slice() : [""];
  state.sitePublicDomains = { name: site.name, domains: ds, busy: false, error: "" };
  state.modal = "publicdomains";
  state.rowMenu = null;
  render();
}
export function closePublicDomains(): void { state.modal = null; render(); }
export function addPublicDomainRow(): void { state.sitePublicDomains.domains.push(""); render(); }
export function delPublicDomainRow(i: number): void {
  state.sitePublicDomains.domains.splice(i, 1);
  if (!state.sitePublicDomains.domains.length) state.sitePublicDomains.domains.push("");
  render();
}
export async function submitPublicDomains(): Promise<void> {
  const sd = state.sitePublicDomains;
  const domains = sd.domains.map((d: string) => d.trim()).filter((d: string) => d.length);
  if (!domains.length) { sd.error = "Add at least one domain"; render(); return; }
  for (const d of domains) { if (!validDomain(d)) { sd.error = "Invalid domain: " + d; render(); return; } }
  sd.busy = true; sd.error = ""; render();
  try {
    const res = await setSitePublicDomains(sd.name, domains);
    state.sites = res.sites;
    toast({ type: "success", title: "Public domains updated", msg: domains.join(", ") });
    state.modal = null;
  } catch (e) {
    sd.error = String(e);
  } finally {
    sd.busy = false; render();
  }
}
```

Lưu ý: khớp chính xác cách `submitDomains` gọi `toast`/gán `state.sites`/`res.warnings` — copy đúng pattern hàng xóm (dòng ~258-270), kể cả xử lý `res.warnings` nếu có.

3. Row menu (~dòng 65): thêm mục "Public domains" ngay sau mục "Domains":

```ts
            '<button class="row-menu-item" data-action="edit-public-domains" data-name="' + esc(s.name) + '">Public domains</button>' +
```

4. Badge: tại chỗ render subrow/hostname của site (tìm nơi hiển thị `s.hostname`/domain trong `sitesView`), thêm badge khi `s.public_domains?.length`:

```ts
            (s.public_domains && s.public_domains.length
              ? '<span class="badge badge-public" title="' + esc(s.public_domains.join(", ")) + '">public</span>'
              : "")
```

(Đặt badge cạnh tên site; nếu chưa có class `.badge`, dùng class sẵn có gần nhất hoặc thêm ở Step 6.)

- [ ] **Step 5: Events wiring**

Trong `src/ui/events.ts`:
1. Import các helper mới từ `./views/sites` (cùng chỗ import `openDomains` etc.): `openPublicDomains, closePublicDomains, addPublicDomainRow, delPublicDomainRow, submitPublicDomains`.
2. Row-menu click (cạnh `a === "edit-domains"`, dòng ~115):

```ts
    else if (a === "edit-public-domains") { const s = state.sites.find((s) => s.name === el.getAttribute("data-name")); if (s) openPublicDomains(s); }
```

3. Input/add/del/submit/close/overlay handlers (cạnh các handler `dm-*`):

```ts
    if (el.dataset.action === "pd-input") { state.sitePublicDomains.domains[parseInt(el.dataset.idx!, 10)] = el.value; }
```

và trong khối click handler thêm các nhánh cho `pd-add` → `addPublicDomainRow()`, `pd-del` → `delPublicDomainRow(parseInt(idx))`, `pd-submit` → `submitPublicDomains()`, `pd-close`/`pd-overlay-click` → `closePublicDomains()` — theo đúng cách các action `dm-*` được phân nhánh trong file.

4. Escape (dòng ~214): thêm `else if (e.key === "Escape" && state.modal === "publicdomains") closePublicDomains();`
5. Tab focus-trap (dòng ~223): thêm `"publicdomains"` vào danh sách modal được trap:

```ts
    if (e.key !== "Tab" || (state.modal !== "newsite" && state.modal !== "linksite" && state.modal !== "proxy" && state.modal !== "domains" && state.modal !== "publicdomains" && state.modal !== "deletesite")) return;
```

- [ ] **Step 6: (Tuỳ chọn) CSS badge**

Nếu class `.badge-public` chưa tồn tại, thêm vào `src/styles.css` một style nhỏ (theo tông màu sẵn có):

```css
.badge-public { display: inline-block; margin-left: 6px; padding: 1px 6px; border-radius: 6px; font-size: 11px; background: var(--accent-soft, #e8f0ff); color: var(--accent, #2563eb); }
```

- [ ] **Step 7: Typecheck + build**

Run: `npm run build`
Expected: `tsc --noEmit` pass, vite build thành công. Sửa mọi lỗi type (thường là thiếu import hoặc field `public_domains` chưa khai trong `Site`).

- [ ] **Step 8: Commit**

```bash
git add src/state.ts src/ipc/types.ts src/ui/modals/publicdomains.ts src/ui/render.ts src/ui/views/sites.ts src/ui/events.ts src/styles.css
git commit -m "feat(ui): public domains modal, row action and badge"
```

---

### Task 10: Docs — hướng dẫn cấu hình server public

**Files:**
- Modify: `README.md` (hoặc tạo `docs/public-domains.md` và link từ README)

**Interfaces:** none.

- [ ] **Step 1: Viết mục docs**

Thêm một mục vào `README.md` (hoặc tạo `docs/public-domains.md`):

````markdown
## Public domains (serve a real domain via an upstream reverse-proxy)

Each site can serve one or more real domains over **plain HTTP**, for when a
public server terminates TLS and reverse-proxies down to the device running
laralux. Add them via the site's row menu → **Public domains**.

laralux serves public domains on port 80 with no HTTPS redirect, no mkcert
certificate, and no `/etc/hosts` entry — TLS is the upstream server's job.

On the **upstream public server** (example nginx), terminate TLS and forward:

```nginx
server {
  listen 443 ssl;
  server_name app.example.com;
  # ssl_certificate ... (e.g. Let's Encrypt)

  location / {
    proxy_pass http://<device-ip>:80;
    proxy_set_header Host              $host;
    proxy_set_header X-Real-IP         $remote_addr;
    proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto https;
  }
}
```

`X-Forwarded-Proto: https` makes laralux set `HTTPS=on` for PHP, so Laravel
generates correct `https://` URLs. In the Laravel app, configure
[`TrustProxies`](https://laravel.com/docs/requests#configuring-trusted-proxies)
to trust the upstream so `X-Forwarded-*` headers are honoured.
````

- [ ] **Step 2: Commit**

```bash
git add README.md docs/public-domains.md
git commit -m "docs: public domains upstream reverse-proxy guide"
```

---

## Self-Review

**Spec coverage:**
- §1 Data model → Task 1. ✅
- §2 Site model → Task 2. ✅
- §3 Nginx vhost (PHP + proxy) → Task 3, Task 4. ✅
- §4 nginx.conf map → Task 5. ✅
- §5 Sync (exclude hosts + cert) → Task 6. ✅
- §6 Command layer → Task 7. ✅
- §7 UI → Task 8 (IPC), Task 9 (state/modal/wiring/badge). ✅
- §8 Docs → Task 10. ✅
- Testing (spec) → phủ trong Task 1–6. ✅

**Type consistency:**
- `set_public_domains` / `public_domains_for` dùng nhất quán Task 1→2→7.
- `Site.public_domains: Vec<String>` (Rust) ↔ `public_domains: string[]` (TS) khớp qua serde (snake_case).
- Biến nginx `$lara_fwd_https` định nghĩa ở Task 5, tiêu thụ ở Task 3 — khớp tên.
- `SetDomainsResult` tái dùng cho command mới (Task 7) và binding (Task 8).
- Action prefix `pd-*` dùng nhất quán trong modal (Task 9 Step 2), events (Step 5).

**Placeholder scan:** Task 5 và Task 6 chủ ý yêu cầu người thực thi khớp helper test sẵn có trong file thay vì bịa tên hàm — đã ghi rõ cảnh báo, không phải placeholder logic.
