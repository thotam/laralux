# Procfile cho proxy site — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) hoặc superpowers:executing-plans để triển khai plan này theo từng task. Các step dùng cú pháp checkbox (`- [ ]`).

**Goal:** Cho proxy site một thư mục project tuỳ chọn, để chức năng Procfile (chạy process theo site) dùng được cho proxy — ví dụ tự chạy `npm run dev` cho upstream Next/Vite.

**Architecture:** Thêm `root: Option<PathBuf>` vào `ProxySite` trong registry và điền nó vào `Site.root` khi liệt kê. Toàn bộ máy móc process sẵn có (`procfile.rs`, `site_procs.rs`, các command proc) đã key theo `root` nên **không phải sửa logic** — chỉ thêm một guard cho root rỗng. Tầng UI thêm field chọn folder trong modal Reverse proxy và mở khoá các nút vốn bị ẩn cho proxy.

**Tech Stack:** Rust (core lib + Tauri commands, `cargo test`), TypeScript/Vite frontend (không có test harness — verify bằng `npm run build`).

## Global Constraints

- KHÔNG thêm `Co-Authored-By` hay chữ ký "Generated with Claude"/🤖 vào commit.
- Folder của proxy là **tuỳ chọn**; `sites.toml` cũ (proxy chưa có `root`) phải load nguyên vẹn (`#[serde(default)]`).
- Folder **KHÔNG** trở thành document root — vhost nginx của proxy giữ nguyên, không đổi một dòng nào.
- **Routing không bao giờ phụ thuộc vào folder**: proxy có `root` nhưng thư mục đã mất thì site **vẫn phải còn** trong danh sách (chỉ warning), tuyệt đối không skip như linked site.
- **Proxy không bao giờ có nút xoá-khỏi-đĩa.** Chỉ *Scanned* mới gọi `delete_scanned_site`; *Proxy* và *Linked* chỉ gỡ entry registry.
- Theo đúng pattern sẵn có của codebase (folder picker tái dùng `openDialog` như Link Site).

---

### Task 1: Registry — `ProxySite.root` + validate

**Files:**
- Modify: `core/src/site_registry.rs`
- Test: `core/src/site_registry.rs` (module `#[cfg(test)]` sẵn có)

