# Readiness gating + auto-restart cho Procfile process — Thiết kế

**Ngày:** 2026-07-21
**Trạng thái:** Đã duyệt (chờ review spec)

## Vấn đề

Người dùng khai báo trong `Procfile` của site:

```procfile
queue_ai: php artisan queue:work --sleep 3 --tries=3 --queue=ai_cpc1hn
```

Khi laralux khởi động stack rồi autostart process này, log
`~/laralux/log/proc-online-queue_ai.log` báo:

```
SQLSTATE[HY000] [2002] Connection refused
(Connection: mariadb, Host: 127.0.0.1, Port: 3306, ...)
```

Worker chết ngay lúc khởi động và **nằm chết luôn**. Hai khiếm khuyết độc lập:

### A. Process chạy trước khi service sẵn sàng

`run_full_start` gọi `orch.start_all()` rồi autostart Procfile ngay, kèm comment
*"once the stack is up"*. Nhưng `start_all()` → `start()` → `do_start()` chỉ
**spawn** tiến trình rồi đánh dấu `Running` — không hề chờ service **nhận được
kết nối**. MariaDB cần vài giây mới mở cổng 3306, nên worker connect ngay và chết.

Đáng chú ý: hạ tầng kiểm tra sẵn sàng **đã có đủ nhưng chưa đấu dây**:

- Trait `Service` có `health_check(&self, paths)` và **mọi** service đều implement:
  `probe_tcp(port)` cho mariadb / redis / postgres / mongodb / nginx / mailpit,
  kiểm tra socket tồn tại cho php-fpm.
- Có sẵn helper `probe_tcp(port)` (TCP connect, timeout 1s).
- **Không có call site nào** — `health_check` hiện là dead code.

### B. Không có auto-restart

`SiteProcs::refresh()` phát hiện handle đã chết thì chỉ gỡ handle và đánh dấu
`Crashed` — không bao giờ respawn. Process chết là nằm im tới khi bấm start tay.

Thêm nữa, `SiteProcs` **không lưu `command` và `root`** của process (chỉ giữ
`handles`), nên hiện tại kể cả muốn cũng không respawn được.

## Hướng giải quyết

Làm cả hai trong một spec (đã chốt với người dùng), vì cùng phục vụ một kết quả:
process của site chạy đúng lúc và tự hồi phục.

Quyết định đã chốt:

- Chờ readiness **trong `Orchestrator::start()`** (sửa đúng gốc — trạng thái
  `Running` toàn app trở thành sự thật), không phải chỉ gating riêng cho Procfile.
- Hết timeout mà chưa sẵn sàng → **`Crashed` + báo lỗi** (không báo Running dối).
- Restart kiểu systemd: **backoff tăng dần + bỏ cuộc sau N lần + reset bộ đếm**
  khi process đã sống ổn định đủ lâu.
- **Luôn restart bất kể exit code.** Lý do: `php artisan queue:work` thoát với
  code 0 khi nhận tín hiệu `queue:restart` và Laravel yêu cầu process monitor
  chạy lại. Nếu chỉ restart khi exit ≠ 0 thì worker sẽ chết vĩnh viễn sau mỗi
  lần deploy. Đánh đổi: entry một-lần (vd `migrate: php artisan migrate`) sẽ bị
  chạy lại tới khi hết lượt retry — chấp nhận được vì Procfile vốn dành cho
  process chạy dài.

## Các thành phần

### 1. Readiness gating — `core/src/orchestrator.rs`

Trong `start()`, sau khi `do_start()` thành công: poll `svc.health_check(&paths)`
mỗi **100ms** cho tới khi `Ok` rồi mới đặt `Running`. Trong lúc chờ giữ nguyên
trạng thái `Starting` (UI hiển thị đúng "đang khởi động").

Timeout tái dùng thông tin `start()` đã tính sẵn:

| Điều kiện | Timeout |
|-----------|---------|
| `needs_init == true` (MariaDB/Postgres lần đầu tạo data dir) | **60s** |
| bình thường | **15s** |

Hết giờ → đặt `Crashed` và trả `ServiceError::HealthCheck`.

Để test được mà không `sleep`: timeout và interval là **field của `Orchestrator`**
với giá trị mặc định như trên, test set giá trị rất nhỏ.

### 2. Tray không được block main thread — `src-tauri/src/main.rs`

*(Lỗi phát hiện khi khảo sát, sửa kèm vì mục 1 làm nó nặng hơn hẳn.)*

Nhánh `stack_toggle` trong `on_menu_event` gọi thẳng `commands::run_full_start(&state)`
mà không bọc thread. Tauri giao event tray trên main thread (GTK), nên hôm nay
"Start All" từ tray đã block UI; cộng thêm tối đa 60s chờ readiness sẽ thành đơ
rõ rệt.

Sửa: bọc phần xử lý trong `std::thread::spawn`, theo đúng mẫu đường
`launch.autostart_services` trong cùng file đã dùng.

### 3. Auto-restart — `core/src/site_procs.rs`

`SiteProcs` thêm bảng giám sát theo từng `(site, proc)`:

```rust
struct Supervised {
    root: PathBuf,
    command: String,
    failures: u32,            // số lần chết liên tiếp
    started_at: Instant,
    next_attempt_at: Option<Instant>,
    supervised: bool,         // false sau khi user stop tay hoặc sau khi bỏ cuộc
}
```

Hằng số:

