# Danh sách version lấy từ nguồn - Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (khuyến nghị) hoặc superpowers:executing-plans để triển khai theo từng task. Các step dùng checkbox (`- [ ]`).

**Goal:** Catalog version của cả 10 tool trong Setup lấy từ feed của nguồn binary, mỗi nhánh 3 patch mới nhất, có cache 24h và danh sách cứng dự phòng.

**Architecture:** Module mới `core/src/version_feed.rs` lo phần chung: chọn version (`select`), cache, `refresh`, `catalog_versions` và bảng nguồn theo tool. Mỗi module `*_static.rs` (và `php_cli.rs`, `php_static.rs`) chỉ thêm hàm parse thuần cho feed của mình. Command `tool_versions` gọi `refresh` cho đúng tool đang mở.

**Tech Stack:** Rust (`cargo test` trong `core/`), `serde_json`. Không đụng frontend.

**Spec:** `docs/superpowers/specs/2026-10-22-remote-version-catalogs-design.md`

## Global Constraints

- KHÔNG thêm `Co-Authored-By` hay chữ ký "Generated with Claude"/🤖 vào commit. Commit message tiếng Anh.
- Mỗi nhánh giữ **3** patch (`PATCHES_PER_LINE = 3`). Nguồn không có thông tin hỗ trợ: **4** nhánh mới nhất (`DEFAULT_LINES = 4`).
- Cache: `tmp/feeds/<key>.json`, lưu `Feed` đã parse, TTL **24h**, ghi `*.part` rồi rename.
- Tải lỗi, parse lỗi hoặc ra rỗng: giữ cache cũ. Không báo lỗi lên UI.
- Tải feed qua `Downloader::fetch_feed`, bản curl có `--max-time 20`.
- Bỏ pre-release: chỉ nhận version mà mọi phần tách bởi `.` đều là số.
- PHP vẫn là catalog **theo minor** (`8.4`).
- Danh sách cứng `KNOWN_*_VERSIONS` giữ nguyên tên và vai trò dự phòng; các test `KNOWN_*.contains(&*_VERSION)` hiện có phải vẫn pass.

## Review Focus

1. Bản đã cài nằm ngoài feed (ví dụ `mongodb 7.0.15` khi feed chỉ còn `7.0.45`): vẫn phải hiện với cờ `installed`. Test ở Task 2 (`catalog_keeps_installed_outside_feed`).
2. File cache hỏng (JSON cắt ngang): coi như không có cache, dùng danh sách cứng, không panic. Test ở Task 2 (`corrupt_cache_falls_back_to_known`).
3. GitHub trả lỗi rate limit `{"message":"API rate limit exceeded..."}` với HTTP 200/403: parse ra rỗng, không ghi đè cache. Test ở Task 6 (`github_rate_limit_body_parses_empty`).
4. Feed lẫn binary arch khác (nginx index có cả `aarch64`): chỉ lấy đúng arch. Test ở Task 4 (`nginx_feed_filters_arch_and_os`).
5. Node: nhánh Current lẻ cũ (25) không được lọt vào khi đã có Current mới hơn (26). Test ở Task 3 (`node_feed_allows_lts_and_newest_current`).

---

### Task 1: Hàm chọn version thuần + `fetch_feed` có timeout

**Files:**
- Create: `core/src/version_feed.rs`
- Modify: `core/src/lib.rs` (thêm `pub mod version_feed;`)
- Modify: `core/src/setup.rs` (trait `Downloader`, `CurlDownloader`)

**Interfaces:**
- Produces:
  - `pub const PATCHES_PER_LINE: usize = 3; pub const DEFAULT_LINES: usize = 4;`
  - `#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)] pub struct Feed { pub versions: Vec<String>, pub allowed_lines: Option<Vec<String>> }`
  - `pub enum Lines { Latest(usize), Allowed(Vec<String>) }`
  - `pub fn is_stable(v: &str) -> bool`
  - `pub fn version_key(v: &str) -> Vec<u32>`
  - `pub fn line_major(v: &str) -> String` (`"24.18.0"` → `"24"`), `pub fn line_minor(v: &str) -> String` (`"11.8.2"` → `"11.8"`)
  - `pub fn select(versions: &[String], line_of: fn(&str) -> String, lines: &Lines, patches: usize) -> Vec<String>`
  - Trong `setup.rs`: `fn fetch_feed(&self, url: &str, dest: &Path) -> Result<(), SetupError>` trên trait `Downloader`, mặc định gọi `self.fetch`. `CurlDownloader` override bằng `curl -fsSL --max-time 20`.

