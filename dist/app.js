/* Laragon Linux — control-center frontend (vanilla, wired to Tauri IPC).
   Ported from the Claude Design handoff. No framework / build step. */

(() => {
  "use strict";

  const TAURI = window.__TAURI__;
  const invoke = (cmd, args) => {
    if (!TAURI || !TAURI.core) return Promise.reject(new Error("Tauri unavailable"));
    return TAURI.core.invoke(cmd, args);
  };

  // ---- inline SVG icons (copied verbatim from the design handoff) ----
  const I = {
    cube: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z"/><path d="M12 12l8-4.5M12 12v9M12 12L4 7.5"/></svg>',
    play: '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><path d="M7 5.5v13l11-6.5z"/></svg>',
    stop: '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>',
    sun: '<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M19.1 4.9l-1.4 1.4M6.3 17.7l-1.4 1.4"/></svg>',
    moon: '<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.8A8.5 8.5 0 1 1 11.2 3a6.6 6.6 0 0 0 9.8 9.8z"/></svg>',
    lock: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="4" y="10" width="16" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/></svg>',
    navDash: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><rect x="3" y="3" width="7.5" height="7.5" rx="1.5"/><rect x="13.5" y="3" width="7.5" height="7.5" rx="1.5"/><rect x="3" y="13.5" width="7.5" height="7.5" rx="1.5"/><rect x="13.5" y="13.5" width="7.5" height="7.5" rx="1.5"/></svg>',
    navSites: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.5 2.4 3.9 5.6 4 9-.1 3.4-1.5 6.6-4 9-2.5-2.4-3.9-5.6-4-9 .1-3.4 1.5-6.6 4-9z"/></svg>',
    navSetup: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12"/><path d="M8 11l4 4 4-4"/><path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2"/></svg>',
    navSettings: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-2.7 1.1V21a2 2 0 1 1-4 0v-.2A1.6 1.6 0 0 0 6.6 19l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1A1.6 1.6 0 0 0 3 13.6H3a2 2 0 1 1 0-4h.1A1.6 1.6 0 0 0 4.6 7l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1A1.6 1.6 0 0 0 10 4.6V4a2 2 0 1 1 4 0v.1a1.6 1.6 0 0 0 2.7 1.1l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8z"/></svg>',
    svcNginx: '<svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/></svg>',
    svcPhp: '<svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M9 7l-5 5 5 5M15 7l5 5-5 5"/></svg>',
    svcMaria: '<svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5.5" rx="7.5" ry="3"/><path d="M4.5 5.5v13c0 1.66 3.36 3 7.5 3s7.5-1.34 7.5-3v-13"/><path d="M4.5 12c0 1.66 3.36 3 7.5 3s7.5-1.34 7.5-3"/></svg>',
    svcRedis: '<svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l9 4.5L12 12 3 7.5 12 3z"/><path d="M3 12l9 4.5L21 12"/><path d="M3 16.5L12 21l9-4.5"/></svg>',
    svcMail: '<svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="M3.5 7.5l8.5 6 8.5-6"/></svg>',
    externalSm: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4h6v6M20 4l-9 9"/><path d="M19 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h4"/></svg>',
    external: '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 4h6v6M20 4l-9 9"/><path d="M19 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h4"/></svg>',
    warnSm: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l9.5 17H2.5z"/><path d="M12 9v5M12 17.5h.01"/></svg>',
    warn: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l9.5 17H2.5z"/><path d="M12 9v5M12 17.5h.01"/></svg>',
    folder: '<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h3.5l2 2H19a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
    folder18: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h3.5l2 2H19a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
    folderBig: '<svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h3.5l2 2H19a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
    copy: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>',
    kebab: '<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><circle cx="5" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="19" cy="12" r="1.6"/></svg>',
    plus: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>',
    setupItem: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M3 9h18"/></svg>',
    checkTag: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6L9 17l-5-5"/></svg>',
    download: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12M8 11l4 4 4-4"/><path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2"/></svg>',
    checkDone: '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6L9 17l-5-5"/></svg>',
    checkReport: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--running)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" style="flex:none"><path d="M20 6L9 17l-5-5"/></svg>',
    clock: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" style="flex:none"><path d="M12 8v5l3 2"/><circle cx="12" cy="12" r="9"/></svg>',
    tSuccess: '<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M8.5 12l2.5 2.5L16 9.5"/></svg>',
    tError: '<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l9.5 17H2.5z"/><path d="M12 9v5M12 17.5h.01"/></svg>',
    tInfo: '<svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/></svg>',
    close: '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18"/></svg>',
  };

  const SVC_KINDS = ["Nginx", "PhpFpm", "Mariadb", "Redis", "Mailpit"];
  const COMP_ORDER = ["Nginx", "Php", "Mariadb", "Redis", "Mkcert", "Mailpit"];
  const DISP = { Nginx: "Nginx", PhpFpm: "PHP-FPM", Mariadb: "MariaDB", Redis: "Redis", Mailpit: "Mailpit" };
  const DISP_COMP = { Nginx: "Nginx", Php: "PHP", Mariadb: "MariaDB", Redis: "Redis", Mkcert: "mkcert", Mailpit: "Mailpit" };
  const SVC_ICON = { Nginx: I.svcNginx, PhpFpm: I.svcPhp, Mariadb: I.svcMaria, Redis: I.svcRedis, Mailpit: I.svcMail };
  const PORTS = { Nginx: ["80", "443"], PhpFpm: ["socket"], Mariadb: ["3306"], Redis: ["6379"], Mailpit: ["8025", "1025"] };
  const LOG_FILE = { Nginx: "nginx-error.log", PhpFpm: "php-fpm.log", Mariadb: "mariadb.log", Redis: "redis.log", Mailpit: "mailpit.log" };

  const META = {
    Running: { label: "Running", cls: "running", busy: false, btn: "Stop", primary: false },
    Stopped: { label: "Stopped", cls: "stopped", busy: false, btn: "Start", primary: true },
    Starting: { label: "Starting…", cls: "starting", busy: true, btn: "Starting", primary: false },
    Stopping: { label: "Stopping…", cls: "starting", busy: true, btn: "Stopping", primary: false },
    Crashed: { label: "Crashed", cls: "crashed", busy: false, btn: "Restart", primary: true },
  };

  // ---- state ----
  const stored = localStorage.getItem("laragon-theme");
  const prefersDark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
  const state = {
    view: "dashboard",
    dark: stored ? stored === "dark" : !!prefersDark,
    compact: false,
    services: { Nginx: "Stopped", PhpFpm: "Stopped", Mariadb: "Stopped", Redis: "Stopped", Mailpit: "Stopped" },
    sites: [],
    setup: { phase: "idle", report: null, components: COMP_ORDER.map((c) => ({ component: c, present: false })) },
    pkexecMsg: null,
    startingAll: false,
    busy: false,
    toasts: [],
    tId: 1,
  };

  // ---- helpers ----
  const esc = (s) =>
    String(s == null ? "" : s).replace(/[&<>"']/g, (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])
    );
  const runningCount = () => SVC_KINDS.filter((k) => state.services[k] === "Running").length;
  const missingCount = () => state.setup.components.filter((c) => !c.present).length;

  function applyServices(arr) {
    if (!Array.isArray(arr)) return;
    for (const s of arr) if (s && s.kind in state.services) state.services[s.kind] = s.state;
  }
  function applyComponents(arr) {
    if (!Array.isArray(arr)) return;
    const byName = {};
    for (const c of arr) byName[c.component] = !!c.present;
    state.setup.components = COMP_ORDER.map((c) => ({ component: c, present: !!byName[c] }));
  }

  // ---- toasts ----
  function toast(t) {
    const id = state.tId++;
    state.toasts.push({ id, ...t });
    render();
    if (!t.sticky) setTimeout(() => dismiss(id), 4200);
  }
  function dismiss(id) {
    state.toasts = state.toasts.filter((x) => x.id !== id);
    render();
  }

  // ---- actions ----
  async function refresh() {
    if (state.busy) return;
    try {
      const [svc, sites, comps] = await Promise.all([
        invoke("stack_status"),
        invoke("list_sites"),
        invoke("setup_status"),
      ]);
      applyServices(svc);
      state.sites = Array.isArray(sites) ? sites : [];
      applyComponents(comps);
      render();
    } catch (e) {
      /* polling: stay quiet */
    }
  }

  async function toggleService(kind) {
    if (state.busy) return;
    const running = state.services[kind] === "Running";
    const cmd = running ? "service_stop" : "service_start";
    state.busy = true;
    state.services[kind] = running ? "Stopping" : "Starting";
    render();
    try {
      const arr = await invoke(cmd, { kind });
      applyServices(arr);
      if (!running) toast({ type: "success", title: DISP[kind] + " started" });
    } catch (e) {
      toast({ type: "error", title: DISP[kind] + (running ? " stop failed" : " start failed"), msg: String(e) });
    } finally {
      state.busy = false;
      render();
    }
  }

  async function startAll() {
    if (state.busy || runningCount() === 5) return;
    state.busy = true;
    state.startingAll = true;
    state.pkexecMsg = "Authorize to update /etc/hosts — enter your password in the system prompt.";
    render();
    try {
      const arr = await invoke("stack_start_all");
      applyServices(arr);
      toast({ type: "success", title: "All services running", msg: "Sites are reachable at https://*.dev" });
    } catch (e) {
      toast({ type: "error", title: "Start failed", msg: String(e) });
    } finally {
      state.busy = false;
      state.startingAll = false;
      state.pkexecMsg = null;
      render();
    }
  }

  async function stopAll() {
    if (state.busy || runningCount() === 0) return;
    state.busy = true;
    for (const k of SVC_KINDS) if (state.services[k] === "Running") state.services[k] = "Stopping";
    render();
    try {
      const arr = await invoke("stack_stop_all");
      applyServices(arr);
      toast({ type: "info", title: "All services stopped" });
    } catch (e) {
      toast({ type: "error", title: "Stop failed", msg: String(e) });
    } finally {
      state.busy = false;
      render();
    }
  }

  async function runSetup() {
    if (state.busy || state.setup.phase === "installing") return;
    state.busy = true;
    state.setup.phase = "installing";
    state.pkexecMsg = "Authorize package installation (apt) — enter your password in the system prompt.";
    render();
    try {
      const report = await invoke("run_setup_cmd");
      state.setup.report = report;
      state.setup.phase = "done";
      if (report && report.errors && report.errors.length)
        toast({ type: "error", sticky: true, title: "Setup finished with errors", details: report.errors });
      else toast({ type: "success", title: "Environment ready", msg: "All components installed" });
      try {
        applyComponents(await invoke("setup_status"));
      } catch (_) {}
    } catch (e) {
      state.setup.phase = "idle";
      toast({ type: "error", title: "Setup failed", msg: String(e) });
    } finally {
      state.busy = false;
      state.pkexecMsg = null;
      render();
    }
  }

  function viewLogs(kind) {
    const f = LOG_FILE[kind] || (kind.toLowerCase() + ".log");
    toast({
      type: "error",
      sticky: true,
      title: DISP[kind] + " crashed",
      details: ["Check ~/laragon/log/" + f, "or: journalctl --user -n 50"],
    });
  }

  async function copySite(name) {
    const url = "https://" + name + ".dev";
    try {
      await navigator.clipboard.writeText(url);
      toast({ type: "success", title: "Copied " + url });
    } catch (e) {
      toast({ type: "error", title: "Copy failed", msg: url });
    }
  }

  function setView(v) {
    state.view = v;
    render();
  }
  function toggleDark() {
    state.dark = !state.dark;
    localStorage.setItem("laragon-theme", state.dark ? "dark" : "light");
    render();
  }

  // ---- render pieces ----
  function spinner(klass) {
    return '<span class="spin spinner ' + klass + '"></span>';
  }

  function header() {
    const run = runningCount();
    const health = run === 5 ? "bgc-running" : run === 0 ? "bgc-stopped" : "bgc-starting";
    const allRunning = run === 5;
    const noneRunning = run === 0;
    const startBtn = state.startingAll
      ? '<button class="btn btn-primary btn-busy" disabled>' + spinner("on-primary") + "Starting…</button>"
      : '<button class="btn btn-primary' + (allRunning ? " btn-dim" : "") + '" data-action="start-all"' +
        (allRunning ? " disabled" : "") +
        ' title="' + (allRunning ? "All services already running" : "Start all services") + '">' + I.play + "Start All</button>";
    return (
      '<header class="header">' +
      '<div class="brand"><div class="brand-mark">' + I.cube + "</div>" +
      '<div class="brand-name">Laragon <span>Linux</span></div></div>' +
      '<span class="spacer"></span>' +
      '<div class="health-pill"><span class="dot ' + health + '"></span>' +
      '<span class="txt">' + run + "/5 running</span></div>" +
      startBtn +
      '<button class="btn btn-outline' + (noneRunning ? " btn-dim" : "") + '" data-action="stop-all"' +
      (noneRunning ? " disabled" : "") +
      ' title="' + (noneRunning ? "Nothing running" : "Stop all services") + '">' + I.stop + "Stop All</button>" +
      '<button class="icon-btn" data-action="toggle-dark" aria-label="Toggle theme">' + (state.dark ? I.sun : I.moon) + "</button>" +
      "</header>"
    );
  }

  function pkexecBanner() {
    if (!state.pkexecMsg) return "";
    return (
      '<div class="pkexec" role="status">' + spinner("warn") + I.lock +
      '<span class="msg">' + esc(state.pkexecMsg) + "</span></div>"
    );
  }

  function navItem(view, label, icon, opts = {}) {
    const active = state.view === view ? " active" : "";
    let badge = "";
    if (opts.dot) badge = '<span class="nav-dot"></span>';
    let trailing = "";
    if (opts.badge != null) trailing = '<span class="nav-badge label-only">' + opts.badge + "</span>";
    const labelSpan = opts.grow
      ? '<span class="grow label-only">' + label + "</span>"
      : '<span class="label-only">' + label + "</span>";
    return (
      '<button class="nav-item' + active + '" data-action="nav" data-view="' + view + '" title="' + label + '">' +
      '<span class="ico">' + icon + badge + "</span>" + labelSpan + trailing + "</button>"
    );
  }

  function sidebar() {
    const miss = missingCount();
    return (
      '<nav class="sidebar">' +
      navItem("dashboard", "Dashboard", I.navDash) +
      navItem("sites", "Sites", I.navSites) +
      navItem("setup", "Setup", I.navSetup, { dot: miss > 0, badge: miss > 0 ? miss : null, grow: true }) +
      navItem("settings", "Settings", I.navSettings) +
      '<span class="spacer"></span>' +
      '<div class="sidebar-footer label-only"><span class="dot"></span>Auto-refresh 2s</div>' +
      "</nav>"
    );
  }

  function svcButton(kind, m) {
    if (m.busy)
      return '<button class="btn-sm busy" disabled>' + spinner("muted") + esc(m.label) + "</button>";
    return (
      '<button class="btn-sm' + (m.primary ? " primary" : "") + '" data-action="svc-toggle" data-kind="' + kind + '">' +
      esc(m.btn) + "</button>"
    );
  }

  function serviceCard(kind) {
    const st = state.services[kind] || "Stopped";
    const m = META[st] || META.Stopped;
    const dotPulse = m.busy ? " pulse" : "";
    const ports = (PORTS[kind] || []).map((p) => '<span class="port-chip">' + esc(p) + "</span>").join("");
    let footRight = "";
    if (kind === "Mailpit" && st === "Running")
      footRight =
        '<a class="btn-xs" href="http://localhost:8025" target="_blank" rel="noreferrer">' + I.externalSm + "Open</a>";
    if (st === "Crashed")
      footRight = '<button class="btn-xs danger" data-action="svc-logs" data-kind="' + kind + '">' + I.warnSm + "View logs</button>";
    return (
      '<div class="card svc-card">' +
      '<div class="svc-top">' +
      '<div class="svc-tile">' + (SVC_ICON[kind] || "") + "</div>" +
      '<div class="svc-meta"><div class="svc-name">' + esc(DISP[kind]) + "</div>" +
      '<div class="svc-status"><span class="dot bgc-' + m.cls + dotPulse + '"></span>' +
      '<span class="txt s-' + m.cls + '">' + esc(m.label) + "</span></div></div>" +
      svcButton(kind, m) +
      "</div>" +
      '<div class="svc-foot">' + ports + '<span class="spacer"></span>' + footRight + "</div>" +
      "</div>"
    );
  }

  function dashboard() {
    const run = runningCount();
    const allRunning = run === 5;
    const noneRunning = run === 0;
    const dots = SVC_KINDS.map((k) => {
      const cls = (META[state.services[k]] || META.Stopped).cls;
      return '<span class="bgc-' + cls + '" title="' + esc(DISP[k] + ": " + state.services[k]) + '"></span>';
    }).join("");
    const startBtn = state.startingAll
      ? '<button class="btn h36 btn-primary btn-busy" disabled>' + spinner("on-primary") + "Starting…</button>"
      : '<button class="btn h36 btn-primary' + (allRunning ? " btn-dim" : "") + '" data-action="start-all"' +
        (allRunning ? " disabled" : "") + ">" + I.play + "Start All</button>";
    const cards = SVC_KINDS.map(serviceCard).join("");
    const preview = state.sites
      .slice(0, 3)
      .map((s) => {
        const url = "https://" + s.hostname;
        return (
          '<div class="card site-row preview"><div class="site-tile">' + I.folder + "</div>" +
          '<div class="site-info"><div class="site-name">' + esc(s.name) + "</div>" +
          '<a class="site-url" href="' + esc(url) + '" target="_blank" rel="noreferrer">' + esc(url) + "</a></div>" +
          '<a class="btn-sm" href="' + esc(url) + '" target="_blank" rel="noreferrer">Open</a></div>'
        );
      })
      .join("");
    return (
      '<div class="view">' +
      "<div><h1 class=\"h1\">Dashboard</h1>" +
      '<p class="subtitle">Local stack · pretty HTTPS at <code class="chip-code">*.dev</code></p></div>' +
      '<div class="card summary">' +
      '<div class="big"><span class="num">' + run + '</span><span class="den">/ 5</span></div>' +
      '<div style="min-width:0"><div class="lbl">services running</div><div class="dots">' + dots + "</div></div>" +
      '<span class="spacer"></span><div class="actions">' + startBtn +
      '<button class="btn h36 btn-outline' + (noneRunning ? " btn-dim" : "") + '" data-action="stop-all"' +
      (noneRunning ? " disabled" : "") + ">" + I.stop + "Stop All</button></div></div>" +
      '<div class="row-between"><h2 class="section-label">Services</h2></div>' +
      '<div class="svc-grid">' + cards + "</div>" +
      '<div class="row-between mt4"><h2 class="section-label">Sites</h2>' +
      '<button class="link-btn" data-action="nav" data-view="sites">View all →</button></div>' +
      '<div class="stack-col">' + preview + "</div>" +
      "</div>"
    );
  }

  function sitesView() {
    const empty = state.sites.length === 0;
    const head =
      '<div class="sites-head"><div><h1 class="h1">Sites</h1>' +
      '<p class="subtitle">Projects under <code class="chip-code">~/laragon/www</code></p></div>' +
      '<div class="sites-actions">' +
      '<button class="btn-newsite" disabled title="Coming soon">' + I.plus + "New site</button></div></div>";
    let bodyHtml;
    if (empty) {
      bodyHtml =
        '<div class="sites-empty"><div class="glyph">' + I.folderBig + "</div>" +
        '<div class="t">No sites yet</div>' +
        '<div class="h">Drop a project folder into <code class="chip-code">~/laragon/www</code> and it gets a pretty <code class="chip-code">https://&lt;name&gt;.dev</code> URL automatically.</div>' +
        '<button class="btn-newsite" disabled style="margin-top:4px">' + I.plus + "New site</button></div>";
    } else {
      bodyHtml =
        '<div class="stack-col g9">' +
        state.sites
          .map((s) => {
            const url = "https://" + s.hostname;
            return (
              '<div class="card site-row"><div class="site-tile">' + I.folder18 + "</div>" +
              '<div class="site-info"><div class="site-name">' + esc(s.name) + "</div>" +
              '<div class="site-sub"><a class="site-url" href="' + esc(url) + '" target="_blank" rel="noreferrer">' + esc(url) + "</a>" +
              '<span class="site-root">' + esc(s.root) + "</span></div></div>" +
              '<button class="icon-btn sq32" data-action="copy-site" data-name="' + esc(s.name) + '" aria-label="Copy URL">' + I.copy + "</button>" +
              '<button class="icon-btn sq32" disabled aria-label="More" title="Open folder · terminal · DB — coming soon" style="opacity:.55">' + I.kebab + "</button>" +
              '<a class="btn-sm" href="' + esc(url) + '" target="_blank" rel="noreferrer">' + I.external + "Open</a></div>"
            );
          })
          .join("") +
        "</div>";
    }
    return '<div class="view">' + head + bodyHtml + "</div>";
  }

  function setupView() {
    const miss = missingCount();
    const subtitle = state.setup.phase === "done" ? "All components installed." : miss + " of 6 components missing.";
    const items = state.setup.components
      .map((c) => {
        const tag = c.present
          ? '<span class="tag ok">' + I.checkTag + "Installed</span>"
          : '<span class="tag miss">' + I.warn + "Missing</span>";
        return (
          '<div class="setup-item"><div class="setup-tile">' + I.setupItem + "</div>" +
          '<span class="nm">' + esc(DISP_COMP[c.component] || c.component) + "</span>" + tag + "</div>"
        );
      })
      .join("");

    let action = "";
    if (state.setup.phase === "idle") {
      action =
        '<div class="setup-idle"><div class="info"><div class="t">' + miss + " component(s) missing</div>" +
        '<div class="h">Installs via <code>apt</code> — asks for your password and can take a few minutes.</div></div>' +
        '<button class="btn h36 btn-primary" data-action="run-setup" style="flex:none">' + I.download + "Install missing</button></div>";
    } else if (state.setup.phase === "installing") {
      action =
        '<div class="setup-installing">' + spinner("primary-lg") +
        '<div class="info"><div class="t">Installing… authorize when prompted</div>' +
        '<div class="h">Fetching packages — this can take a few minutes. Don\'t close the window.</div></div></div>' +
        '<div class="progress"><div class="shim bar"></div></div>';
    } else {
      const rep = state.setup.report || {};
      const rows = [
        ["apt packages", (rep.apt_packages ? rep.apt_packages.length : 0) + " installed"],
        ["Mailpit binary", rep.mailpit_fetched ? "fetched" : "skipped"],
        ["mkcert local CA", rep.mkcert_ca ? "trusted" : "skipped"],
        ["Nginx bind 80/443", rep.nginx_setcap ? "setcap ok" : "skipped"],
      ]
        .map(
          ([l, v]) =>
            '<div class="report-row">' + I.checkReport + '<span class="lbl">' + esc(l) + "</span>" +
            '<span class="spacer"></span><span class="val">' + esc(v) + "</span></div>"
        )
        .join("");
      const phpNotice = rep.php_version
        ? '<div class="notice-warn">' + I.clock + '<span class="t">PHP ' + esc(rep.php_version) + " installed — restart the app to apply.</span></div>"
        : "";
      action =
        '<div class="setup-done-head">' + I.checkDone + '<span class="t">Environment ready</span></div>' +
        '<div class="report-box">' + rows + "</div>" + phpNotice;
    }

    return (
      '<div class="view narrow">' +
      '<div><h1 class="h1">Setup</h1><p class="subtitle">' + esc(subtitle) + "</p></div>" +
      '<div class="card setup-card"><div class="setup-list">' + items + "</div>" +
      '<div class="setup-action">' + action + "</div></div></div>"
    );
  }

  function settingsView() {
    return (
      '<div class="view narrow-620">' +
      '<div><h1 class="h1">Settings</h1><p class="subtitle">Appearance and environment defaults.</p></div>' +
      '<div class="card settings-card">' +
      '<div class="set-row"><div class="grow"><div class="t">Appearance</div><div class="h">Light / dark theme</div></div>' +
      '<button class="btn-sm" data-action="toggle-dark">' + (state.dark ? "Dark" : "Light") + "</button></div>" +
      '<div class="set-row"><div class="grow"><div class="t">Local TLD</div><div class="h">Pretty-URL domain suffix</div></div>' +
      '<code class="code-chip">.dev</code></div>' +
      '<div class="set-row"><div class="grow"><div class="t">Sites directory</div><div class="h">Where projects are scanned</div></div>' +
      '<code class="code-chip">~/laragon/www</code></div>' +
      '<div class="set-row"><div class="grow"><div class="t">Start on login</div><div class="h">Autostart in system tray — coming soon</div></div>' +
      '<span class="toggle-off"><span class="knob"></span></span></div>' +
      "</div>" +
      '<div class="settings-foot">Laragon Linux · window 900×600 · min 720×480 · tray: Start All · Stop All · Dashboard · Quit</div>' +
      "</div>"
    );
  }

  function toasts() {
    if (!state.toasts.length) return '<div class="toasts"></div>';
    const items = state.toasts
      .map((t) => {
        const ico = t.type === "success" ? I.tSuccess : t.type === "error" ? I.tError : I.tInfo;
        const msg = t.msg ? '<div class="msg">' + esc(t.msg) + "</div>" : "";
        const details =
          t.details && t.details.length
            ? '<div class="details">' + t.details.map((d) => "<span>" + esc(d) + "</span>").join("") + "</div>"
            : "";
        return (
          '<div class="toast ' + t.type + '" role="status"><span class="ico">' + ico + "</span>" +
          '<div class="body"><div class="ttl">' + esc(t.title) + "</div>" + msg + details + "</div>" +
          '<button class="close" data-action="toast-dismiss" data-id="' + t.id + '" aria-label="Dismiss">' + I.close + "</button></div>"
        );
      })
      .join("");
    return '<div class="toasts">' + items + "</div>";
  }

  // ---- render ----
  const app = document.getElementById("app");
  let lastSig = "";

  function render() {
    document.documentElement.dataset.theme = state.dark ? "dark" : "light";
    let main;
    if (state.view === "dashboard") main = dashboard();
    else if (state.view === "sites") main = sitesView();
    else if (state.view === "setup") main = setupView();
    else main = settingsView();

    const html =
      '<div class="root" data-compact="' + state.compact + '">' +
      header() +
      pkexecBanner() +
      '<div class="body">' + sidebar() + '<main class="main">' + main + "</main></div>" +
      toasts() +
      "</div>";

    // Avoid needless DOM churn (preserves scroll/focus) when nothing changed.
    const sig = html;
    if (sig === lastSig) return;
    lastSig = sig;
    app.innerHTML = html;
  }

  // ---- events (delegated; bound once) ----
  app.addEventListener("click", (e) => {
    const el = e.target.closest("[data-action]");
    if (!el) return;
    const a = el.getAttribute("data-action");
    if (a === "nav") setView(el.getAttribute("data-view"));
    else if (a === "toggle-dark") toggleDark();
    else if (a === "start-all") startAll();
    else if (a === "stop-all") stopAll();
    else if (a === "run-setup") runSetup();
    else if (a === "svc-toggle") toggleService(el.getAttribute("data-kind"));
    else if (a === "svc-logs") viewLogs(el.getAttribute("data-kind"));
    else if (a === "copy-site") copySite(el.getAttribute("data-name"));
    else if (a === "toast-dismiss") dismiss(parseInt(el.getAttribute("data-id"), 10));
  });

  // ---- responsive (compact <820px) ----
  if (window.ResizeObserver) {
    const ro = new ResizeObserver((entries) => {
      const w = entries[0].contentRect.width;
      const c = w < 820;
      if (c !== state.compact) {
        state.compact = c;
        render();
      }
    });
    ro.observe(app);
  }

  // ---- boot ----
  render();
  refresh();
  setInterval(refresh, 2000);
})();
