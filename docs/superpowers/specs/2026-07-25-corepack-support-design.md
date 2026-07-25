# Corepack (yarn/pnpm) support — Thiết kế

**Ngày:** 2026-07-25
**Trạng thái:** Đã duyệt (chờ review spec)

## Vấn đề

laralux tải Node.js static build vào `bin/node/<version>/` và expose `node`,
`npm`, `npx` cho terminal + Procfile bằng cách tạo **relative symlink ở version
root** (`root/node -> bin/node`, v.v. — `make_root_links`), rồi symlink tiếp ra
`/usr/local/bin` (`tools.rs` `cli_binaries`).

Node build (đến Node 24) **ship sẵn `bin/corepack`** — cổng chuẩn để dùng
`yarn`/`pnpm`. Người dùng đã tự chạy `corepack enable`, tạo shim
`yarn/yarnpkg/pnpm/pnpx` trong `bin/node/current/bin/`. Nhưng:

- `make_root_links` chỉ expose `node/npm/npx` — không có corepack/yarn/pnpm ở
  version root.
- `managed_bin_dirs` (PATH của terminal + Procfile) chỉ thêm `bin/node/current`
  (version root), **không** thêm `bin/node/current/bin/`.

Kết quả: shim yarn/pnpm nằm off-PATH → terminal và Procfile của laralux không
thấy `yarn`/`pnpm`, dù chúng đã tồn tại.

## Bối cảnh đã tra cứu (nguồn chính thức)

- Corepack **bundled với Node từ 14.19.0 đến < 25.0.0**. Node 24 (mặc định của
  laralux) có; **Node 25+ sẽ không ship** corepack nữa (trang
  `nodejs.org/api/corepack.html` nay redirect sang repo GitHub riêng).
- `corepack enable` (mặc định) tạo shim cho **yarn và pnpm** ngay **cạnh binary
  corepack** (tức trong `bin/`). Chỉ tạo shim, **offline**, không tải PM.
- Không còn gắn nhãn experimental.

Hệ quả thiết kế: mọi thao tác phải **best-effort, skip-if-absent** để Node 25+
(không có corepack) vẫn hoạt động sạch.

## Hướng giải quyết

Tái dùng nguyên cơ chế `make_root_links` sẵn có (relative symlink vào `bin/`) —
chỉ mở rộng danh sách CLI và thêm một bước chạy `corepack enable`. Không dùng
`--install-directory`: để `corepack enable` tạo shim mặc định trong `bin/`, rồi
`make_root_links` symlink chúng lên version root (đúng như node/npm/npx).

`bin/corepack`, `bin/yarn`, `bin/pnpm` đều là symlink tới `.js` với shebang
`#!/usr/bin/env node`; khi chạy trong terminal laralux, PATH có version root nơi
`node` symlink nằm, nên shebang phân giải được — không phụ thuộc PATH hệ thống.

## Các thành phần

### 1. `core/src/node_static.rs`

**Helper mới `expose_node_clis(dir, runner)`** (gọi ở CẢ hai nhánh của
`install_node_version`: fresh-extract và already-installed early-return — để
retrofit các bản node đã cài):

1. **Enable corepack** — nếu `dir/bin/corepack` tồn tại **và** `dir/bin/yarn`
   chưa có (guard tránh spawn thừa mỗi lần gọi):
   ```
   runner.run(dir/bin/node, [dir/bin/corepack, "enable"], None)
   ```
   Chạy qua binary `node` tuyệt đối nên không cần PATH. Lỗi enable là best-effort
   (log/bỏ qua, không làm hỏng install) — ví dụ Node 25+ không có corepack.

2. **`make_root_links`** mở rộng danh sách:
   `["node", "npm", "npx", "corepack", "yarn", "yarnpkg", "pnpm", "pnpx"]`.
   Vẫn skip-if-absent, vẫn relative symlink `root/<name> -> bin/<name>`.

Thứ tự bắt buộc: enable **trước** make_root_links (để `bin/yarn` tồn tại khi
make_root_links symlink nó).

Nhánh already-installed (hiện chỉ `set_current` rồi return) phải gọi
`expose_node_clis` trước khi return, để node đã cài được nâng cấp có
corepack/yarn/pnpm.

### 2. `core/src/tools.rs`

`ToolInfo` của Node: `cli_binaries` từ `&["node", "npm", "npx"]` →
`&["node", "npm", "npx", "corepack", "yarn", "yarnpkg", "pnpm", "pnpx"]`.

`symlinks.rs::link_tool` đã skip CLI vắng mặt (test
`link_tool_skips_absent_secondary_clis`), nên `/usr/local/bin` chỉ nhận cái nào
thực sự có — Node 25+ tự động chỉ có node/npm/npx.

## Luồng dữ liệu

1. Setup/cài node → `install_node_version` extract tarball → `expose_node_clis`:
   `corepack enable` tạo shim trong `bin/` → `make_root_links` symlink
   node/npm/npx/corepack/yarn/pnpm/… lên version root.
2. Version root nằm trong `managed_bin_dirs` → terminal + Procfile thấy ngay
   `yarn`/`pnpm`/`corepack`.
3. Bật "symlink to PATH" → `/usr/local/bin` nhận các CLI có mặt.
4. Đổi version node (`set_current` repoint `current`) → version mới đã có shim
   riêng từ lúc cài, `current` trỏ sang là dùng được.

## Xử lý lỗi

- `bin/corepack` vắng (Node 25+) → không enable; make_root_links + cli_binaries
  skip corepack/yarn/pnpm. Không lỗi.
- `corepack enable` thất bại → best-effort: bỏ qua, install node vẫn thành công
  (node/npm/npx vẫn expose bình thường).
- Node cũ đã cài trước khi có tính năng → lần `install_node`/setup kế tiếp gọi
  `expose_node_clis` ở nhánh already-installed sẽ retrofit.

## Testing

- `make_root_links`: có case cho corepack/yarn/pnpm — link khi có target trong
  `bin/`, skip khi vắng.
- `expose_node_clis` với fake `CommandRunner`:
  - `bin/corepack` có + `bin/yarn` chưa có → chạy đúng `node corepack enable`.
  - `bin/yarn` đã có → **không** gọi enable lại (idempotent, không spawn thừa).
  - `bin/corepack` vắng → không gọi enable, không lỗi.
- `tools.rs`: cập nhật test `cli_binaries` của Node cho danh sách mới.

## Ngoài phạm vi (YAGNI)

- Không thêm tuỳ chọn chọn PM (dùng mặc định `corepack enable` = yarn + pnpm).
- Không pin phiên bản yarn/pnpm — để field `packageManager` trong project tự
  quyết (đúng cơ chế corepack).
- Không tự cài corepack cho Node 25+ (khi Node bỏ bundle).
- Không đụng UI (không cần — corepack/yarn/pnpm tự lên PATH qua cơ chế sẵn có).