- [ ] **Step 1: Viết test fail trong `version_feed.rs`**

```rust
fn s(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }

#[test]
fn select_keeps_three_patches_of_four_newest_lines() {
    let vs = s(&["1.31.6","1.31.5","1.31.4","1.31.3","1.30.5","1.29.8","1.28.3","1.27.5"]);
    assert_eq!(select(&vs, line_minor, &Lines::Latest(4), 3),
        s(&["1.31.6","1.31.5","1.31.4","1.30.5","1.29.8","1.28.3"]));
}

#[test]
fn select_allowed_lines_and_drops_prerelease_and_dupes() {
    let vs = s(&["13.1.0-rc","12.3.3","12.3.3","11.8.6","10.5.29","abc123"]);
    let lines = Lines::Allowed(s(&["12.3","11.8"]));
    assert_eq!(select(&vs, line_minor, &lines, 3), s(&["12.3.3","11.8.6"]));
}

#[test]
fn select_sorts_numerically_not_lexically() {
    assert_eq!(select(&s(&["9.0.0","10.0.0"]), line_major, &Lines::Latest(4), 3), s(&["10.0.0","9.0.0"]));
}

#[test]
fn is_stable_rejects_non_numeric_parts() {
    assert!(is_stable("2.10.3") && is_stable("8.4"));
    assert!(!is_stable("2.12.0-RC1") && !is_stable("v1.4.4") && !is_stable("") && !is_stable("1..2"));
}
```

- [ ] **Step 2: Chạy, xác nhận fail**

Run: `cd core && cargo test version_feed`
Expected: lỗi biên dịch, `select`/`Lines` chưa có.

- [ ] **Step 3: Cài đặt các hàm ở Interfaces + `fetch_feed`**

`select`: lọc `is_stable`, bỏ trùng, gom theo `line_of`, sắp nhánh giảm dần theo `version_key` của khoá nhánh, lấy nhánh theo `Lines`, mỗi nhánh `patches` bản lớn nhất, nối lại mới nhất trước.

- [ ] **Step 4: Chạy, xác nhận pass**

Run: `cd core && cargo test version_feed`
Expected: 4 test pass.

- [ ] **Step 5: Commit**

```bash
git add core/src/version_feed.rs core/src/lib.rs core/src/setup.rs
git commit -m "feat(versions): add version selection helpers and timed feed fetch"
```

---

### Task 2: Cache, `refresh`, `catalog_versions`, chuyển Composer và nối vào `tools`/command