**Interfaces:**
- Produces:
  - `ProxySite` có thêm `pub root: Option<PathBuf>`
  - `SiteRegistry::add_proxy(&mut self, name: &str, routes: &[ProxyRoute], websocket: bool, root: Option<&Path>) -> Result<(), RegistryError>`
  - `SiteRegistry::update_proxy(&mut self, name: &str, routes: &[ProxyRoute], websocket: bool, root: Option<&Path>) -> Result<(), RegistryError>`

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/site_registry.rs`:

```rust
    #[test]
    fn proxy_root_is_optional_validated_and_roundtrips() {
        let r = root();
        let proj = r.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let routes = vec![ProxyRoute { path: "/".into(), upstream: "3000".into() }];

        let mut reg = SiteRegistry::default();
        // không có folder -> None
        reg.add_proxy("plain", &routes, true, None).unwrap();
        assert_eq!(reg.proxies().iter().find(|p| p.name == "plain").unwrap().root, None);

        // có folder -> lưu lại
        reg.add_proxy("withdir", &routes, true, Some(&proj)).unwrap();
        let saved = reg.proxies().iter().find(|p| p.name == "withdir").unwrap().root.clone().unwrap();
        assert!(saved.is_dir());

        // folder không tồn tại -> RootNotFound
        assert!(matches!(
            reg.add_proxy("bad", &routes, true, Some(&r.join("nope"))),
            Err(RegistryError::RootNotFound(_))
        ));

        // update_proxy đổi được root (kể cả gỡ về None)
        reg.update_proxy("plain", &routes, true, Some(&proj)).unwrap();
        assert!(reg.proxies().iter().find(|p| p.name == "plain").unwrap().root.is_some());
        reg.update_proxy("plain", &routes, true, None).unwrap();
        assert_eq!(reg.proxies().iter().find(|p| p.name == "plain").unwrap().root, None);

        // save/load roundtrip
        let file = r.join("sites.toml");
        reg.save(&file).unwrap();
        let back = SiteRegistry::load(&file).unwrap();
        assert!(back.proxies().iter().find(|p| p.name == "withdir").unwrap().root.is_some());
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn old_sites_toml_proxy_without_root_loads() {
        let r = root();
        std::fs::create_dir_all(&r).unwrap();
        let file = r.join("sites.toml");
        std::fs::write(
            &file,
            "[[proxies]]\nname = \"next\"\nwebsocket = true\n\n[[proxies.routes]]\npath = \"/\"\nupstream = \"127.0.0.1:3000\"\n",
        )
        .unwrap();
        let reg = SiteRegistry::load(&file).unwrap();
        let p = reg.proxies().iter().find(|p| p.name == "next").unwrap();
        assert_eq!(p.root, None);
        assert!(p.websocket);
        std::fs::remove_dir_all(&r).ok();
    }
```

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core site_registry::tests::proxy_root_is_optional_validated_and_roundtrips`
Expected: FAIL — không compile (`add_proxy` chưa nhận tham số thứ 4, `ProxySite` chưa có field `root`).

- [ ] **Step 3: Cài đặt tối thiểu**

Trong `core/src/site_registry.rs`:

1. Thêm field vào `ProxySite` (sau `routes`):

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
```

2. Thêm helper trước `impl SiteRegistry` (cạnh `validate_routes`):

```rust
/// Validate an optional proxy project folder: it must be an existing directory.
/// Returns the canonicalized path, falling back to the given path if
/// canonicalize fails (mirrors what `SiteRegistry::add` does for linked sites).
fn normalize_proxy_root(root: Option<&Path>) -> Result<Option<PathBuf>, RegistryError> {
    match root {
        None => Ok(None),
        Some(p) => {
            if !p.is_dir() {
                return Err(RegistryError::RootNotFound(p.display().to_string()));
            }
            Ok(Some(std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())))
        }
    }
}
```

3. Đổi `add_proxy` — thêm tham số `root` và dùng helper:

```rust
    pub fn add_proxy(
        &mut self,
        name: &str,
        routes: &[ProxyRoute],
        websocket: bool,
        root: Option<&Path>,
    ) -> Result<(), RegistryError> {
        validate_site_name(name).map_err(|_| RegistryError::InvalidName(name.to_string()))?;
        if self.sites.iter().any(|s| s.name == name) || self.proxies.iter().any(|p| p.name == name) {
            return Err(RegistryError::Duplicate(name.to_string()));
        }
        let routes = validate_routes(routes)?;
        let root = normalize_proxy_root(root)?;
        self.proxies.push(ProxySite { name: name.to_string(), websocket, routes, root });
        Ok(())
    }
```

4. Đổi `update_proxy` tương tự:

```rust
    pub fn update_proxy(
        &mut self,
        name: &str,
        routes: &[ProxyRoute],
        websocket: bool,
        root: Option<&Path>,
    ) -> Result<(), RegistryError> {
        let routes = validate_routes(routes)?;
        let root = normalize_proxy_root(root)?;
        let p = self
            .proxies
            .iter_mut()
            .find(|p| p.name == name)
            .ok_or_else(|| RegistryError::NotFound(name.to_string()))?;
        p.routes = routes;
        p.websocket = websocket;
        p.root = root;
        Ok(())
    }
```

5. Sửa mọi lời gọi `add_proxy`/`update_proxy` cũ trong module test của chính file này (các test `add_proxy_rejects_duplicate_across_lists`, `update_proxy_replaces_or_errors_not_found`, `remove_handles_proxies_and_old_file_loads`) — thêm `None` làm tham số cuối.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core site_registry`
Expected: PASS toàn bộ.

