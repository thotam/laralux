# Chặn thực thi dưới `.well-known` — Thiết kế

**Ngày:** 2026-07-21
**Trạng thái:** Đã duyệt (chờ review spec)

## Vấn đề

Vhost do laralux sinh ra cố tình chừa `.well-known` khỏi rule chặn dotfile:

```nginx
location ~ /\.(?!well-known).* { deny all; }
```

Điều này đúng và cần thiết — `.well-known` phải phục vụ được ACME challenge và,
với các app MCP, các endpoint OAuth discovery. Nhưng vì `.well-known` được thả
qua, request tới nó rơi xuống `location ~ \.php$` và **PHP đặt dưới
`.well-known` sẽ được thực thi**.

Đã kiểm chứng trên site thật đang chạy: đặt `public/.well-known/probe.php` chứa
`<?php echo "EXECUTED".PHP_VERSION;` rồi request thì nhận về `EXECUTED-8.4.22`.

Đây là đường leo thang quen thuộc: `.well-known` là thư mục mà nhiều công cụ
(ACME client, thư viện OAuth) ghi vào, và quyền ghi vào đó không nên đồng nghĩa
với quyền chạy code.

### Không phải vấn đề: OAuth discovery đã hoạt động

Cần nói rõ để tránh sửa nhầm: các path OAuth **đã** tới được Laravel. Kiểm chứng
bằng 5 path trong [MCP spec 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)
— tất cả trả 404 **kèm `x-powered-by: PHP/8.4.22`**, tức nginx đã chuyển tới PHP
và Laravel trả 404 vì app chưa khai route:

- `/.well-known/oauth-protected-resource` (RFC 9728 — MCP server **bắt buộc**)
- `/.well-known/oauth-protected-resource/{path}` (dạng path-insertion)
- `/.well-known/oauth-authorization-server` (RFC 8414)
- `/.well-known/openid-configuration` (OIDC Discovery)
- `/{tenant}/.well-known/openid-configuration` (OIDC path-appending — lưu ý
  `.well-known` nằm **giữa** path)

Việc khai route là của app, không phải của laralux. Spec này **không** đụng tới
phần đó.

## Hướng giải quyết

Hai rule, **cả hai đều bắt buộc**:

```nginx
location ^~ /.well-known/ {
  location ~ ^/\.well-known/(.*/)?\. { deny all; }
  location ~ \.(php|phar|phtml)$ { deny all; }
  try_files $uri $uri/ /index.php?$query_string;
}
location ~ /\.well-known/.*\.(php|phar|phtml)$ { deny all; }
```

`^~` khiến nginx **không xét các regex location bên ngoài** khi prefix này khớp,
nên `location ~ \.php$` không bao giờ chạm tới nội dung dưới `.well-known`. Tính
"không thực thi" nhờ vậy là **cấu trúc**, không phụ thuộc thứ tự khai báo hay
việc blocklist đuôi file có đủ hay không (`.phps`, `.php7`, `.pht`…).

### Dòng deny dotfile lồng bên trong là BẮT BUỘC

Chính `^~` sinh ra một regression, phát hiện khi test thật: vì nó bỏ qua mọi
regex location bên ngoài, nó cũng bỏ qua luôn rule chặn dotfile. Kết quả đo được
với bản thiết kế thiếu dòng này:

| Request | Config hiện tại | `^~` thiếu deny dotfile |
|---------|-----------------|--------------------------|
| `/.well-known/.env` | 403 | **200, trả ra `SECRET=leaked`** |

Nói cách khác, bản vá ngây thơ sẽ *tạo ra* một lỗ hổng trong khi đi bịt lỗ hổng
khác. Dòng `location ~ ^/\.well-known/(.*/)?\. { deny all; }` khôi phục lại hành
vi chặn dotfile cho mọi segment bắt đầu bằng `.` nằm dưới `.well-known`.

### Rule regex thứ hai cũng BẮT BUỘC: `.well-known` lồng

`^~` là prefix **neo đầu**, chỉ khớp path bắt đầu bằng `/.well-known/`. Nó KHÔNG
phủ dạng lồng như `/mcp/.well-known/x.php` — mà đó lại đúng hình dạng OIDC
path-appending (`/{tenant}/.well-known/openid-configuration`) mà MCP dùng.