**Files:**
- Modify: `core/src/version_feed.rs`
- Modify: `core/src/php_cli.rs` (bỏ `composer_versions_cache`, `refresh_composer_versions`, `composer_catalog_versions`; giữ `parse_composer_versions`, thêm `composer_feed`)
- Modify: `core/src/tools.rs` (`known_catalog`, `available_versions`, test)
- Modify: `src-tauri/src/commands.rs` (`tool_versions`)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub struct Source { pub key: &'static str, pub fallback: &'static [&'static str], pub line_of: fn(&str) -> String, pub fetch: fn(&dyn Downloader, &Path) -> Option<Feed> }`. `fetch` nhận thư mục tạm để ghi file tải về, trả `None` khi lỗi hoặc rỗng.
  - `pub fn source(tool: ManagedTool) -> Option<Source>`. Task này chỉ có `Composer`; các task sau thêm nhánh match cho tool của mình.
  - `pub fn fetch_text(dl: &dyn Downloader, url: &str, scratch: &Path) -> Option<String>` (gọi `fetch_feed` vào `scratch/<hash hoặc tên>.part`, đọc ra chuỗi, xoá file).
  - `pub fn refresh(paths: &LaraluxPaths, dl: &dyn Downloader, tool: ManagedTool)`
  - `pub fn cached_feed(paths: &LaraluxPaths, tool: ManagedTool) -> Option<Feed>`
  - `pub fn catalog_versions(paths: &LaraluxPaths, tool: ManagedTool) -> Vec<String>`: `Lines` = `Allowed(feed.allowed_lines)` nếu có, ngược lại `Latest(DEFAULT_LINES)`; đầu vào `select` = `feed.versions ∪ fallback`. Không có cache: `select(fallback, Latest(DEFAULT_LINES))`.
  - `php_cli::composer_feed(dl: &dyn Downloader, scratch: &Path) -> Option<Feed>`: `versions` = `parse_composer_versions`, `allowed_lines = Some(line_minor của từng version)`.
  - `tools::known_catalog(versions: Vec<String>, installed: Vec<String>, active: &str)` (đổi tham số đầu từ `&[&str]`).

- [ ] **Step 1: Viết test fail trong `version_feed.rs`**

Dùng `LaraluxPaths` tạm như `php_cli::tests::root()` và một `StubDownloader(&'static str)` ghi chuỗi cố định vào `dest`.

```rust
#[test]
fn refresh_writes_cache_and_skips_while_fresh() {
    // Stub trả VERSIONS_JSON của composer (stable 2.11.0, 2.2.31).
    refresh(&paths, &StubDownloader(COMPOSER_JSON), ManagedTool::Composer);
    assert!(cached_feed(&paths, ManagedTool::Composer).unwrap().versions.contains(&"2.11.0".into()));
    let dl = crate::setup::FakeDownloader::new();
    refresh(&paths, &dl, ManagedTool::Composer);
    assert!(dl.requested().lock().unwrap().is_empty());
}

#[test]
fn refresh_keeps_old_cache_on_bad_feed() { /* ghi cache tốt, đặt mtime cũ hơn 24h bằng File::set_modified, refresh với "garbage", cache cũ vẫn còn */ }

#[test]
fn corrupt_cache_falls_back_to_known() {
    // ghi "{\"versions\":[" vào tmp/feeds/composer.json
    assert_eq!(catalog_versions(&paths, ManagedTool::Composer),
        select(&known, line_minor, &Lines::Latest(DEFAULT_LINES), PATCHES_PER_LINE));
}

#[test]
fn catalog_uses_allowed_lines_from_feed() {
    // cache composer {versions:[2.11.0,2.2.31], allowed:[2.11,2.2]}: kết quả không chứa 2.9.7, 2.8.12
}
```

Trong `tools.rs`:

```rust
#[test]
fn catalog_keeps_installed_outside_feed() {
    // cài composer "2.6.6" (tạo version_dir), không cache
    let vs = available_versions(ManagedTool::Composer, &paths);
    assert!(vs.iter().find(|v| v.version == "2.6.6").unwrap().installed);
}
```

Sửa `composer_available_versions_includes_known_set_newest_first` để so với `catalog_versions` thay vì `KNOWN_COMPOSER_VERSIONS.len()`.

- [ ] **Step 2: Chạy, xác nhận fail**

Run: `cd core && cargo test version_feed tools::`
Expected: lỗi biên dịch, `refresh`/`Source` chưa có.

- [ ] **Step 3: Cài đặt `version_feed` (Interfaces trên) và chuyển Composer**

Mỗi lần tải đầy đủ thì mtime của file cache là thời điểm làm mới. `refresh` bỏ qua khi `tool` không có `Source`.

- [ ] **Step 4: `tools.rs` dùng `catalog_versions` cho Composer; `tool_versions` gọi `refresh` chung**

`commands.rs`: thay nhánh `if t == ManagedTool::Composer { refresh_composer_versions(..) }` bằng `laralux_core::version_feed::refresh(&state.paths, &CurlDownloader, t);`.

- [ ] **Step 5: Chạy toàn bộ test và check workspace**

Run: `cd core && cargo test && cd .. && cargo check --workspace`
Expected: pass, trừ `coredns::tests::pick_coredns_port_skips_a_busy_default` khi CoreDNS của laralux đang chạy trên máy (fail sẵn từ trước, không liên quan).

- [ ] **Step 6: Commit**

```bash
git add core/src/version_feed.rs core/src/php_cli.rs core/src/tools.rs src-tauri/src/commands.rs
git commit -m "feat(versions): shared feed cache and catalog, migrate Composer onto it"
```

---

### Task 3: Node

**Files:** Modify `core/src/node_static.rs`, `core/src/version_feed.rs` (thêm `Source`), `core/src/tools.rs` (Node dùng `catalog_versions`, sửa test Node).

**Interfaces:**
- Produces: `pub fn parse_node_feed(json: &str) -> Feed` và `pub fn node_feed(dl, scratch) -> Option<Feed>` (URL `https://nodejs.org/dist/index.json`). `Source { key: "node", fallback: &KNOWN_NODE_VERSIONS, line_of: line_major, .. }`.

`versions` bỏ tiền tố `v`. `allowed_lines` = các major có `lts` khác `false` + major lớn nhất trong toàn feed (Current).

- [ ] **Step 1: Test fail**

```rust
#[test]
fn node_feed_allows_lts_and_newest_current() {
    let json = r#"[{"version":"v26.12.1","lts":false},{"version":"v25.9.0","lts":false},
                   {"version":"v24.18.0","lts":"Krypton"},{"version":"v22.23.1","lts":"Jod"}]"#;
    let f = parse_node_feed(json);
    assert_eq!(f.allowed_lines, Some(vec!["26".into(), "24".into(), "22".into()]));
    assert!(f.versions.contains(&"26.12.1".to_string()));
}
#[test]
fn node_feed_bad_json_is_empty() { assert!(parse_node_feed("<html>").versions.is_empty()); }
```

- [ ] **Step 2:** `cd core && cargo test node_feed`, Expected: lỗi biên dịch.
- [ ] **Step 3:** Cài đặt `parse_node_feed`, `node_feed`, `Source` cho Node, `tools.rs` dùng `catalog_versions`.
- [ ] **Step 4:** `cd core && cargo test`, Expected: pass (trừ test coredns như Task 2).
- [ ] **Step 5:** Commit `feat(versions): Node catalog from nodejs.org index`.

---

### Task 4: Nginx và PostgreSQL (feed lọc theo arch)

**Files:** Modify `core/src/nginx_static.rs`, `core/src/postgres_static.rs`, `core/src/version_feed.rs`, `core/src/tools.rs`.

**Interfaces:**
- `nginx_static::parse_nginx_feed(json: &str, arch: &str) -> Feed`: lấy `version` của mục `name == "nginx"`, `os == "linux"`, đúng `arch`, chấp nhận cả object `{contents: [...]}` lẫn mảng (giống `latest_nginx`). `allowed_lines = None`. URL `NGINX_INDEX_URL`, arch `nginx_arch()`. `Source { key: "nginx", line_of: line_minor }`.
- `postgres_static::parse_postgres_feed(xml: &str) -> Feed`: mọi `<version>…</version>` trong `maven-metadata.xml`, tách bằng tìm chuỗi (không thêm crate XML). URL `https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-{postgres_arch()}/maven-metadata.xml`. `Source { key: "postgres", line_of: line_major }`.
- Không xác định được arch: `fetch` trả `None`.

- [ ] **Step 1: Test fail**

```rust
#[test]
fn nginx_feed_filters_arch_and_os() {
    let json = r#"{"contents":[
      {"name":"nginx","os":"linux","arch":"x86_64","version":"1.31.6"},
      {"name":"nginx","os":"linux","arch":"aarch64","version":"1.31.7"},
      {"name":"nginx","os":"darwin","arch":"x86_64","version":"1.31.8"},
      {"name":"njs","os":"linux","arch":"x86_64","version":"0.9.1"}]}"#;
    assert_eq!(parse_nginx_feed(json, "x86_64").versions, vec!["1.31.6".to_string()]);
}
#[test]
fn postgres_feed_reads_maven_versions() {
    let xml = "<metadata><versioning><versions><version>17.11.0</version><version>18.6.0</version></versions></versioning></metadata>";
    assert_eq!(parse_postgres_feed(xml).versions, vec!["17.11.0".to_string(), "18.6.0".to_string()]);
}
```

- [ ] **Step 2:** `cd core && cargo test _feed`, Expected: lỗi biên dịch.
- [ ] **Step 3:** Cài đặt hai parser, hai `Source`, `tools.rs` dùng `catalog_versions`; sửa test catalog nginx/postgres trong `tools.rs` để không còn so `len()` với `KNOWN_*` (so với `catalog_versions` và kiểm tra cờ installed).
- [ ] **Step 4:** `cd core && cargo test`, Expected: pass.
- [ ] **Step 5:** Commit `feat(versions): Nginx and PostgreSQL catalogs from their binary indexes`.

---

### Task 5: MariaDB và MongoDB (nguồn có thông tin nhánh hỗ trợ)

**Files:** Modify `core/src/mariadb_static.rs`, `core/src/mongodb_static.rs`, `core/src/version_feed.rs`, `core/src/tools.rs`.

**Interfaces:**
- `mariadb_static::parse_mariadb_lines(json: &str) -> Vec<String>`: `release_id` của `major_releases` có `release_status == "Stable"`.
- `mariadb_static::parse_mariadb_releases(json: &str) -> Vec<String>`: các khoá trong object `releases`.
- `mariadb_static::mariadb_feed(dl, scratch) -> Option<Feed>`: gọi `https://downloads.mariadb.org/rest-api/mariadb/`, rồi `.../rest-api/mariadb/<line>/` cho từng nhánh Stable. Nhánh nào lỗi thì bỏ nhánh đó. `allowed_lines = Some(lines)`. `Source { key: "mariadb", line_of: line_minor }`.
- `mongodb_static::parse_mongodb_feed(json: &str) -> Feed`: `versions[].version` của `current.json`; `allowed_lines = Some(line_minor của từng bản)`. URL `https://downloads.mongodb.org/current.json`. `Source { key: "mongodb", line_of: line_minor }`.

- [ ] **Step 1: Test fail**

```rust
#[test]
fn mariadb_lines_keep_only_stable() {
    let j = r#"{"major_releases":[{"release_id":"13.2","release_status":"Preview"},
        {"release_id":"13.1","release_status":"RC"},{"release_id":"12.3","release_status":"Stable"}]}"#;
    assert_eq!(parse_mariadb_lines(j), vec!["12.3".to_string()]);
}
#[test]
fn mariadb_feed_merges_lines_and_skips_failed_line() {
    // stub downloader: URL kết thúc "/mariadb/" trả danh sách 12.3 + 11.8 Stable,
    // ".../12.3/" trả {"releases":{"12.3.3":{},"12.3.2":{}}}, ".../11.8/" trả lỗi
    // kỳ vọng: versions == ["12.3.3","12.3.2"], allowed_lines == Some(["12.3","11.8"])
}
#[test]
fn mongodb_feed_reads_current_json() {
    let j = r#"{"versions":[{"version":"9.0.4"},{"version":"8.0.34"}]}"#;
    let f = parse_mongodb_feed(j);
    assert_eq!(f.versions, vec!["9.0.4".to_string(), "8.0.34".to_string()]);
    assert_eq!(f.allowed_lines, Some(vec!["9.0".to_string(), "8.0".to_string()]));
}
```

- [ ] **Step 2:** `cd core && cargo test _feed _lines`, Expected: lỗi biên dịch.
- [ ] **Step 3:** Cài đặt, thêm hai `Source`, `tools.rs` dùng `catalog_versions`, sửa test catalog mariadb/mongodb.
- [ ] **Step 4:** `cd core && cargo test`, Expected: pass.
- [ ] **Step 5:** Commit `feat(versions): MariaDB and MongoDB catalogs limited to supported lines`.

---

### Task 6: Valkey, Mailpit, mkcert (GitHub releases)

**Files:** Modify `core/src/version_feed.rs` (parser GitHub dùng chung), `core/src/redis_static.rs`, `core/src/mailpit_static.rs`, `core/src/mkcert_static.rs`, `core/src/tools.rs`.

**Interfaces:**
- `version_feed::parse_github_releases(json: &str) -> Feed`: bỏ mục `draft` hoặc `prerelease` true, bỏ tiền tố `v` ở `tag_name`, `allowed_lines = None`. JSON không phải mảng (ví dụ body rate limit) thì rỗng.
- `version_feed::github_feed(dl, scratch, repo: &str) -> Option<Feed>`: URL `https://api.github.com/repos/{repo}/releases?per_page=100`.
- `Source`: `"redis"` ← `valkey-io/valkey`, `"mailpit"` ← `axllent/mailpit`, `"mkcert"` ← `FiloSottile/mkcert`, đều `line_minor`. Vì `fetch` là `fn` pointer, mỗi tool có một hàm bọc `fn valkey_feed(dl, scratch) -> Option<Feed> { github_feed(dl, scratch, "valkey-io/valkey") }` đặt trong module của tool.

- [ ] **Step 1: Test fail**

```rust
#[test]
fn github_releases_skip_prerelease_and_strip_v() {
    let j = r#"[{"tag_name":"v1.31.6","prerelease":false,"draft":false},
                {"tag_name":"9.2.0-rc1","prerelease":true,"draft":false},
                {"tag_name":"v1.31.7","prerelease":false,"draft":true}]"#;
    assert_eq!(parse_github_releases(j).versions, vec!["1.31.6".to_string()]);
}
#[test]
fn github_rate_limit_body_parses_empty() {
    assert!(parse_github_releases(r#"{"message":"API rate limit exceeded for 1.2.3.4."}"#).versions.is_empty());
}
```

- [ ] **Step 2:** `cd core && cargo test github_`, Expected: lỗi biên dịch.
- [ ] **Step 3:** Cài đặt, thêm ba `Source`, `tools.rs` dùng `catalog_versions`, sửa test catalog redis/mailpit (mkcert hiện chưa có test catalog, không cần thêm).
- [ ] **Step 4:** `cd core && cargo test`, Expected: pass.
- [ ] **Step 5:** Commit `feat(versions): Valkey, Mailpit and mkcert catalogs from GitHub releases`.

---

### Task 7: PHP (catalog theo minor từ index static-php)

**Files:** Modify `core/src/php_static.rs`, `core/src/php_versions.rs`, `core/src/version_feed.rs`, `core/src/tools.rs`.

**Interfaces:**
- `php_static::parse_php_minors(json: &str, arch_tag: &str) -> Feed`: từ các `name` dạng `php-<maj>.<min>.<patch>-cli-linux-<arch_tag>.tar.gz` (khớp đúng prefix/suffix mà `latest_patch` đang dùng), trả `versions` là các minor duy nhất (`"8.4"`), `allowed_lines = None`. URL `{STATIC_PHP_BASE}/?format=json`, arch `arch_tag()`.
- `Source { key: "php", fallback: &KNOWN_PHP_VERSIONS, line_of: line_minor, .. }`. Với `select(.., Latest(4), 3)`, mỗi nhánh chỉ có một phần tử nên kết quả là 4 minor mới nhất.
- `php_versions::php_versions_from(known: &[String], installed: &[String], active: &str)` nhận thêm `known`. `php_versions(paths, active)` truyền `version_feed::catalog_versions(paths, ManagedTool::Php)`.

- [ ] **Step 1: Test fail**

```rust
#[test]
fn php_minors_from_index_for_arch() {
    let j = r#"[{"name":"php-8.5.1-cli-linux-x86_64.tar.gz"},{"name":"php-8.4.22-cli-linux-x86_64.tar.gz"},
                {"name":"php-8.4.21-cli-linux-x86_64.tar.gz"},{"name":"php-8.3.30-cli-linux-aarch64.tar.gz"},
                {"name":"php-8.4.22-fpm-linux-x86_64.tar.gz"}]"#;
    assert_eq!(parse_php_minors(j, "x86_64").versions, vec!["8.5".to_string(), "8.4".to_string()]);
}
```

Sửa test `php_versions_marks_installed_and_active` và test PHP trong `tools.rs` (đang khẳng định 6 phần tử) theo danh sách truyền vào.

- [ ] **Step 2:** `cd core && cargo test php_minors php_versions`, Expected: lỗi biên dịch.
- [ ] **Step 3:** Cài đặt; xác minh định dạng tên file bằng cách đọc `latest_patch` trong `php_static.rs` trước khi viết parser, fixture phải theo đúng định dạng đó.
- [ ] **Step 4:** `cd core && cargo test && cd .. && cargo check --workspace`, Expected: pass.
- [ ] **Step 5:** Commit `feat(versions): PHP minor catalog from the static-php index`.

---

### Task 8: Kiểm tra thực tế với feed thật

**Files:** không sửa code, trừ khi phát hiện lỗi parse.

- [ ] **Step 1:** Thêm một test `#[ignore]` trong `version_feed.rs`, gọi `refresh` + `catalog_versions` cho cả 10 tool bằng `CurlDownloader` vào một root tạm, in kết quả.

Run: `cd core && cargo test live_feeds -- --ignored --nocapture`
Expected: mỗi tool in ra danh sách không rỗng, đúng quy tắc (Node có 26/24/22, MariaDB chỉ nhánh Stable, PostgreSQL có 18.x, không có RC/beta).

- [ ] **Step 2:** Với mỗi tool, chọn bản đầu danh sách và kiểm tra URL tải bằng `curl -sIL -o /dev/null -w '%{http_code}'` (dựng URL bằng hàm `*_url` tương ứng). Expected: 200.
- [ ] **Step 3:** Commit `test(versions): ignored live check across all feeds`.