- [ ] **Step 5: Commit**

```bash
git add core/src/site_registry.rs
git commit -m "feat(registry): optional project folder for proxy sites"
```

---

### Task 2: `list_all_sites` — điền root cho proxy, folder mất thì cảnh báo (không skip)

**Files:**
- Modify: `core/src/sites.rs`
- Test: `core/src/sites.rs`

**Interfaces:**
- Consumes: `ProxySite.root` (Task 1).
- Produces: `Site.root` được điền cho proxy site có folder; proxy có folder đã mất vẫn nằm trong danh sách trả về.

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/sites.rs`:

```rust
    #[test]
    fn proxy_site_gets_root_from_registry() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www")).unwrap();
        let proj = root.join("nodeapp");
        std::fs::create_dir_all(&proj).unwrap();
        let paths = LaraluxPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        let routes = vec![crate::site_registry::ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true, Some(&proj)).unwrap();
        reg.save(&paths.sites_file()).unwrap();

        let (sites, warnings) = list_all_sites(&paths, "dev").unwrap();
        let api = sites.iter().find(|s| s.name == "api").unwrap();
        assert!(api.root.is_dir(), "proxy root should be populated");
        assert!(warnings.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn proxy_with_missing_folder_is_kept_with_warning() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("www")).unwrap();
        let proj = root.join("gone");
        std::fs::create_dir_all(&proj).unwrap();
        let paths = LaraluxPaths::new(root.clone());

        let mut reg = crate::site_registry::SiteRegistry::default();
        let routes = vec![crate::site_registry::ProxyRoute { path: "/".into(), upstream: "3000".into() }];
        reg.add_proxy("api", &routes, true, Some(&proj)).unwrap();
        reg.save(&paths.sites_file()).unwrap();
        // Folder biến mất SAU khi đã đăng ký.
        std::fs::remove_dir_all(&proj).unwrap();

        let (sites, warnings) = list_all_sites(&paths, "dev").unwrap();
        // Routing không được sập: site vẫn còn, chỉ mất khả năng chạy process.
        let api = sites.iter().find(|s| s.name == "api").expect("proxy must survive a missing folder");
        assert_eq!(api.source, SiteSource::Proxy);
        assert_eq!(api.root, std::path::PathBuf::new());
        assert!(api.proxy.is_some());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("api"));
        std::fs::remove_dir_all(&root).ok();
    }
```

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core sites::tests::proxy_site_gets_root_from_registry`
Expected: FAIL — `api.root.is_dir()` sai vì proxy vẫn đang set root rỗng.

- [ ] **Step 3: Cài đặt tối thiểu**

Trong `core/src/sites.rs::list_all_sites`, thay phần thân vòng `for p in registry.proxies()` — đoạn đang tạo `Site` với `root: std::path::PathBuf::new()` — bằng:

```rust
    for p in registry.proxies() {
        if sites.iter().any(|s| s.name == p.name) {
            warnings.push(format!("proxy site `{}` is shadowed by another site", p.name));
            continue;
        }
        // A proxy's folder is optional and only powers Procfile processes. If it
        // has gone missing, keep serving the route and just warn — unlike a
        // linked site, a proxy must never disappear because of its folder.
        let root = match &p.root {
            Some(r) if r.is_dir() => r.clone(),
            Some(r) => {
                warnings.push(format!(
                    "proxy site `{}`: folder `{}` not found; processes unavailable",
                    p.name,
                    r.display()
                ));
                std::path::PathBuf::new()
            }
            None => std::path::PathBuf::new(),
        };
        let hostname = format!("{}.{}", p.name, tld);
        sites.push(Site {
            domains: vec![hostname.clone()],
            hostname,
            root,
            name: p.name.clone(),
            source: SiteSource::Proxy,
            proxy: Some(ProxySpec { routes: p.routes.clone(), websocket: p.websocket }),
            public_domains: Vec::new(),
        });
    }
```

