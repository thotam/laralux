# Danh sách version lấy từ nguồn - Thiết kế

**Ngày:** 2026-10-22
**Trạng thái:** Đã duyệt thiết kế (chờ review spec)

## Vấn đề

Modal Setup của mỗi tool hiện danh sách version hard-code (`KNOWN_*_VERSIONS`).
Danh sách cũ dần giữa các lần release: Node dừng ở `24.18.0` khi nodejs.org đã
có `26.12.1`, MariaDB thiếu nhánh `12.3`/`13.0`, PostgreSQL thiếu `18.x`.
Bản 0.8.5 đã sửa riêng cho Composer (feed `getcomposer.org/versions`, cache
24h). Spec này đưa cách đó thành cơ chế chung cho cả 10 tool.

## Mục tiêu

- Catalog của mọi tool lấy từ nguồn mà laralux thực sự tải binary về.
- Mỗi nhánh còn hỗ trợ hiện **3 bản patch mới nhất**.
- Offline hoặc nguồn lỗi vẫn hiện được danh sách (danh sách cứng làm dự phòng).
- Bản đã cài luôn hiện, kể cả khi đã rơi khỏi feed.

Ngoài phạm vi: tự động cập nhật/nâng version, thông báo "có bản mới", đổi PHP
sang catalog theo patch.

## Quy tắc chọn version

`select(versions, rule) -> Vec<String>` là hàm thuần:

1. Bỏ pre-release (`rc`, `beta`, `alpha`, `preview`, `-dev`, chuỗi không phải số).
2. Gom theo nhánh. Khoá nhánh do từng tool quy định (bảng dưới).
3. Chọn nhánh theo `rule.lines`:
   - `Latest(n)`: n nhánh mới nhất.
   - `Allowed(set)`: các nhánh mà nguồn đánh dấu còn hỗ trợ.
4. Mỗi nhánh giữ `rule.patches` (= 3) bản mới nhất.
5. Kết quả sắp xếp mới nhất trước.

Catalog cuối cùng = `select(feed ∪ danh sách cứng)` ∪ bản đã cài, gắn cờ
`installed`/`active` như `known_catalog` hiện tại.

## Nguồn theo từng tool

| Tool | Feed | Khoá nhánh | Nhánh hiện |
|---|---|---|---|
| Node | `nodejs.org/dist/index.json` + `raw.githubusercontent.com/nodejs/Release/main/schedule.json` | major | các major có `start ≤ hôm nay < end` theo schedule |
| PHP | `dl.static-php.dev/static-php-cli/bulk/?format=json` | minor | 4 minor mới nhất, catalog **theo minor** (xem dưới) |
| Nginx | `jirutka.github.io/nginx-binaries/index.json` (lọc `name=nginx`, `os=linux`, đúng arch) | minor | 4 mới nhất |
| MariaDB | `downloads.mariadb.org/rest-api/mariadb/` rồi `.../rest-api/mariadb/<nhánh>/` | minor | `release_status = Stable` và chưa quá `release_eol_date` (API vẫn để nhánh EOL là Stable) |
| PostgreSQL | `repo1.maven.org/.../embedded-postgres-binaries-linux-<arch>/maven-metadata.xml` | major | 4 mới nhất |
| MongoDB | `downloads.mongodb.org/current.json` | minor | nhánh trong `current.json` có build `ubuntu2204` đúng arch |
| Valkey | `api.github.com/repos/valkey-io/valkey/releases?per_page=100` | minor | 4 mới nhất |
| Mailpit | `api.github.com/repos/axllent/mailpit/releases?per_page=100` | minor | 4 mới nhất |
| mkcert | `api.github.com/repos/FiloSottile/mkcert/releases?per_page=100` | minor | 4 mới nhất |
| Composer | `getcomposer.org/versions` (mục `stable`) | minor | mọi nhánh trong `stable` (mới nhất + LTS) |

Đã kiểm tra ngày 2026-10-22: bản mới nhất ở từng nguồn đều có binary đúng định
dạng laralux tải (HTTP 200), gồm MongoDB `9.0.4` bản `ubuntu2204`, MariaDB
`13.0.2` bintar, Valkey `9.1.2` jammy, PostgreSQL `18.6.0`, Mailpit `1.31.6`.

Ghi chú theo nguồn:

- **Mailpit, mkcert** chỉ giữ release có file build đúng arch trong `assets` (mkcert 1.1–1.3 không có bản arm64). Valkey không lọc được vì binary nằm ở `download.valkey.io`.
- **GitHub API** không xác thực giới hạn 60 request/giờ/IP. Cache 24h và chỉ
  tải khi mở modal nên dư sức. Bỏ release có `prerelease: true` hoặc `draft`,
  bỏ tiền tố `v` ở `tag_name`.
