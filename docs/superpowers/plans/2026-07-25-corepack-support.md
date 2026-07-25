# Corepack (yarn/pnpm) support — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (khuyến nghị) hoặc superpowers:executing-plans để triển khai theo từng task. Các step dùng checkbox (`- [ ]`).

**Goal:** Expose `corepack` và tự bật `yarn`/`pnpm` cho mỗi bản Node laralux quản, để chúng có mặt trên PATH của terminal + Procfile và (khi bật) `/usr/local/bin` — tái dùng đúng cơ chế symlink sẵn có.

**Architecture:** Thêm helper `expose_node_clis` trong `node_static.rs` chạy `corepack enable` (tạo shim yarn/pnpm trong `bin/`) rồi mở rộng `make_root_links` để symlink chúng lên version root; gọi ở cả hai nhánh install. Mở rộng `cli_binaries` của Node trong `tools.rs`. Tất cả best-effort skip-if-absent để Node 25+ (không ship corepack) vẫn sạch.

**Tech Stack:** Rust (`cargo test`). Không đụng frontend.

## Global Constraints

- KHÔNG thêm `Co-Authored-By` / chữ ký "Generated with Claude"/🤖 vào commit.
- Mọi bước **best-effort, skip-if-absent**: `bin/corepack` vắng (Node 25+) → không enable, không link corepack/yarn/pnpm, KHÔNG lỗi; install node vẫn thành công với node/npm/npx.
- `corepack enable` chạy **qua binary node tuyệt đối** (không phụ thuộc PATH) và **chỉ chạy khi `bin/corepack` có và `bin/yarn` chưa có** (idempotent, không spawn thừa).
- Thứ tự bắt buộc: enable **trước** `make_root_links` (để `bin/yarn` tồn tại khi link).
- `expose_node_clis` phải được gọi ở **cả** nhánh fresh-extract lẫn nhánh already-installed của `install_node_version` (retrofit node đã cài).
- Danh sách CLI thống nhất giữa hai file: `node, npm, npx, corepack, yarn, yarnpkg, pnpm, pnpx`.

---

### Task 1: `node_static.rs` — expose corepack + auto-enable yarn/pnpm

**Files:**
- Modify: `core/src/node_static.rs`
- Test: `core/src/node_static.rs` (module `#[cfg(test)]` sẵn có)

