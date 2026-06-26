# Laralux — UI Redesign Brief (for Claude Design)

This is a complete, implementation-grounded brief to redesign the desktop UI of **Laralux**. It is written so the resulting design can be implemented as-is in the existing tech stack. Read every section — the data contract and technical constraints are binding.

---

## 1. Product context

- **What it is:** A lightweight desktop app that manages a local web-dev stack on Linux (Ubuntu), like Laralux (Windows) / Laravel Herd (macOS) / DBngin. It installs, starts/stops, and configures: **nginx, PHP-FPM, MariaDB, Redis, Mailpit**, and auto-creates pretty HTTPS URLs (`*.dev`) per project.
- **Target user:** PHP/Laravel developers on Linux. Comfortable with terminals but want a one-click GUI. Value speed, clarity, "it just works."
- **Brand feel:** Lightweight, fast, trustworthy, developer-grade. Laralux's identity is a friendly emerald/green. Keep it clean and uncluttered (the original is famously minimal and low-RAM). Modern but not flashy.
- **Platform:** Linux desktop window (not web, not mobile). Also lives in the system tray.

## 2. Technical constraints (BINDING — design must fit these)

- **Renderer:** Tauri 2 → **WebKitGTK** webview. Treat capabilities like Safari/WebKit: modern CSS is fine (flexbox, grid, custom properties, transitions, `prefers-color-scheme`, inline SVG). **Avoid:** `backdrop-filter` (unreliable), anything needing Chromium-only APIs.
- **No build tooling:** Frontend is **plain static HTML + CSS + vanilla JS** in a `dist/` folder. **No React/Vue/Tailwind/npm/bundler.** Deliverables must be implementable with hand-written HTML/CSS/JS. (If you provide a component system, express it as plain CSS classes + small JS, or CSS custom properties — not a framework.)
- **Self-contained / offline:** The app may run with **no internet**. **Do NOT use CDN fonts, icon fonts, or remote images.** Use the `system-ui` font stack, or bundle font files locally in `dist/`. Use **inline SVG** for all icons (provide the SVG markup).
- **Backend calls:** The UI talks to Rust via `window.__TAURI__.core.invoke("command_name", args)` returning JSON. See §5 for the exact contract. Design interactions around these calls (some are slow / prompt for a system password).
- **Window:** Single window, default **900×600**, resizable. Design a layout that works from ~**720×480 up to large/maximized**. Define a sensible **min-width/min-height** and how the layout reflows.
- **Performance/weight:** Keep CSS/JS small and snappy (Laralux's whole appeal is being lean). No heavy animation libraries.
- **Dark mode:** Support **light + dark** via `prefers-color-scheme` and/or an in-app toggle. Provide both palettes as design tokens.

## 3. What exists today (baseline to replace)

A bare, unstyled single page (`dist/index.html` + `main.js` + `styles.css`):
- A header: title "Laralux" + two buttons **Start All / Stop All**.
- Three stacked sections: **Setup** (list of components installed/missing + "Install missing" button), **Services** (a `<table>` of service name / state / Start|Stop button), **Sites** (a list of links `name — https://name.dev`).
- Status auto-refreshes every **2 seconds** via polling.
- Feedback is via `alert()` popups (ugly — replace with in-app toasts/inline UI).
- Colors are ad-hoc (green=running, grey=stopped, red=crashed). No spacing system, no icons, no empty/loading states, no dark mode.

**Goal of redesign:** turn this into a polished, modern control-center dashboard while keeping it lightweight and mapping 1:1 to the same data/actions.

## 4. Information architecture / screens

Design these screens/regions (single-window app; can be one dashboard with a sidebar, or tabs — propose the best):

1. **Dashboard / Home** (primary): at-a-glance stack status + global Start/Stop + per-service control + sites list. This is the screen the app opens to.
2. **Setup / First-run state**: when components are missing, surface a prominent "Set up your environment" flow (install missing). When everything is installed, this collapses into a small, unobtrusive status (or moves to Settings).
3. **Sites**: list of projects under `~/laralux/www`, each with its `https://<name>.dev` link, an "Open" action, and (future) "New site". Include the **empty state** ("No sites yet").
4. **Service detail (optional/secondary)**: clicking a service could reveal port(s), log path, and quick actions — design at least the hooks.
5. **Global chrome:** app header/identity, light/dark toggle, and a place for global actions (Start All / Stop All / Settings).

Also account for the **system tray menu** (native, not HTML, but keep parity): items `Start All`, `Stop All`, `Dashboard`, `Quit`.

## 5. EXACT data & action contract (design to this)

All via `invoke(name, args)` → returns JSON. Field names and enum string values are **exact** (serde variant names).

### Read (poll every ~2s, or on demand)
- `stack_status()` → `ServiceStatus[]`
- `list_sites()` → `Site[]`
- `setup_status()` → `ComponentStatus[]`

### Actions
- `stack_start_all()` → `ServiceStatus[]`  — NOTE: also runs site sync; may trigger a **system password prompt (pkexec)** the first time (writing `/etc/hosts`). Can take a few seconds.
- `stack_stop_all()` → `ServiceStatus[]`
- `service_start({ kind })` → `ServiceStatus[]`
- `service_stop({ kind })` → `ServiceStatus[]`
- `run_setup_cmd()` → `SetupReport`  — installs packages via apt; **slow (tens of seconds to minutes)** + **password prompt**. Needs progress/disabled UI.

### Types & exact string values
```
ServiceStatus = { kind: ServiceKind, state: ServiceState }
ServiceKind   = "Nginx" | "PhpFpm" | "Mariadb" | "Redis" | "Mailpit"
ServiceState  = "Stopped" | "Starting" | "Running" | "Stopping" | "Crashed"

Site = { name: string, root: string, hostname: string }   // hostname e.g. "blog.dev"

ComponentStatus = { component: Component, present: boolean }
Component       = "Nginx" | "Php" | "Mariadb" | "Redis" | "Mkcert" | "Mailpit"

SetupReport = {
  apt_packages: string[],     // packages it attempted to install
  mailpit_fetched: boolean,
  mkcert_ca: boolean,
  nginx_setcap: boolean,
  php_version: string | null, // e.g. "8.4" — if set, show "restart app to apply"
  errors: string[]            // human-readable error lines (may be empty)
}
```

**Display-name mapping (design should show friendly labels, not raw enum):**
- `Nginx` → "Nginx", `PhpFpm` → "PHP-FPM", `Mariadb` → "MariaDB", `Redis` → "Redis", `Mailpit` → "Mailpit", `Php` (setup) → "PHP".

**Known ports/endpoints to optionally surface per service:**
- Nginx: HTTP `80`, HTTPS `443`
- MariaDB: `3306`
- Redis: `6379`
- Mailpit: web UI `http://localhost:8025`, SMTP `1025`
- PHP-FPM: unix socket (no port)

## 6. Components to design (with ALL states)

For each, provide visual spec + every state:

1. **App header / title bar**: app name + logo mark, global actions (Start All, Stop All), dark-mode toggle, optional settings. States: idle, "starting all…" (in progress), "stopping…".
2. **Stack summary**: a compact indicator of overall health (e.g. "4/5 running"), maybe a single big Start/Stop primary button that toggles.
3. **Service row/card** (×5). States: **Stopped** (neutral), **Starting** (spinner/pulse), **Running** (green, with a dot + maybe port chips + quick links), **Stopping**, **Crashed** (red, with "view logs" affordance). Each has a Start/Stop toggle (primary action) and room for secondary actions (logs, restart). Show the service's port(s) as small chips.
4. **Setup panel / first-run card**: a checklist of the 6 components (installed ✓ / missing). A prominent **"Install missing"** primary button. **In-progress state** (disabled + "Installing… authorize when prompted" + progress feel; this can run minutes). **Result state** (success summary; or error list from `SetupReport.errors`). Special note row when `php_version` is set: "PHP 8.x installed — restart to apply."
5. **Site row/card**: project `name`, the clickable `https://<hostname>` link (opens browser), an "Open" button, copy-URL affordance, secondary menu (open folder/terminal/DB — future). **Empty state**: friendly "No sites yet — add a project to ~/laralux/www" with guidance, and a future "New site" CTA.
6. **Toasts / inline feedback** (replace `alert()`): success, error, info. Error toasts should be able to show a short message + "details" (the `errors[]` lines). Non-blocking.
7. **Password/long-op affordance**: when an action triggers `pkexec`, show a subtle "waiting for authorization…" state.
8. **Empty / loading / error states** for the whole dashboard (first paint, backend not responding).
9. **Dark + light variants** of everything.

## 7. Key interaction flows (design the happy path + states)

1. **First run (nothing installed):** open app → Setup is front-and-center → click "Install missing" → password prompt → long progress → success summary (maybe "restart to apply PHP"). After install, Setup collapses; dashboard becomes primary.
2. **Daily use:** open app → see services (mostly Stopped) → click **Start All** → (first time) password prompt for hosts → services flip Stopped→Starting→Running → sites become reachable. Click a site → opens `https://name.dev`.
3. **Per-service toggle:** click Start/Stop on a single service row; row reflects Starting→Running or Stopping→Stopped; on failure show an error toast.
4. **Crash:** a running service dies → row shows **Crashed** (red) with a "view logs" affordance.
5. **Stop all / quit:** Stop All flips everything to Stopped. (Closing the window keeps app in tray; tray "Quit" stops + exits.)

## 8. Visual direction (give 1–2 concrete options)

- **Brand color:** emerald/green primary (Laralux heritage). Suggested primary ~`#10b981`/`#0E9F6E` family; pick an accessible set. Provide full token ramp.
- **Layout proposal:** a clean **control center** — either (a) left sidebar nav (Dashboard / Sites / Setup / Settings) + content area, or (b) a single scrollable dashboard with clear section cards. Recommend one; the app is small so a single dashboard or a slim sidebar both fit.
- **Surfaces:** card-based, soft borders/shadows, generous spacing, a clear visual hierarchy (status is the hero). Status uses color + an icon/dot, never color alone (accessibility).
- **Typography:** `system-ui` stack (or bundle Inter). Define a type scale (e.g. 12/13/14/16/20/24) with weights.
- **Iconography:** inline SVG, simple line icons (provide SVGs for: each service, start ▶, stop ◼, restart ↻, running dot, warning, settings gear, external-link, folder, terminal, database, copy, moon/sun).
- **Motion:** subtle only — state transitions (fade/slide ~150–200ms), spinner for in-progress. No bounce/heavy effects.

## 9. Design tokens to deliver (so devs can wire CSS variables)

Provide a token set for **light AND dark**:
- Color: `--bg`, `--surface`, `--surface-2`, `--border`, `--text`, `--text-muted`, `--primary`, `--primary-hover`, `--success`, `--warning`, `--danger`, plus state colors for Running/Stopped/Starting/Crashed.
- Radius: e.g. `--radius-sm/md/lg`.
- Spacing scale: 4/8/12/16/24/32.
- Shadows: `--shadow-sm/md`.
- Type scale + weights.
Deliver as a table AND as ready-to-paste `:root { --token: value }` (+ `@media (prefers-color-scheme: dark)` overrides).

## 10. Accessibility & quality bar

- Status conveyed by **icon + text + color**, not color alone (color-blind safe). Check contrast (WCAG AA).
- Full **keyboard** operability: focus states, tab order, Enter/Space on buttons.
- Hit targets ≥ 32px. Don't rely on hover-only affordances.
- Respect reduced-motion (`prefers-reduced-motion`).

## 11. Future-proofing (leave room, don't fully design unless quick)

The roadmap (Laralux parity) will add: **New site** (create blank/Laravel/WordPress), **switch PHP/Node version**, **PHP quick-settings**, **open terminal/DB client/folder per site**, **logs viewer**, **PostgreSQL/Mongo**, **share via ngrok/cloudflared**, **settings** (TLD, paths, autostart). Design the IA so these slot in (e.g. a Settings area, a per-site "…" menu, a versions control) without a rework.

## 12. Deliverables expected from Claude Design

1. **High-fidelity mockups** (light + dark) for: Dashboard (services running + mixed states), First-run/Setup (idle, installing, result), Sites (with items + empty state), and key states (crashed service, error toast, in-progress Start All).
2. **Component specs** for every item in §6 with all states (redlines: spacing, sizes, colors via tokens).
3. **Design tokens** (§9) as a table + ready-to-paste CSS custom properties for light & dark.
4. **Inline SVG icon set** (§8) as raw `<svg>` markup.
5. **Responsive notes**: behavior at min size vs maximized; min-width/height recommendation.
6. **A short rationale** of layout choice (sidebar vs single dashboard) and how it stays lightweight.
7. (Nice to have) A **plain HTML/CSS prototype** of the dashboard using the tokens, since the real frontend is hand-written HTML/CSS/JS — this would map almost directly to `dist/`.

## 13. Hard constraints recap (do NOT violate)

- Plain HTML/CSS/vanilla-JS only; no framework, no build step, no npm.
- No remote assets (offline-safe): system/bundled fonts, inline SVG icons.
- WebKitGTK-compatible CSS (no `backdrop-filter` reliance).
- Map exactly to the §5 data/actions and enum string values.
- Keep it lean and fast (Laralux's core value). Replace `alert()` with in-app toasts.
- Support light + dark.

## 14. Reference inspiration

Laralux (Windows) dashboard for spirit; Laravel Herd, DBngin, TablePlus, Docker Desktop's container list for clean dev-tool control-center patterns. Aim: as calm and legible as Herd, as lean as the original Laralux.
