# Procfile cho proxy site — Thiết kế

**Ngày:** 2026-07-15
**Trạng thái:** Đã duyệt (chờ review spec)

## Vấn đề

Laralux có sẵn chức năng chạy process theo từng site kiểu Foreman: đọc file
`Procfile` ở gốc project, start/stop từng process trong GUI (menu ⋯ →
**Processes**), ghi log ra `proc-<site>-<name>.log`, và autostart theo config.

Chức năng này **không dùng được cho proxy site**. Nguyên nhân duy nhất: proxy
site không có thư mục.

- `ProxySite` trong registry chỉ có `name`, `websocket`, `routes` — không có
  đường dẫn nào.
- `list_all_sites` tạo proxy site với `root: PathBuf::new()` (rỗng).
- `site_proc_counts` đếm process bằng `read_procfile(&s.root)`; root rỗng nên
  luôn ra 0 → mục **Processes** không bao giờ hiện cho proxy site.
- `SiteProcs::spawn_spec` dùng `.cwd(root)` làm thư mục làm việc của process —
  root rỗng thì cũng không có chỗ để chạy.

Hệ quả thực tế: một proxy site trỏ tới dev-server (ví dụ Next/Vite ở
`127.0.0.1:3000`) vẫn phải tự mở terminal chạy `npm run dev` bằng tay, dù
laralux đã lo phần route + HTTPS.

## Hướng giải quyết

Cho proxy site **một field folder tuỳ chọn**. Có folder là toàn bộ máy móc
process sẵn có chạy được ngay, không phải viết lại gì.

Folder này **chỉ phục vụ process và tiện ích UI** — nó KHÔNG trở thành document
root. Proxy site vẫn chỉ proxy; cấu hình nginx không đổi.

## Các thành phần

### 1. Data model — `core/src/site_registry.rs`

`ProxySite` thêm field:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub root: Option<PathBuf>,
```

- `#[serde(default)]` → `sites.toml` cũ (proxy chưa có `root`) vẫn load bình thường.
- `skip_serializing_if` → proxy không có folder không ghi thừa key vào file.

Đổi chữ ký:

- `add_proxy(&mut self, name, routes, websocket, root: Option<&Path>)`
- `update_proxy(&mut self, name, routes, websocket, root: Option<&Path>)`

Nếu `root` là `Some`, phải là thư mục đang tồn tại — trả `RegistryError::RootNotFound`
nếu không, và canonicalize đường dẫn (tái dùng đúng cách `SiteRegistry::add` đang làm).

### 2. Site model — `core/src/sites.rs`

Trong `list_all_sites`, nhánh proxy đang set cứng `root: PathBuf::new()`; đổi thành:

```rust
root: p.root.clone().unwrap_or_default(),
```

**Xử lý folder bị mất (quyết định quan trọng):** với *linked site*, root không tồn
tại thì site bị **skip** kèm warning. Với *proxy site* thì KHÔNG được skip — folder
chỉ phục vụ process, skip site sẽ làm sập luôn routing + HTTPS của domain đó.

Quy tắc: nếu proxy có `root` nhưng thư mục không tồn tại → **giữ nguyên site**, đặt
`root` về rỗng, và thêm một **warning**. Routing không bao giờ phụ thuộc vào folder.

### 3. Process — không cần sửa logic

`procfile.rs`, `site_procs.rs`, và các command `site_procs` / `start_site_procs` /
`stop_site_procs` / `site_proc_counts` đều đã key theo `root`. Khi proxy site có
root, mọi thứ chạy ngay: đọc `Procfile`, CWD đúng thư mục, log
`proc-<site>-<name>.log`, start/stop, autostart.

**Gia cố một bug tiềm ẩn có sẵn:** `read_procfile(Path::new(""))` sẽ đọc `Procfile`
theo đường dẫn tương đối từ CWD của tiến trình laralux. Hiện chưa lộ ra vì menu
Processes bị ẩn, nhưng khi proxy có thể có root thì đường này dễ chạm tới. Thêm
guard: **root rỗng → coi như không có process** (trong `site_proc_counts` và
`site_procs_view`).

### 4. UI — `src/`

- **Modal Reverse proxy** (`src/ui/modals/proxy.ts`): thêm field
  *Project folder (optional)* kèm nút Browse, tái dùng đúng pattern `openDialog`
  mà modal Link Site đang dùng.