| Tham số | Giá trị |
|---------|---------|
| Backoff trước lần thử lại thứ n | n=1 → **1s**, n=2 → **5s**, n=3 → **15s**, n=4 → **30s** |
| Bỏ cuộc sau | **5** lần chết liên tiếp (lần thứ 5 không thử lại nữa) |
| Reset bộ đếm khi sống ổn định | **30s** |

Nghĩa là mỗi chu kỳ có tối đa **4 lần thử lại** (chờ 1s, 5s, 15s, 30s) rồi tới
lần chết thứ 5 thì dừng hẳn — tổng cộng ~51s trước khi bỏ cuộc. Khoảng này đủ
rộng để một service khởi động chậm (MariaDB tạo data dir lần đầu) kịp lên trước
khi process của site bỏ cuộc.

Nhịp giám sát nằm trong `refresh()` (vòng nền trong `main.rs` đã gọi sẵn ~1s/lần):

1. Handle còn sống, uptime ≥ 30s, `failures > 0` → **reset** `failures = 0`.
   (Phân biệt crash-loop với sự cố lẻ tẻ: worker chạy cả tiếng rồi mới chết sẽ
   được retry lại từ đầu thay vì bị tính dồn.)
2. Handle đã chết và `supervised` → `failures += 1`.
   - `failures >= 5` → `Crashed`, `supervised = false`, thôi retry.
   - ngược lại → hẹn `next_attempt_at = now + backoff(failures)`.
3. Tới `next_attempt_at` → respawn bằng `root` + `command` đã lưu, đặt lại
   `started_at`. Restart **bất kể exit code**.

**Hệ quả đã cân nhắc:** trần backoff (30s) trùng ngưỡng reset (30s), nên một
process cứ sống đúng ~30s rồi chết sẽ reset bộ đếm mỗi lần và được restart vô
hạn. Chấp nhận: đó không phải crash-loop đốt CPU (mỗi 30s một lần, và nó có làm
việc thật giữa hai lần chết) — tương tự hành vi `StartLimitIntervalSec` của
systemd. Chỉ crash nhanh liên tiếp mới bị bỏ cuộc.

Hai bất biến bắt buộc:

- **Stop tay không bao giờ bị hồi sinh** — `stop()` / `stop_site()` / `stop_all()`
  đặt `supervised = false`.
- **Start tay reset bộ đếm** — `start()` đặt `failures = 0`, `supervised = true`.

Để test được mà không `sleep`: tách `tick_at(now: Instant)` chứa toàn bộ logic
trên; `refresh()` chỉ là `self.tick_at(Instant::now())`. Test bơm mốc thời gian
giả để kiểm backoff/reset/bỏ cuộc chạy tức thì.

### 4. UI — `ProcStatus` + modal Processes

`ProcStatus` thêm `failures: u32` để modal phân biệt được:

- *"Retrying (2/5)…"* — đang trong chu kỳ backoff;
- *"Crashed — gave up after 5 restarts"* — đã thôi retry.

Không có thông tin này người dùng chỉ thấy `Crashed` mà không hiểu vì sao process
thôi không tự dậy nữa.

## Luồng dữ liệu

1. Người dùng bấm Start All (UI hoặc tray, tray giờ chạy trên thread riêng).
2. `orch.start_all()` khởi động từng service theo thứ tự phụ thuộc; mỗi service
   chỉ chuyển `Running` sau khi `health_check` pass (hoặc `Crashed` khi hết giờ).
3. `run_full_start` autostart Procfile của các site có bật — lúc này MariaDB đã
   thật sự nhận kết nối, nên `queue:work` không còn `Connection refused`.
4. Vòng nền gọi `refresh()` ~1s/lần: reset bộ đếm cho process sống ổn định,
   respawn theo backoff cho process chết, dừng hẳn sau 5 lần liên tiếp.

## Xử lý lỗi

- Service không sẵn sàng trong timeout → `Crashed` + `ServiceError::HealthCheck`,
  hiển thị lên UI như lỗi start hiện tại.
- Respawn thất bại (spawn lỗi) → tính như một lần chết, vào backoff bình thường.
- Process bỏ cuộc sau 5 lần → `Crashed`, `supervised = false`; user start tay là
  chạy lại từ đầu với bộ đếm sạch.
- Site bị xoá / Procfile đổi khi process đang chạy: `refresh()` chỉ respawn từ
  `command` đã lưu lúc start; muốn áp dụng Procfile mới thì stop rồi start lại
  (giữ nguyên hành vi hiện tại, không thêm reload ngầm).

## Testing

- `orchestrator.rs`: health pass ngay → `Running`; pass sau vài nhịp poll →
  `Running`; không bao giờ pass → `Crashed` + `Err(HealthCheck)`; dùng timeout
  nhỏ nên test chạy tức thì.
- `site_procs.rs` (qua `tick_at` với mốc thời gian giả): backoff đúng khoảng
  cách 1/5/15/30s; bỏ cuộc sau đúng 5 lần chết liên tiếp (4 lần thử lại); reset
  bộ đếm sau 30s uptime; **stop tay không bị respawn**; start tay reset `failures`.
- Không test nào được `sleep` theo thời gian thật.

## Ngoài phạm vi (YAGNI)

- Không thêm cú pháp mới trong `Procfile`.
- Không có phụ thuộc giữa các process (process A chờ process B).
- Không cho cấu hình số lần retry / backoff (hằng số cho tới khi có nhu cầu thật).
- Không tự nạp lại `Procfile` khi file đổi lúc process đang chạy.