Sửa các lời gọi `add_proxy` trong module test của file này (ví dụ `list_all_includes_proxy_sites`, `list_all_scanned_shadows_proxy_of_same_name`) — thêm `None` làm tham số cuối.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core sites`
Expected: PASS toàn bộ.

- [ ] **Step 5: Commit**

```bash
git add core/src/sites.rs
git commit -m "feat(sites): populate proxy root; keep proxy when its folder is gone"
```

---

### Task 3: `read_procfile` — guard root rỗng

**Files:**
- Modify: `core/src/procfile.rs`
- Test: `core/src/procfile.rs`

**Interfaces:**
- Produces: `read_procfile` trả `None` khi `site_root` rỗng (không đọc `Procfile` tương đối theo CWD).

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/procfile.rs`:

```rust
    #[test]
    fn empty_root_never_reads_cwd_procfile() {
        // Một proxy site không có folder mang root rỗng. Nếu không guard,
        // `"".join("Procfile")` thành đường dẫn tương đối `Procfile` và sẽ đọc
        // nhầm file nằm trong CWD của tiến trình.
        let dir = std::env::temp_dir().join(format!("lara-proc-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Procfile"), b"web: should-not-be-read\n").unwrap();
        std::env::set_current_dir(&dir).unwrap();

        assert!(read_procfile(std::path::Path::new("")).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
```

Lưu ý người thực thi: test này đổi CWD của tiến trình test. Chạy nó **một mình** bằng `--test-threads=1` như ở Step 2 để không ảnh hưởng test khác; nếu thấy nó gây nhiễu cho các test song song khác trong lần chạy full suite, hãy báo lại là concern thay vì tự ý đổi thiết kế.

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core procfile::tests::empty_root_never_reads_cwd_procfile -- --test-threads=1`
Expected: FAIL — hiện đang đọc được `Procfile` trong CWD nên trả `Some(...)`.

- [ ] **Step 3: Cài đặt tối thiểu**

Trong `core/src/procfile.rs`, sửa `read_procfile`:

```rust
pub fn read_procfile(site_root: &Path) -> Option<Vec<ProcEntry>> {
    // An empty root means "no folder" (e.g. a proxy site without a project
    // folder). Without this guard the join yields the relative path `Procfile`,
    // reading whatever happens to sit in the process's working directory.
    if site_root.as_os_str().is_empty() {
        return None;
    }
    match std::fs::read_to_string(site_root.join("Procfile")) {
        Ok(text) => Some(parse_procfile(&text)),
        Err(_) => None,
    }
}
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core procfile -- --test-threads=1`
Expected: PASS toàn bộ.

- [ ] **Step 5: Commit**

```bash
git add core/src/procfile.rs
git commit -m "fix(procfile): empty site root reads no Procfile"
```

---

### Task 4: Tauri commands — `add_proxy` / `update_proxy` nhận `root`

**Files:**
- Modify: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `SiteRegistry::add_proxy(.., root: Option<&Path>)` và `update_proxy(.., root: Option<&Path>)` (Task 1).
- Produces: command `add_proxy(name, routes, websocket, root: Option<String>)` và `update_proxy(name, routes, websocket, root: Option<String>)` — cả hai vẫn trả `Site`.

- [ ] **Step 1: Sửa command `add_proxy`**

Trong `src-tauri/src/commands.rs`, thêm tham số `root` vào chữ ký và truyền xuống registry (phần còn lại giữ nguyên):

```rust
#[tauri::command]
pub async fn add_proxy(
    app: tauri::AppHandle,
    name: String,
    routes: Vec<ProxyRoute>,
    websocket: bool,
    root: Option<String>,
) -> Result<Site, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Site, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry =
            SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        let root_path = root.as_deref().filter(|s| !s.is_empty()).map(Path::new);
        registry
            .add_proxy(&name, &routes, websocket, root_path)
            .map_err(|e| e.to_string())?;
        registry.save(&state.paths.sites_file()).map_err(|e| e.to_string())?;

        sync_and_reload(&state, &config);

        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        sites
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("proxy `{name}` not found after sync"))
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 2: Sửa command `update_proxy` giống hệt**