- **Sites view** (`src/ui/views/sites.ts`): hiện tại `isProxy` đang gate ẩn
  `folderBtn`, `termBtn`, `subRight` (đường dẫn root). Đổi điều kiện từ `isProxy`
  sang `!s.root`:
  - proxy **có** folder → hiện đủ **Open folder**, **Open terminal**, đường dẫn root
    (nhất quán với site thường);
  - proxy **không** folder → giữ nguyên hành vi hiện tại.
- Mục **Processes** không cần sửa: nó đã gate theo `state.procCounts[s.name]`, nên
  tự xuất hiện khi proxy có Procfile hợp lệ.

### 5. Xoá site — `src/ui/modals/deletesite.ts`

**Bất biến bắt buộc giữ:** proxy site **không bao giờ** có nút xoá-khỏi-đĩa. Luồng
xoá phân nhánh theo `source`: chỉ *Scanned* mới có **Delete** (gọi
`delete_scanned_site` → `remove_dir_all`); *Linked* và *Proxy* chỉ có **Remove**
(gỡ entry khỏi registry, giữ nguyên thư mục). Thêm folder cho proxy KHÔNG được
đổi điều này.

Lớp phòng vệ sẵn có cần giữ nguyên: `delete_scanned_site` luôn ghép
`paths.www().join(name)` và chặn tên chứa `/`, `\`, `.`, `..` qua
`valid_scanned_name`, nên nó chỉ có thể xoá bên trong `~/laralux/www/` — không
bao giờ chạm tới folder project bên ngoài của proxy.

**Cần sửa:** modal hiện ẩn đường dẫn root khi `source === "Proxy"`. Khi proxy có
folder, người dùng bấm Remove không thấy dòng nào trấn an rằng folder được giữ —
trong khi nhánh *Linked* có. Đổi thành:

- proxy **có** folder → hiện đường dẫn + câu kiểu *"Your project folder `<path>`
  is kept."* (thống nhất với nhánh Linked);
- proxy **không** folder → giữ nguyên nội dung hiện tại.

### 6. Command layer — `src-tauri/src/commands.rs`

Command `add_proxy` và `update_proxy` nhận thêm tham số `root: Option<String>`,
truyền xuống registry. Phần còn lại giữ nguyên (sync + reload nginx như hiện tại).

## Luồng dữ liệu

1. Người dùng tạo/sửa proxy site, chọn *Project folder* trỏ tới project Node.
2. `add_proxy`/`update_proxy` validate thư mục, lưu `root` vào `sites.toml`.
3. `list_all_sites` trả proxy site với `root` đã điền.
4. `site_proc_counts` đọc `Procfile` trong folder đó → menu **Processes** hiện ra.
5. Người dùng start `dev: npm run dev`; process chạy với CWD = folder, log vào
   `~/laralux/log/proc-<site>-dev.log`.
6. nginx vẫn route `https://<site>.dev` → upstream `127.0.0.1:3000` như cũ.

## Xử lý lỗi

- Chọn folder không tồn tại lúc lưu → `RootNotFound`, hiện lỗi trong modal.
- Folder bị xoá sau khi đã lưu → proxy site vẫn hoạt động (routing nguyên vẹn),
  kèm warning, không có process.
- `Procfile` không tồn tại hoặc rỗng → không có mục Processes (đúng như site thường).
- Dòng `Procfile` sai định dạng → bỏ qua dòng đó (hành vi `parse_procfile` sẵn có).

## Testing

- `site_registry.rs`: proxy có `root` roundtrip qua save/load; `sites.toml` cũ
  không có `root` vẫn load; `root` trỏ thư mục không tồn tại → `RootNotFound`;
  `update_proxy` đổi được root.
- `sites.rs`: `list_all_sites` điền `root` cho proxy; proxy có root nhưng thư mục
  đã mất → **site vẫn còn trong danh sách** + có warning (không bị skip);
  proxy không có root → `root` rỗng như cũ.
- `procfile`/proc counts: proxy có folder chứa `Procfile` thì đếm ra đúng số
  process; root rỗng → 0 (guard).
- Xoá site: gỡ một proxy có folder chỉ xoá entry trong registry — thư mục trên
  đĩa **vẫn còn**. `delete_scanned_site` từ chối tên không hợp lệ và không thoát
  ra ngoài `www/` (các test `valid_scanned_name` / `delete_scanned_site` sẵn có
  đã phủ, giữ nguyên).

## Ngoài phạm vi (YAGNI)

- Folder KHÔNG trở thành document root; nginx vhost của proxy không đổi.
- Không tự động start upstream khi mở URL của site.
- Không cho override CWD riêng cho từng process (CWD dùng chung = folder).
- Không tự phát hiện/đề xuất folder từ upstream port.