**Interfaces:**
- Consumes: `crate::scaffold::CommandRunner` (đã là param `runner` của `install_node_version`), `crate::scaffold::FakeCommandRunner` (trong test).
- Produces: `fn expose_node_clis(dir: &Path, runner: &dyn CommandRunner)` (private); `make_root_links` mở rộng danh sách CLI.

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/node_static.rs`:

```rust
    #[test]
    fn make_root_links_covers_corepack_and_pms() {
        let dir = std::env::temp_dir().join(format!("lara-node-cp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        for b in ["node", "npm", "corepack", "yarn", "pnpm"] {
            std::fs::write(dir.join("bin").join(b), b"x").unwrap();
        }
        // npx + pnpx intentionally absent → must be skipped, not linked.
        make_root_links(&dir);
        #[cfg(unix)]
        {
            for b in ["node", "npm", "corepack", "yarn", "pnpm"] {
                assert_eq!(
                    std::fs::read_link(dir.join(b)).unwrap(),
                    std::path::PathBuf::from(format!("bin/{b}")),
                    "root link for {b}"
                );
            }
            assert!(!dir.join("npx").exists());
            assert!(!dir.join("pnpx").exists());
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expose_enables_corepack_once_when_yarn_absent() {
        use crate::scaffold::FakeCommandRunner;
        let dir = std::env::temp_dir().join(format!("lara-node-en-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin").join("node"), b"x").unwrap();
        std::fs::write(dir.join("bin").join("corepack"), b"x").unwrap();
        // bin/yarn absent → enable should run.

        let runner = FakeCommandRunner::new();
        let calls = runner.calls();
        expose_node_clis(&dir, &runner);

        let c = calls.lock().unwrap();
        assert_eq!(c.len(), 1, "corepack enable should run exactly once");
        assert!(c[0].0.ends_with("/bin/node"), "runs via the node binary, got {}", c[0].0);
        assert_eq!(c[0].1, vec![
            dir.join("bin").join("corepack").display().to_string(),
            "enable".to_string(),
        ]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expose_skips_enable_when_yarn_present_or_corepack_absent() {
        use crate::scaffold::FakeCommandRunner;
        // yarn already there → no enable.
        let dir = std::env::temp_dir().join(format!("lara-node-sk1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin").join("node"), b"x").unwrap();
        std::fs::write(dir.join("bin").join("corepack"), b"x").unwrap();
        std::fs::write(dir.join("bin").join("yarn"), b"x").unwrap();
        let runner = FakeCommandRunner::new();
        let calls = runner.calls();
        expose_node_clis(&dir, &runner);
        assert!(calls.lock().unwrap().is_empty(), "yarn present → enable must be skipped");
        std::fs::remove_dir_all(&dir).ok();

        // corepack absent (Node 25+) → no enable, no panic.
        let dir2 = std::env::temp_dir().join(format!("lara-node-sk2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir2);
        std::fs::create_dir_all(dir2.join("bin")).unwrap();
        std::fs::write(dir2.join("bin").join("node"), b"x").unwrap();
        let runner2 = FakeCommandRunner::new();
        let calls2 = runner2.calls();
        expose_node_clis(&dir2, &runner2);
        assert!(calls2.lock().unwrap().is_empty(), "no corepack → enable must be skipped");
        std::fs::remove_dir_all(&dir2).ok();
    }
```

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core node_static::tests::expose_enables_corepack_once_when_yarn_absent`
Expected: FAIL — `expose_node_clis` chưa tồn tại (không compile).

- [ ] **Step 3: Cài đặt**

Trong `core/src/node_static.rs`:

1. Mở rộng danh sách trong `make_root_links` — đổi:

```rust
    for name in ["node", "npm", "npx"] {
```

thành:

```rust
    for name in ["node", "npm", "npx", "corepack", "yarn", "yarnpkg", "pnpm", "pnpx"] {
```

2. Thêm helper mới ngay sau `make_root_links`:

```rust
/// Ensure corepack is enabled (dropping yarn/pnpm shims into `bin/`) and expose
/// node's CLIs at the version root. Best-effort: Node 25+ ships no corepack, so
/// every step is guarded and never fails the install.
fn expose_node_clis(dir: &Path, runner: &dyn CommandRunner) {
    let corepack = dir.join("bin").join("corepack");
    let yarn = dir.join("bin").join("yarn");
    // `corepack enable` (default) writes yarn/pnpm shims next to corepack in
    // `bin/`. It is offline (shims only) and idempotent, so run it once — only
    // when corepack exists and the shims are not there yet. Run it THROUGH the
    // node binary by absolute path so it does not depend on node being on PATH.
    if corepack.exists() && !yarn.exists() {
        let node = dir.join("bin").join("node");
        let _ = runner.run(
            &node.display().to_string(),
            &[corepack.display().to_string(), "enable".to_string()],
            None,
        );
    }
    // Root symlinks (node/npm/npx/corepack/yarn/pnpm/…) — skip-if-absent.
    make_root_links(dir);
}
```

3. Gọi `expose_node_clis` ở **cả hai** nhánh của `install_node_version`:

Nhánh already-installed — đổi:

```rust
    if installed(&node_bin) {
        let _ = crate::layout::set_current(paths, "node", version);
        return Ok(version.to_string());
    }
```

thành:

```rust
    if installed(&node_bin) {
        // Retrofit an already-installed node so upgrades pick up corepack/yarn/pnpm.
        expose_node_clis(&dir, runner);
        let _ = crate::layout::set_current(paths, "node", version);
        return Ok(version.to_string());
    }
```

Nhánh fresh-extract — đổi dòng `make_root_links(&dir);` (ngay trước `set_current`) thành:

```rust
    expose_node_clis(&dir, runner);
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core node_static`
Expected: PASS toàn bộ (kể cả test cũ `root_links_point_into_bin`).

- [ ] **Step 5: Commit**

```bash
git add core/src/node_static.rs
git commit -m "feat(node): enable corepack and expose yarn/pnpm on the managed PATH"
```

---

### Task 2: `tools.rs` — thêm corepack/yarn/pnpm vào cli_binaries của Node

**Files:**
- Modify: `core/src/tools.rs`
- Test: `core/src/tools.rs` (module test sẵn có)

**Interfaces:**
- Consumes: shim yarn/pnpm do Task 1 tạo ở version root; logic skip-if-absent trong `symlinks.rs::link_tool` (đã có).
- Produces: `ToolInfo` của Node với `cli_binaries` mở rộng.

- [ ] **Step 1: Sửa test cho khớp danh sách mới**

Trong `core/src/tools.rs`, đổi assertion test (dòng ~210):

```rust
        assert_eq!(info(ManagedTool::Node).cli_binaries, &["node", "npm", "npx"]);
```

thành:

```rust
        assert_eq!(
            info(ManagedTool::Node).cli_binaries,
            &["node", "npm", "npx", "corepack", "yarn", "yarnpkg", "pnpm", "pnpx"]
        );
```

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core tools`
Expected: FAIL — `cli_binaries` hiện vẫn là `&["node", "npm", "npx"]`.

- [ ] **Step 3: Cài đặt**

Trong `core/src/tools.rs`, đổi dòng `ToolInfo` của Node (dòng ~48):

```rust
        Node => ToolInfo { key: "node", display: "Node.js", cli_binaries: &["node", "npm", "npx"], service_kind: None },
```

thành:

```rust
        Node => ToolInfo { key: "node", display: "Node.js", cli_binaries: &["node", "npm", "npx", "corepack", "yarn", "yarnpkg", "pnpm", "pnpx"], service_kind: None },
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core tools`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/tools.rs
git commit -m "feat(tools): link corepack/yarn/pnpm from Node into /usr/local/bin"
```

---

## Self-Review

**Spec coverage:**
- §1 `node_static.rs`: helper `expose_node_clis` (enable guard qua node tuyệt đối) + make_root_links mở rộng + gọi ở 2 nhánh → Task 1. ✅
- §2 `tools.rs` cli_binaries mở rộng → Task 2. ✅
- Forward-compat Node 25+ (skip-if-absent) → Task 1 test `expose_skips_enable_when_yarn_present_or_corepack_absent` + make_root_links skip; Task 2 dựa `link_tool` skip-if-absent (đã có sẵn). ✅
- Retrofit already-installed → Task 1 Step 3 sửa nhánh early-return. ✅
- Testing (make_root_links, expose 3 ca, tools) → có đủ. ✅

**Type consistency:**
- `expose_node_clis(dir: &Path, runner: &dyn CommandRunner)` — `runner` đúng kiểu param sẵn có của `install_node_version`.
- Danh sách CLI `["node","npm","npx","corepack","yarn","yarnpkg","pnpm","pnpx"]` giống hệt giữa `make_root_links` (Task 1) và `cli_binaries` (Task 2).
- Lệnh enable: `runner.run(node_path, [corepack_path, "enable"], None)` — khớp assertion test ở Task 1 Step 1.

**Placeholder scan:** không có TBD/TODO; mọi step có code/lệnh cụ thể + expected output.