Tìm command `update_proxy` trong cùng file, thêm `root: Option<String>` vào chữ ký, và đổi lời gọi registry thành:

```rust
        let root_path = root.as_deref().filter(|s| !s.is_empty()).map(Path::new);
        registry
            .update_proxy(&name, &routes, websocket, root_path)
            .map_err(|e| e.to_string())?;
```

- [ ] **Step 3: Build để xác minh**

Run: `cargo check -p laralux-desktop`
Expected: build sạch, không lỗi. (Tên package là `laralux-desktop`, không phải `laralux`.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat(tauri): accept an optional project folder for proxy sites"
```

---

### Task 5: Frontend — chọn folder cho proxy + mở khoá nút

**Files:**
- Modify: `src/ipc/commands.ts` (chữ ký `addProxy`/`updateProxy`)
- Modify: `src/state.ts` (`ProxyState.root`)
- Modify: `src/ui/modals/proxy.ts` (field folder + nút Browse)
- Modify: `src/ui/views/sites.ts` (`browseProxyFolder`, `openProxy`, `submitProxy`, đổi gate ẩn nút)
- Modify: `src/ui/events.ts` (handler `px-browse`)

**Interfaces:**
- Consumes: command `add_proxy`/`update_proxy` có `root` (Task 4); `Site.root` đã được điền cho proxy (Task 2).
- Produces: `state.proxy.root: string` (chuỗi rỗng = không có folder).

- [ ] **Step 1: IPC**

Trong `src/ipc/commands.ts`, thay hai export `addProxy` và `updateProxy` thành:

```ts
export const addProxy = (
  name: string,
  routes: ProxyRoute[],
  websocket: boolean,
  root?: string | null,
): Promise<Site> =>
  invoke<Site>("add_proxy", { name, routes, websocket, root: root || null });

/**
 * Update an existing reverse proxy site.
 * Arg keys: `name`, `routes`, `websocket`, `root`.
 */
export const updateProxy = (
  name: string,
  routes: ProxyRoute[],
  websocket: boolean,
  root?: string | null,
): Promise<Site> =>
  invoke<Site>("update_proxy", { name, routes, websocket, root: root || null });
```

- [ ] **Step 2: State**

Trong `src/state.ts`, thêm field vào `ProxyState` (sau `routes`):

```ts
  root: string;
```

- [ ] **Step 3: Modal — field folder + Browse**

Trong `src/ui/modals/proxy.ts`, chèn khối sau ngay **trước** dòng checkbox WebSocket (`'<label class="ns-check">...'`):

```ts
    '<label class="ns-label">Project folder (optional)</label>' +
    '<div class="pr-row">' +
    '<input class="ns-input" type="text" placeholder="Chưa chọn — dùng để chạy Procfile" value="' + esc(p.root) + '" readonly' + d + ' />' +
    '<button class="btn btn-outline" data-action="px-browse"' + d + '>Browse</button>' +
    (p.root ? '<button class="icon-btn sq32" data-action="px-root-clear" aria-label="Clear folder"' + d + '>' + I.close + '</button>' : '') +
    '</div>' +
    '<div class="ns-hint">Chọn thư mục project để chạy các process khai báo trong <code>Procfile</code>. Bỏ trống nếu không cần.</div>' +
```

- [ ] **Step 4: View helpers**

Trong `src/ui/views/sites.ts`:

1. Thêm hàm chọn folder cho proxy (đặt cạnh `browseFolder`):

```ts
export async function browseProxyFolder(): Promise<void> {
  try {
    const picked = await openDialog({ directory: true, multiple: false, title: "Choose project folder" });
    if (!picked) return; // cancelled
    state.proxy.root = Array.isArray(picked) ? picked[0] : picked;
    state.proxy.error = "";
    render();
  } catch (e) {
    toast({ type: "error", title: "Folder picker failed", msg: String(e) });
  }
}

export function clearProxyFolder(): void {
  state.proxy.root = "";
  render();
}
```

2. Trong `openProxy`, thêm `root` vào cả hai nhánh khởi tạo state:

```ts
export function openProxy(site?: Site): void {
  if (site && site.proxy) {
    state.proxy = {
      mode: "edit", name: site.name, websocket: !!site.proxy.websocket,
      routes: (site.proxy.routes || []).map((r) => ({ path: r.path, upstream: r.upstream })),
      root: site.root || "",
      busy: false, error: "",
    };
    if (!state.proxy.routes.length) state.proxy.routes = [{ path: "/", upstream: "" }];
  } else {
    state.proxy = { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], root: "", busy: false, error: "" };
  }
  state.modal = "proxy";
  render();
  requestAnimationFrame(() => { const inp = document.getElementById("px-name") as HTMLInputElement | null; if (inp && !inp.readOnly) inp.focus(); });
}
```

3. Trong `submitProxy`, truyền `root` và reset nó khi thành công. Đổi lời gọi:

```ts
    const site = await (p.mode === "edit"
      ? updateProxy(p.name, routes, p.websocket, p.root)
      : addProxy(p.name, routes, p.websocket, p.root));