Đo thực tế với site thật (đặt file `.php` ở cả hai vị trí):

| Thiết kế | `/.well-known/probe.php` | `/mcp/.well-known/probe.php` | OIDC lồng tới Laravel |
|----------|--------------------------|------------------------------|------------------------|
| Chưa vá | `EXECUTED-ROOT` | `EXECUTED-NESTED` | ✓ |
| Chỉ `^~` | 403 | **200 — vẫn CHẠY** | ✓ |
| `^~` + deny regex lồng | 403 | **403** | ✓ |

Vì vậy phải có thêm `location ~ /\.well-known/.*\.(php|phar|phtml)$ { deny all; }`
(regex, **không neo đầu**) để phủ `.well-known` ở mọi độ sâu. Rule này chỉ chặn
đuôi thực thi nên path OIDC lồng không có đuôi vẫn đi tiếp tới Laravel bình thường.

Đây đúng là loại lỗi "phạm vi bắt rộng nhưng phạm vi bảo vệ hẹp" — bản thiết kế
đầu tiên của chính spec này đã mắc phải và chỉ lộ ra khi đo thật.

## Các thành phần

### `core/src/sites.rs`

Chèn block trên vào **hai** chỗ, ngay sau dòng `location ~ /\.(?!well-known).*`
đã có:

1. `vhost_config` — block local (`.dev`, HTTPS + mkcert).
2. `public_vhost_block` — block public domain (80 + 443).

Cả hai đều có `root` + `location ~ \.php$` nên đều dính lỗi.

**Site kiểu proxy không cần** và không được thêm: nhánh proxy không có `root`,
không có PHP handler, mọi request đều `proxy_pass` lên upstream — nginx không
thực thi gì cả.

## Kết quả mong đợi

Đã đo trên nginx đang chạy với site thật (chèn block, `nginx -t`, SIGHUP reload,
probe, rồi khôi phục config gốc):

| Request | Trước | Sau |
|---------|-------|-----|
| `/.well-known/probe.php` | `EXECUTED-8.4.22` | **403** |
| `/mcp/.well-known/probe.php` (lồng) | `EXECUTED-NESTED` | **403** |
| `/.well-known/.env` | 403 | **403** (giữ nguyên) |
| `/.well-known/acme-token` (tĩnh) | 200 | **200** |
| `/.well-known/oauth-protected-resource` | Laravel 404 | **Laravel 404** |
| `/mcp/.well-known/openid-configuration` | Laravel | **Laravel** |

Nghĩa là: bịt được đường thực thi, không lộ source, không phá ACME, không phá
OAuth discovery.

## Testing

- `sites.rs`: vhost sinh ra cho site PHP (local **và** public) chứa
  `location ^~ /.well-known/`, chứa cả deny dotfile lẫn deny `.php` lồng bên
  trong, và giữ nguyên `try_files … /index.php?$query_string`.
- `sites.rs`: vhost đó cũng chứa rule regex `~ /\.well-known/.*\.(php|phar|phtml)$`
  cho `.well-known` lồng — test phải khẳng định rule này **không neo `^`**, vì
  neo vào là tái sinh đúng lỗ hổng đã đo được ở trên.
- `sites.rs`: vhost của site **proxy** KHÔNG chứa block `.well-known` (nhánh
  proxy không có PHP để bảo vệ).
- Thứ tự: block `.well-known` phải nằm **sau** rule dotfile-deny hiện có và
  **trước** `location ~ \.php$`, để một lần đọc file conf là thấy rõ ý đồ.

## Ngoài phạm vi (YAGNI)

- Không chặn `.sql/.zip/.bak/.tar.gz` như bản snippet của panel: nginx ở đây
  không thực thi chúng, và deny dotfile đã phủ nhóm nhạy cảm phổ biến nhất.
- Không khai route OAuth phía Laravel — đó là việc của app.
- Không đụng nhánh proxy.
- Không thêm cấu hình bật/tắt: đây là mặc định an toàn, không có lý do chính
  đáng để cho phép thực thi code dưới `.well-known`.