- **MariaDB** cần 1 request danh sách nhánh + 1 request mỗi nhánh Stable. Cache
  gộp thành một file danh sách version.
- **MongoDB** dùng `current.json` (khoảng 465KB), không dùng `full.json` (51MB). File này còn liệt kê 5.0 và 4.4, vốn không có build `ubuntu2204` (403), nên lọc theo `downloads[].target`/`arch`.
- **Node**: `index.json` chỉ cho biết một nhánh từng là LTS, không cho biết còn hỗ trợ không (lấy theo LTS sẽ ra tới Node 4). Lịch `schedule.json` cho `start`/`end` của từng nhánh. Không tải được lịch thì coi như feed lỗi, giữ cache cũ.
- **PHP**: catalog vẫn theo minor (`8.4`), vì `php_version` trong config và PHP
  theo từng site gắn với minor, còn lúc cài đã tự chọn patch mới nhất trong
  index. Feed chỉ thay mảng `KNOWN_PHP_VERSIONS` bằng danh sách minor có trong
  index. Đây là ngoại lệ duy nhất của quy tắc "3 patch mỗi nhánh".

## Kiến trúc

Module mới `core/src/version_feed.rs`:

```rust
pub enum Lines { Latest(usize), Allowed(Vec<String>) }
pub struct FeedSpec {
    pub key: &'static str,                    // tên file cache: tmp/feeds/<key>.json
    pub fallback: &'static [&'static str],    // danh sách cứng
    pub line_of: fn(&str) -> String,          // khoá nhánh
}
pub fn refresh(paths, downloader, tool)       // tải nếu cache thiếu hoặc > 24h
pub fn catalog_versions(paths, tool) -> Vec<String>
pub fn select(versions, lines, patches) -> Vec<String>
```

- Mỗi module `*_static.rs` cung cấp URL feed và hàm `parse_feed(text) -> Feed`
  (`Feed { versions, allowed_lines: Option<Vec<String>> }`). Hàm parse là
  thuần, test bằng mẫu cắt từ feed thật.
- Quy tắc nhánh suy từ feed: `allowed_lines = Some(..)` thì `Allowed`, `None`
  thì `Latest(4)`. Node, MariaDB, MongoDB, Composer trả `Some` (Node lấy từ
  `schedule.json`). Chưa có cache thì
  áp `Latest(4)` lên danh sách cứng.
- Cache lưu **kết quả đã parse** (JSON `{versions, allowed_lines}`), không lưu
  feed thô: MongoDB và nginx index lớn, MariaDB gồm nhiều request.
- Ghi cache nguyên tử: ghi `*.part` rồi rename. Tải lỗi, parse lỗi hoặc ra rỗng
  thì giữ cache cũ.
- `tools::available_versions` gọi `catalog_versions` thay cho `KNOWN_*`.
  `known_catalog` giữ nguyên vai trò gắn cờ installed/active.
- Composer chuyển từ code riêng trong `php_cli.rs` sang module chung.
- Command `tool_versions` (đã async + `spawn_blocking`) gọi `refresh` cho đúng
  tool đang mở, rồi trả catalog. Không tải gì lúc khởi động app.

## Xử lý lỗi

- Không có mạng, timeout, HTTP lỗi, JSON hỏng: catalog dùng cache cũ, không có
  cache thì dùng danh sách cứng. Không báo lỗi lên UI.
- curl cần timeout cho feed (`--max-time 20`) để modal không treo lâu khi mạng
  chập chờn. Thêm phương thức `fetch_text`/timeout vào `Downloader` nếu
  `fetch` hiện tại không có.
- Version trong feed có nhưng thiếu binary: cài sẽ báo lỗi 404 như hiện tại.
- Tải feed lỗi: ghi `<key>.failed`, 1 giờ sau mới thử lại, để mạng chết không làm mỗi lần mở modal chờ 20–40 giây.
- Modal hiện "Loading versions…" trong lúc chờ `tool_versions`.

## Kiểm thử

- `select`: gom nhánh, `Latest(n)`, `Allowed`, 3 patch, lọc pre-release, sắp xếp.
- Mỗi `parse_feed`: một fixture nhỏ cắt từ feed thật, gồm cả mục pre-release và
  mục sai arch (nginx, postgres).
- `refresh`: cache còn hạn thì không tải; feed hỏng thì không ghi cache; tải
  lỗi thì giữ cache cũ.
- `catalog_versions`: không có cache thì ra danh sách cứng; bản đã cài nằm ngoài
  feed vẫn có mặt.
- Test hiện có của từng tool (`available_versions_includes_known_set...`) sửa
  theo hành vi mới.