```

và đổi dòng reset state sau khi thành công thành:

```ts
    state.proxy = { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], root: "", busy: false, error: "" };
```

4. **Mở khoá nút cho proxy có folder** — trong `sitesView`, đổi ba dòng đang gate bằng `isProxy` (đường dẫn root, nút mở folder, nút mở terminal) sang gate bằng `!s.root`:

```ts
          const subRight = !s.root ? "" : '<span class="site-root" title="' + esc(s.root) + '">' + esc(s.root) + "</span>";
          const folderBtn = !s.root
            ? ""
            : '<button class="icon-btn sq32" data-action="open-folder" data-path="' + esc(s.root) + '" aria-label="Open folder" title="Open project folder">' + I.folder + "</button>";
          const termBtn = !s.root
            ? ""
            : '<button class="icon-btn sq32" data-action="open-terminal" data-path="' + esc(s.root) + '" aria-label="Open terminal" title="Open terminal here">' + I.terminal + "</button>";
```

Giữ nguyên biến `isProxy` — nó vẫn được dùng cho badge, `target`, và mục "Edit proxy".

- [ ] **Step 5: Events**

Trong `src/ui/events.ts`:

1. Import thêm `browseProxyFolder, clearProxyFolder` từ `./views/sites` (cùng chỗ đang import `openProxy`).
2. Thêm hai nhánh click cạnh các action `px-*` sẵn có:

```ts
    else if (a === "px-browse") { browseProxyFolder(); }
    else if (a === "px-root-clear") { clearProxyFolder(); }
```

Dùng đúng cách phân nhánh mà các action `px-close` / `px-submit` đang dùng trong file (nếu chúng nằm trong một chuỗi `if/else if` theo biến action thì thêm vào chuỗi đó).

- [ ] **Step 6: Typecheck + build**

Run: `npm run build`
Expected: `tsc --noEmit` pass, vite build thành công. Sửa hết lỗi type (thường là thiếu `root` ở một chỗ khởi tạo `state.proxy` nào đó — tìm tất cả `mode: "create"` để chắc chắn).

- [ ] **Step 7: Commit**

```bash
git add src/ipc/commands.ts src/state.ts src/ui/modals/proxy.ts src/ui/views/sites.ts src/ui/events.ts
git commit -m "feat(ui): pick a project folder for proxy sites"
```

---

### Task 6: Modal xoá — nói rõ folder được giữ lại

**Files:**
- Modify: `src/ui/modals/deletesite.ts`

**Interfaces:**
- Consumes: `state.deleteSite.root` (đã có sẵn), `source === "Proxy"`.

- [ ] **Step 1: Sửa modal**

Trong `src/ui/modals/deletesite.ts`:

1. Đổi dòng `info` (đang ẩn đường dẫn với mọi proxy) để hiện đường dẫn bất cứ khi nào có root:

```ts
  const info =
    '<div class="ns-label">' + esc(d.name) + "</div>" +
    '<div class="ds-url">' + esc(d.url) + "</div>" +
    (d.root ? '<div class="ds-root" title="' + esc(d.root) + '">' + esc(d.root) + "</div>" : "");
```

2. Đổi nhánh `else` (proxy) để trấn an khi có folder — chỉ đổi `body`, **giữ nguyên `footer` chỉ có Cancel + Remove** (proxy không bao giờ được có nút Delete-from-disk):

```ts
  } else {
    body =
      "<p>Removes the reverse-proxy <b>" + esc(d.name) + "</b> from Laralux.</p>" +
      (d.root
        ? "<p>Your project folder <code>" + esc(d.root) + "</code> is kept.</p>"
        : "");
    footer =
      '<button class="btn btn-outline" data-action="ds-close"' + dis + ">Cancel</button>" +
      '<button class="btn btn-danger" data-action="ds-remove"' + dis + ">Remove</button>";
  }
```

- [ ] **Step 2: Typecheck + build**

Run: `npm run build`
Expected: pass.

- [ ] **Step 3: Kiểm tra bất biến bằng mắt**

Đọc lại `deleteSiteModal` và xác nhận: chỉ nhánh `d.source === "Scanned"` mới có `data-action="ds-delete"`. Nhánh Linked và Proxy chỉ có `ds-remove`. Nếu không đúng, dừng lại và báo.

- [ ] **Step 4: Commit**

```bash
git add src/ui/modals/deletesite.ts
git commit -m "feat(ui): delete modal states that a proxy's folder is kept"
```

---

## Self-Review

**Spec coverage:**
- §1 Data model (`ProxySite.root`, add/update, validate) → Task 1. ✅
- §2 Site model (điền root; folder mất → warning, không skip) → Task 2. ✅
- §3 Process (không sửa logic; guard root rỗng) → Task 3. ✅
- §4 UI (modal folder; gate `isProxy` → `!s.root`) → Task 5. ✅
- §5 Xoá site (hiện đường dẫn + "is kept"; giữ bất biến không có Delete) → Task 6. ✅
- §6 Command layer (`root: Option<String>`) → Task 4. ✅
- Testing (spec) → phủ trong Task 1–3; phần UI verify bằng `npm run build` + kiểm tra bất biến ở Task 6 Step 3.

**Type consistency:**
- `root: Option<&Path>` (tham số) ↔ `Option<PathBuf>` (field) nhất quán Task 1 → 2 → 4.
- `Option<String>` (Tauri command) ↔ `string` (TS state, rỗng = không có) nhất quán Task 4 ↔ 5; command lọc chuỗi rỗng thành `None` (`filter(|s| !s.is_empty())`).
- `state.proxy.root` được khởi tạo ở **cả ba** chỗ tạo `ProxyState` (hai nhánh `openProxy`, một chỗ reset trong `submitProxy`) — Task 5 Step 4 liệt kê đủ.
- Action strings `px-browse` / `px-root-clear` khai ở modal (Task 5 Step 3) và có handler ở events (Task 5 Step 5).

**Placeholder scan:** Task 3 Step 1 chủ ý cảnh báo về việc test đổi CWD (yêu cầu chạy `--test-threads=1` và báo lại nếu gây nhiễu) — đây là hướng dẫn cụ thể, không phải placeholder.
