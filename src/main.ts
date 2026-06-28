// @ts-nocheck  -- temporary: the monolith is typed module-by-module in later tasks; removed in the final task.
import "./styles.css";
import { state } from "./state";
import { I } from "./ui/icons";
import { esc, validName } from "./ui/util";
import { toast, dismiss, toasts } from "./ui/toast";
import { setLoop } from "./ui/loop";
import { dashboard, startAll, stopAll, toggleService, viewLogs } from "./ui/views/dashboard";
import { sitesView, openNewSite, closeNewSite, submitNewSite, openLinkSite, closeLinkSite, browseFolder, submitLinkSite, openProxy, closeProxy, addProxyRoute, delProxyRoute, submitProxy, openDomains, closeDomains, addDomainRow, delDomainRow, submitDomains, removeSite, copySite, openTerminal, openExternal } from "./ui/views/sites";
import { setupView, runSetup } from "./ui/views/setup";
import { settingsView, toggleDark } from "./ui/views/settings";
/* Laralux — control-center frontend (vanilla, wired to Tauri IPC).
   Ported from the Claude Design handoff. No framework / build step. */

(() => {
  "use strict";

  const TAURI = window.__TAURI__;
  const invoke = (cmd, args) => {
    if (!TAURI || !TAURI.core) return Promise.reject(new Error("Tauri unavailable"));
    return TAURI.core.invoke(cmd, args);
  };

  const SVC_KINDS = ["Nginx", "PhpFpm", "Mariadb", "Redis", "Mailpit"];
  const COMP_ORDER = ["Nginx", "Php", "Mariadb", "Redis", "Mkcert", "Mailpit", "Composer", "Node"];
  const DISP_COMP = { Nginx: "Nginx", Php: "PHP", Mariadb: "MariaDB", Redis: "Redis", Mkcert: "mkcert", Mailpit: "Mailpit", Composer: "Composer", Node: "Node.js" };
  const TOOL_KEY = { Nginx: "nginx", Php: "php", Mariadb: "mariadb", Redis: "redis", Mkcert: "mkcert", Mailpit: "mailpit", Composer: "composer", Node: "node" };
  const TOOL_CLI = { nginx: "nginx", php: "php", mariadb: "mariadb", redis: "redis-cli", mkcert: "mkcert", mailpit: null, composer: "composer", node: "node, npm, npx" };

  const META = {
    Running: { label: "Running", cls: "running", busy: false, btn: "Stop", primary: false },
    Stopped: { label: "Stopped", cls: "stopped", busy: false, btn: "Start", primary: true },
    Starting: { label: "Starting…", cls: "starting", busy: true, btn: "Starting", primary: false },
    Stopping: { label: "Stopping…", cls: "starting", busy: true, btn: "Stopping", primary: false },
    Crashed: { label: "Crashed", cls: "crashed", busy: false, btn: "Restart", primary: true },
  };

  // ---- helpers ----
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

  // ---- download progress helpers ----
  function resetDownload() {
    state.download = { active: false, label: "", step: { done: 0, total: 0 }, bytes: { current: 0, total: 0 }, overall: 0 };
  }
  // Combine step (files done / total) + the current file's byte fraction into a
  // single OVERALL fraction (0..1) for the whole operation, clamped monotonic so
  // it only ever fills forward — never resets per file (which would mislead).
  function computeOverall() {
    const d = state.download;
    const byteFrac = d.bytes.total > 0 ? Math.min(1, d.bytes.current / d.bytes.total) : 0;
    const raw = d.step.total > 0 ? (d.step.done + byteFrac) / d.step.total : byteFrac;
    d.overall = Math.max(d.overall, Math.min(1, raw));
  }
  function applyProgress(p) {
    if (!p || !p.kind) return;
    state.download.active = true;
    if (p.kind === "phase") state.download.label = String(p.label || "");
    else if (p.kind === "step") { state.download.step = { done: p.done | 0, total: p.total | 0 }; if (p.label) state.download.label = String(p.label); }
    else if (p.kind === "bytes") state.download.bytes = { current: Number(p.current) || 0, total: Number(p.total) || 0 };
    computeOverall();
  }

  // ---- actions (tool modal — shared, stays in main.ts until Task 6) ----
  async function openTool(toolKey) {
    const comp = Object.keys(TOOL_KEY).find((k) => TOOL_KEY[k] === toolKey);
    state.modal = {
      open: true, toolKey, display: DISP_COMP[comp] || toolKey, cliBinary: TOOL_CLI[toolKey],
      versions: [], linked: false, busy: false, busyVersion: null,
    };
    render();
    try {
      const [versions, linked] = await Promise.all([
        invoke("tool_versions", { tool: toolKey }),
        invoke("tool_symlinks"),
      ]);
      state.modal.versions = versions;
      state.toolSymlinks = linked;
      state.modal.linked = linked.includes(toolKey);
    } catch (e) {
      toast({ type: "error", title: "Load failed", msg: String(e) });
    }
    if (toolKey === "php") {
      try { state.modal.phpIni = await invoke("php_ini_settings"); }
      catch (e) { state.modal.phpIni = null; }
    }
    render();
  }

  function closeTool() { if (state.modal && state.modal.busy) return; state.modal = null; render(); }

  async function useToolVersion(version) {
    const tk = state.modal.toolKey;
    state.modal.busy = true; state.modal.busyVersion = version; render();
    try {
      await invoke("set_tool_version", { tool: tk, version });
      state.modal.versions = await invoke("tool_versions", { tool: tk });
      toast({ type: "success", title: "Version switched", msg: state.modal.display + " " + version });
    } catch (e) {
      toast({ type: "error", title: "Switch failed", msg: String(e) });
    } finally {
      if (state.modal) { state.modal.busy = false; state.modal.busyVersion = null; } resetDownload(); render();
    }
  }

  async function installToolVersion(version) {
    const tk = state.modal.toolKey;
    state.modal.busy = true; state.modal.busyVersion = version; render();
    try {
      state.modal.versions = await invoke("install_tool_version", { tool: tk, version });
      toast({ type: "success", title: "Installed", msg: state.modal.display + " " + version });
    } catch (e) {
      toast({ type: "error", title: "Install failed", msg: String(e) });
    } finally {
      if (state.modal) { state.modal.busy = false; state.modal.busyVersion = null; } resetDownload(); render();
    }
  }

  async function toggleToolSymlink() {
    const tk = state.modal.toolKey;
    const next = !state.modal.linked;
    state.modal.busy = true; render();
    try {
      state.toolSymlinks = await invoke("set_tool_symlink", { tool: tk, enabled: next });
      state.modal.linked = state.toolSymlinks.includes(tk);
      toast({ type: "success", title: next ? "Linked" : "Unlinked",
              msg: String(state.modal.cliBinary).split(", ").map((b) => "/usr/local/bin/" + b).join(", ") });
    } catch (e) {
      toast({ type: "error", title: "Symlink failed", msg: String(e) });
    } finally {
      if (state.modal) { state.modal.busy = false; } render();
    }
  }

  async function applyPhpIni() {
    if (!state.modal || !state.modal.phpIni) return;
    const payload = { ...state.modal.phpIni };
    payload.max_execution_time = parseInt(payload.max_execution_time, 10) || 0;
    state.modal.busy = true; render();
    try {
      state.modal.phpIni = await invoke("set_php_ini_settings", { settings: payload });
      toast({ type: "success", title: "PHP settings applied", msg: "Restarted php-fpm; CLI uses them too." });
    } catch (e) {
      toast({ type: "error", title: "Couldn't apply PHP settings", msg: String(e) });
    } finally {
      if (state.modal) state.modal.busy = false; render();
    }
  }

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
      // While a modal is open, only refresh state — don't rebuild the DOM, or
      // the user gets kicked out of the input they're typing into. The modal's
      // own actions (and closing it) call render() with the fresh state.
      if (!state.modal) render();
    } catch (e) {
      /* polling: stay quiet */
    }
  }

  function setView(v) {
    state.view = v;
    state.confirmRemove = null;
    render();
  }

  // ---- render pieces ----
  function spinner(klass) {
    return '<span class="spin spinner ' + klass + '"></span>';
  }

  // Small inline ring that fills to the OVERALL fraction (no % number). Stable
  // DOM (determinate + spin circle both present, toggled via `.ring-hide`) so
  // updateRing() can mutate it in place without a full re-render. Spins
  // indeterminately until the first progress (overall === 0), then fills.
  function progressRing() {
    const d = state.download;
    const R = 9, C = 2 * Math.PI * R;
    const has = d.overall > 0;
    const off = C * (1 - Math.min(1, d.overall));
    return (
      '<span class="ring-sm" role="status" aria-label="Downloading">' +
      '<svg width="22" height="22" viewBox="0 0 22 22">' +
      '<circle class="ring-bg" cx="11" cy="11" r="' + R + '"/>' +
      '<circle class="ring-fg' + (has ? '' : ' ring-hide') + '" cx="11" cy="11" r="' + R + '" stroke-dasharray="' + C + '" stroke-dashoffset="' + off + '"/>' +
      '<circle class="ring-spin spin' + (has ? ' ring-hide' : '') + '" cx="11" cy="11" r="' + R + '" stroke-dasharray="' + (C * 0.25) + ' ' + C + '"/>' +
      '</svg></span>'
    );
  }

  // Update the on-screen ring in place from state.download.overall — no innerHTML
  // churn, so scroll/focus are preserved during rapid progress ticks. Falls back
  // to a full render() only when the ring isn't mounted yet.
  function updateRing() {
    const ring = document.querySelector(".ring-sm");
    if (!ring) return; // ring is mounted by the handler before the op starts; nothing to update otherwise
    const d = state.download;
    const R = 9, C = 2 * Math.PI * R;
    const has = d.overall > 0;
    const fg = ring.querySelector(".ring-fg");
    if (fg) { fg.setAttribute("stroke-dashoffset", C * (1 - Math.min(1, d.overall))); fg.classList.toggle("ring-hide", !has); }
    const spin = ring.querySelector(".ring-spin");
    if (spin) spin.classList.toggle("ring-hide", has);
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
      '<div class="brand-name">Laralux</div></div>' +
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
      '<div class="pkexec" role="status">' + I.lock +
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
      '<div class="sidebar-footer label-only"><span class="dot"></span>Live</div>' +
      "</nav>"
    );
  }

  function phpIniField(label, key, val) {
    return '<div class="set-row"><div class="grow"><div class="t">' + esc(label) + "</div></div>" +
      '<input class="ns-input" data-action="php-ini-input" data-key="' + key + '" value="' + esc(String(val)) + '" /></div>';
  }
  function phpIniToggle(label, key, on) {
    return '<div class="set-row"><div class="grow"><div class="t">' + esc(label) + "</div></div>" +
      '<button class="btn-sm" data-action="php-ini-toggle" data-key="' + key + '">' + (on ? "On" : "Off") + "</button></div>";
  }

  function toolModal() {
    const m = state.modal;
    if (!m || !m.open) return "";
    const verRows = (m.versions || [])
      .map((v) => {
        let right;
        if (m.busy && m.busyVersion === v.version) right = progressRing();
        else if (v.active) right = '<span class="tag ok">Active</span>';
        else if (v.installed) right = '<button class="btn-sm" data-action="use-tool-version" data-version="' + esc(v.version) + '"' + (m.busy ? " disabled" : "") + ">Use</button>";
        else right = '<button class="btn-sm" data-action="install-tool-version" data-version="' + esc(v.version) + '"' + (m.busy ? " disabled" : "") + ">Install</button>";
        return '<div class="set-row"><div class="grow"><div class="t">' + esc(m.display) + " " + esc(v.version) + '</div><div class="h">' + (v.installed ? "Installed" : "Not installed") + "</div></div>" + right + "</div>";
      })
      .join("") || '<div class="set-row"><div class="h">No versions — run "Install missing" first.</div></div>';

    const anyInstalled = (m.versions || []).some((v) => v.installed);
    const symlinkRow = m.cliBinary
      ? '<div class="modal-divider"></div>' +
        '<div class="set-row"><div class="grow"><div class="t">In terminal (/usr/local/bin)</div>' +
        '<div class="h"><code>' + esc(m.cliBinary) + "</code> available system-wide</div></div>" +
        '<button class="btn-sm" data-action="toggle-tool-symlink"' + (m.busy || !anyInstalled ? " disabled" : "") + ">" +
        (m.linked ? "On" : "Off") + "</button></div>"
      : "";

    const pi = m.phpIni;
    const phpSettings = (m.toolKey === "php" && pi)
      ? '<div class="modal-divider"></div>' +
        '<div class="modal-sec-label">Settings</div>' +
        phpIniField("memory_limit", "memory_limit", pi.memory_limit) +
        phpIniField("upload_max_filesize", "upload_max_filesize", pi.upload_max_filesize) +
        phpIniField("post_max_size", "post_max_size", pi.post_max_size) +
        phpIniField("max_execution_time", "max_execution_time", pi.max_execution_time) +
        phpIniField("date.timezone", "timezone", pi.timezone) +
        phpIniToggle("display_errors", "display_errors", pi.display_errors) +
        phpIniToggle("opcache.enable", "opcache_enable", pi.opcache_enable) +
        '<div class="auth-note">' + (I.lock || "") +
        '<span class="auth-tx">First Apply asks for your password once to enable the CLI (php) — the web stack applies immediately.</span></div>' +
        '<div class="set-row"><div class="grow"></div>' +
        '<button class="btn-sm" data-action="apply-php-ini"' + (m.busy ? " disabled" : "") + ">Apply</button></div>"
      : "";

    return (
      '<div class="modal-backdrop" data-action="close-tool"></div>' +
      '<div class="modal" role="dialog" aria-modal="true">' +
      '<div class="modal-head"><span class="modal-title">' + esc(m.display) + "</span>" +
      '<button class="modal-close" data-action="close-tool" aria-label="Close">' + I.close + "</button></div>" +
      '<div class="modal-body"><div class="modal-sec-label">Versions</div>' + verRows + symlinkRow + phpSettings + "</div>" +
      "</div>"
    );
  }

  function newSiteModal() {
    const ns = state.newSite;
    const ok = validName(ns.name);
    const preview = ns.name ? '<span class="ns-preview">→ https://' + esc(ns.name) + '.dev</span>' : '<span class="ns-preview muted">→ https://&lt;name&gt;.dev</span>';
    const errorHtml = ns.error ? '<div class="ns-error">' + esc(ns.error) + '</div>' : '';
    const disabledAttr = ns.busy ? ' disabled' : '';
    const templateOpts = ["Blank", "Laravel", "Wordpress"].map((t) =>
      '<option value="' + t + '"' + (ns.template === t ? ' selected' : '') + '>' + t + '</option>'
    ).join('');
    const createLabel = ns.busy
      ? '<span class="spin spinner on-primary"></span>Creating… (this can take a minute)'
      : 'Create';
    return (
      '<div class="ns-overlay" data-action="ns-overlay-click" role="dialog" aria-modal="true" aria-labelledby="ns-title">' +
      '<div class="ns-card" role="document">' +
      '<div class="ns-head"><h2 class="ns-title" id="ns-title">New site</h2>' +
      '<button class="icon-btn" data-action="ns-close" aria-label="Close"' + disabledAttr + '>' + I.close + '</button></div>' +
      '<div class="ns-body">' +
      '<label class="ns-label" for="ns-name">Site name</label>' +
      '<input class="ns-input" type="text" id="ns-name" name="ns-name" placeholder="my-app"' +
      ' value="' + esc(ns.name) + '" autocomplete="off" spellcheck="false" maxlength="63"' +
      disabledAttr + ' data-action="ns-name-input" />' +
      preview +
      '<label class="ns-label" for="ns-template">Template</label>' +
      '<select class="ns-select" id="ns-template" name="ns-template"' + disabledAttr + ' data-action="ns-template-change">' +
      templateOpts + '</select>' +
      errorHtml +
      '</div>' +
      '<div class="ns-foot">' +
      '<button class="btn btn-outline" data-action="ns-close"' + disabledAttr + '>Cancel</button>' +
      '<button class="btn btn-primary' + (!ok || ns.busy ? ' btn-dim' : '') + '" data-action="ns-submit"' +
      (!ok || ns.busy ? ' disabled' : '') + '>' + createLabel + '</button>' +
      '</div></div></div>'
    );
  }

  function linkSiteModal() {
    const ls = state.linkSite;
    const ok = ls.root && validName(ls.name);
    const preview = ls.name ? '<span class="ns-preview">→ https://' + esc(ls.name) + '.dev</span>' : '<span class="ns-preview muted">→ https://&lt;name&gt;.dev</span>';
    const errorHtml = ls.error ? '<div class="ns-error">' + esc(ls.error) + '</div>' : '';
    const d = ls.busy ? ' disabled' : '';
    const submitLabel = ls.busy ? '<span class="spin spinner on-primary"></span>Linking…' : 'Add site';
    return (
      '<div class="ns-overlay" data-action="ls-overlay-click" role="dialog" aria-modal="true" aria-labelledby="ls-title">' +
      '<div class="ns-card" role="document">' +
      '<div class="ns-head"><h2 class="ns-title" id="ls-title">Add existing folder</h2>' +
      '<button class="icon-btn" data-action="ls-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
      '<div class="ns-body">' +
      '<label class="ns-label" for="ls-root">Folder</label>' +
      '<div class="ls-row">' +
      '<input class="ns-input grow" type="text" id="ls-root" placeholder="/home/me/projects/my-app"' +
      ' value="' + esc(ls.root) + '" autocomplete="off" spellcheck="false"' + d + ' data-action="ls-root-input" />' +
      '<button class="btn btn-outline" data-action="ls-browse"' + d + '>Browse…</button>' +
      '</div>' +
      '<label class="ns-label" for="ls-name">Site name</label>' +
      '<input class="ns-input" type="text" id="ls-name" placeholder="my-app"' +
      ' value="' + esc(ls.name) + '" autocomplete="off" spellcheck="false" maxlength="63"' + d + ' data-action="ls-name-input" />' +
      preview + errorHtml +
      '</div>' +
      '<div class="ns-foot">' +
      '<button class="btn btn-outline" data-action="ls-close"' + d + '>Cancel</button>' +
      '<button class="btn btn-primary' + (!ok || ls.busy ? ' btn-dim' : '') + '" data-action="ls-submit"' +
      (!ok || ls.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
      '</div></div></div>'
    );
  }

  function proxyModal() {
    const p = state.proxy;
    const ok = validName(p.name) && p.routes.length > 0;
    const isEdit = p.mode === "edit";
    const preview = p.name ? '<span class="ns-preview">→ https://' + esc(p.name) + '.dev</span>' : '<span class="ns-preview muted">→ https://&lt;name&gt;.dev</span>';
    const errorHtml = p.error ? '<div class="ns-error">' + esc(p.error) + '</div>' : '';
    const d = p.busy ? ' disabled' : '';
    const rows = p.routes.map((r, i) =>
      '<div class="pr-row">' +
      '<input class="ns-input pr-path" type="text" placeholder="/" value="' + esc(r.path) + '" autocomplete="off" spellcheck="false" data-action="pr-path" data-idx="' + i + '"' + d + ' />' +
      '<input class="ns-input pr-up" type="text" placeholder="3000 or 127.0.0.1:5173" value="' + esc(r.upstream) + '" autocomplete="off" spellcheck="false" data-action="pr-upstream" data-idx="' + i + '"' + d + ' />' +
      (p.routes.length > 1 ? '<button class="icon-btn sq32" data-action="pr-del" data-idx="' + i + '" aria-label="Remove route"' + d + '>' + I.close + '</button>' : '') +
      '</div>'
    ).join('');
    const submitLabel = p.busy
      ? '<span class="spin spinner on-primary"></span>' + (isEdit ? 'Saving…' : 'Creating…')
      : (isEdit ? 'Save' : 'Create proxy');
    return (
      '<div class="ns-overlay" data-action="px-overlay-click" role="dialog" aria-modal="true" aria-labelledby="px-title">' +
      '<div class="ns-card" role="document">' +
      '<div class="ns-head"><h2 class="ns-title" id="px-title">' + (isEdit ? 'Edit reverse proxy' : 'Reverse proxy') + '</h2>' +
      '<button class="icon-btn" data-action="px-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
      '<div class="ns-body">' +
      '<label class="ns-label" for="px-name">Site name</label>' +
      '<input class="ns-input" type="text" id="px-name" placeholder="my-app" value="' + esc(p.name) + '" autocomplete="off" spellcheck="false" maxlength="63"' + (isEdit ? ' readonly' : '') + d + ' data-action="px-name-input" />' +
      preview +
      '<label class="ns-label">Routes</label>' +
      rows +
      '<button class="link-btn" data-action="pr-add"' + d + '>+ Add route</button>' +
      '<label class="ns-check"><input type="checkbox" data-action="px-ws"' + (p.websocket ? ' checked' : '') + d + ' /> WebSocket support</label>' +
      errorHtml +
      '</div>' +
      '<div class="ns-foot">' +
      '<button class="btn btn-outline" data-action="px-close"' + d + '>Cancel</button>' +
      '<button class="btn btn-primary' + (!ok || p.busy ? ' btn-dim' : '') + '" data-action="px-submit"' + (!ok || p.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
      '</div></div></div>'
    );
  }

  function domainsModal() {
    const sd = state.siteDomains;
    const hasAny = sd.domains.some((d) => d.trim().length > 0);
    const errorHtml = sd.error ? '<div class="ns-error">' + esc(sd.error) + '</div>' : '';
    const d = sd.busy ? ' disabled' : '';
    const rows = sd.domains.map((v, i) =>
      '<div class="pr-row">' +
      '<input class="ns-input" type="text" placeholder="app.example.com or *.example.com" value="' + esc(v) + '" autocomplete="off" spellcheck="false" data-action="dm-input" data-idx="' + i + '"' + d + ' />' +
      (sd.domains.length > 1 ? '<button class="icon-btn sq32" data-action="dm-del" data-idx="' + i + '" aria-label="Remove domain"' + d + '>' + I.close + '</button>' : '') +
      '</div>'
    ).join('');
    const submitLabel = sd.busy
      ? '<span class="spin spinner on-primary"></span>Saving…'
      : 'Save';
    return (
      '<div class="ns-overlay" data-action="dm-overlay-click" role="dialog" aria-modal="true" aria-labelledby="dm-title">' +
      '<div class="ns-card" role="document">' +
      '<div class="ns-head"><h2 class="ns-title" id="dm-title">Edit domains — ' + esc(sd.name) + '</h2>' +
      '<button class="icon-btn" data-action="dm-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
      '<div class="ns-body">' +
      '<label class="ns-label">Domains</label>' +
      rows +
      '<button class="link-btn" data-action="dm-add"' + d + '>+ Add domain</button>' +
      errorHtml +
      '</div>' +
      '<div class="ns-foot">' +
      '<button class="btn btn-outline" data-action="dm-close"' + d + '>Cancel</button>' +
      '<button class="btn btn-primary' + (!hasAny || sd.busy ? ' btn-dim' : '') + '" data-action="dm-submit"' + (!hasAny || sd.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
      '</div></div></div>'
    );
  }

  // ---- render ----
  const app = document.getElementById("app");
  let lastSig = "";
  let lastView = null;

  function render() {
    document.documentElement.dataset.theme = state.dark ? "dark" : "light";
    let main;
    if (state.view === "dashboard") main = dashboard();
    else if (state.view === "sites") main = sitesView();
    else if (state.view === "setup") main = setupView();
    else main = settingsView();

    const modalHtml = state.modal === "newsite" ? newSiteModal()
      : state.modal === "linksite" ? linkSiteModal()
      : state.modal === "proxy" ? proxyModal()
      : state.modal === "domains" ? domainsModal()
      : "";
    const html =
      '<div class="root" data-compact="' + state.compact + '">' +
      header() +
      '<div class="body">' + sidebar() + '<main class="main">' + pkexecBanner() + main + "</main></div>" +
      toasts() +
      modalHtml +
      toolModal() +
      "</div>";

    // Avoid needless DOM churn (preserves scroll/focus) when nothing changed.
    const sig = html;
    if (sig === lastSig) return;
    lastSig = sig;

    // Preserve focus + caret across the full innerHTML replacement, so an
    // event-driven background render can't kick the user out of an
    // input they are typing into. Identify the focused field by id, or by
    // data-action(+data-idx) for the modal route fields (which have no id).
    const ae = document.activeElement;
    let fId = null, fAction = null, fIdx = null, selS = null, selE = null;
    if (ae && app.contains(ae) && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA")) {
      fId = ae.id || null;
      fAction = ae.getAttribute("data-action");
      fIdx = ae.getAttribute("data-idx");
      try { selS = ae.selectionStart; selE = ae.selectionEnd; } catch (_) {}
    }

    // Preserve the scroll position of the main content area across the full
    // innerHTML replacement (a background services/sites event, or a
    // download-progress tick, otherwise yanks the user back to the top). Only restore when the view is
    // unchanged — a deliberate navigation should start at the top.
    const scroller = app.querySelector(".main");
    const prevScroll = scroller ? scroller.scrollTop : 0;
    const sameView = state.view === lastView;
    lastView = state.view;

    // The tool modal (.modal) is its own scroll area (overflow:auto, max-height:82vh);
    // an in-modal re-render (version use/install, symlink or settings toggle, Apply)
    // otherwise yanks it back to the top. Restore it whenever the modal stays open.
    const modalEl = app.querySelector(".modal");
    const prevModalScroll = modalEl ? modalEl.scrollTop : 0;

    app.innerHTML = html;

    if (sameView) { const ns = app.querySelector(".main"); if (ns) ns.scrollTop = prevScroll; }
    { const nm = app.querySelector(".modal"); if (nm) nm.scrollTop = prevModalScroll; }

    if (fId || fAction) {
      let el = fId ? document.getElementById(fId) : null;
      if (!el && fAction) {
        let sel = '[data-action="' + fAction + '"]';
        if (fIdx != null) sel += '[data-idx="' + fIdx + '"]';
        el = app.querySelector(sel);
      }
      if (el) {
        el.focus();
        if (selS != null) { try { el.setSelectionRange(selS, selE); } catch (_) {} }
      }
    }
  }

  // Register render + refresh into the loop bridge so extracted view modules can call them.
  setLoop(render, refresh);

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
    else if (a === "open-terminal") openTerminal(el.getAttribute("data-path"));
    else if (a === "open-url") { e.preventDefault(); openExternal(el.getAttribute("data-url")); }
    else if (a === "open-tool") openTool(el.dataset.tool);
    else if (a === "close-tool") closeTool();
    else if (a === "use-tool-version") useToolVersion(el.dataset.version);
    else if (a === "install-tool-version") installToolVersion(el.dataset.version);
    else if (a === "toggle-tool-symlink") toggleToolSymlink();
    else if (a === "php-ini-toggle") {
      if (state.modal && state.modal.phpIni) {
        state.modal.phpIni[el.dataset.key] = !state.modal.phpIni[el.dataset.key];
        render();
      }
    }
    else if (a === "apply-php-ini") applyPhpIni();
    else if (a === "toast-dismiss") dismiss(parseInt(el.getAttribute("data-id"), 10));
    else if (a === "new-site") openNewSite();
    else if (a === "ns-close") closeNewSite();
    else if (a === "ns-submit") submitNewSite();
    else if (a === "ns-overlay-click") {
      // close only if click is directly on the overlay (not the card inside it)
      if (e.target === el) closeNewSite();
    }
    else if (a === "link-site") openLinkSite();
    else if (a === "remove-site") removeSite(el.getAttribute("data-name"));
    else if (a === "ls-close") closeLinkSite();
    else if (a === "ls-submit") submitLinkSite();
    else if (a === "ls-browse") browseFolder();
    else if (a === "ls-overlay-click") { if (e.target === el) closeLinkSite(); }
    else if (a === "proxy-site") openProxy();
    else if (a === "edit-proxy") openProxy(state.sites.find((s) => s.name === el.getAttribute("data-name")));
    else if (a === "px-close") closeProxy();
    else if (a === "px-submit") submitProxy();
    else if (a === "pr-add") addProxyRoute();
    else if (a === "pr-del") delProxyRoute(parseInt(el.getAttribute("data-idx"), 10));
    else if (a === "px-overlay-click") { if (e.target === el) closeProxy(); }
    else if (a === "edit-domains") openDomains(state.sites.find((s) => s.name === el.getAttribute("data-name")));
    else if (a === "dm-close") closeDomains();
    else if (a === "dm-submit") submitDomains();
    else if (a === "dm-add") addDomainRow();
    else if (a === "dm-del") delDomainRow(parseInt(el.getAttribute("data-idx"), 10));
    else if (a === "dm-overlay-click") { if (e.target === el) closeDomains(); }
  });

  // ---- modal input events (delegated on app) ----
  app.addEventListener("input", (e) => {
    const el = e.target;
    if (el.dataset.action === "ns-name-input") {
      state.newSite.name = el.value;
      state.newSite.error = "";
      // Re-render preview + button state without full DOM replace (avoid losing focus)
      const preview = document.querySelector(".ns-preview");
      if (preview) {
        if (el.value) {
          preview.classList.remove("muted");
          preview.textContent = "→ https://" + el.value + ".dev";
        } else {
          preview.classList.add("muted");
          preview.innerHTML = "→ https://&lt;name&gt;.dev";
        }
      }
      const submitBtn = document.querySelector('[data-action="ns-submit"]');
      if (submitBtn) {
        const ok = validName(el.value);
        submitBtn.disabled = !ok;
        submitBtn.classList.toggle("btn-dim", !ok);
      }
      const errEl = document.querySelector(".ns-error");
      if (errEl) errEl.remove();
    }
    if (el.dataset.action === "ls-root-input") {
      state.linkSite.root = el.value;
      state.linkSite.error = "";
      const submitBtn = document.querySelector('[data-action="ls-submit"]');
      if (submitBtn) { const ok = el.value && validName(state.linkSite.name); submitBtn.disabled = !ok; submitBtn.classList.toggle("btn-dim", !ok); }
    }
    if (el.dataset.action === "ls-name-input") {
      state.linkSite.name = el.value;
      state.linkSite.error = "";
      const preview = document.querySelector(".ns-preview");
      if (preview) {
        if (el.value) { preview.classList.remove("muted"); preview.textContent = "→ https://" + el.value + ".dev"; }
        else { preview.classList.add("muted"); preview.innerHTML = "→ https://&lt;name&gt;.dev"; }
      }
      const submitBtn = document.querySelector('[data-action="ls-submit"]');
      if (submitBtn) { const ok = state.linkSite.root && validName(el.value); submitBtn.disabled = !ok; submitBtn.classList.toggle("btn-dim", !ok); }
    }
    if (el.dataset.action === "px-name-input") {
      state.proxy.name = el.value;
      state.proxy.error = "";
      const preview = document.querySelector(".ns-preview");
      if (preview) {
        if (el.value) { preview.classList.remove("muted"); preview.textContent = "→ https://" + el.value + ".dev"; }
        else { preview.classList.add("muted"); preview.innerHTML = "→ https://&lt;name&gt;.dev"; }
      }
      const submitBtn = document.querySelector('[data-action="px-submit"]');
      if (submitBtn) { const ok = validName(el.value) && state.proxy.routes.length > 0; submitBtn.disabled = !ok; submitBtn.classList.toggle("btn-dim", !ok); }
    }
    if (el.dataset.action === "pr-path") { state.proxy.routes[parseInt(el.dataset.idx, 10)].path = el.value; }
    if (el.dataset.action === "pr-upstream") { state.proxy.routes[parseInt(el.dataset.idx, 10)].upstream = el.value; }
    if (el.dataset.action === "dm-input") { state.siteDomains.domains[parseInt(el.dataset.idx, 10)] = el.value; }
    if (el.dataset.action === "php-ini-input") { if (state.modal && state.modal.phpIni) state.modal.phpIni[el.dataset.key] = el.value; }
  });

  app.addEventListener("change", (e) => {
    const el = e.target;
    if (el.dataset.action === "ns-template-change") {
      state.newSite.template = el.value;
    }
    if (el.dataset.action === "px-ws") { state.proxy.websocket = el.checked; }
  });

  // ---- Esc closes modal ----
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && state.modal === "newsite") closeNewSite();
    else if (e.key === "Escape" && state.modal === "linksite") closeLinkSite();
    else if (e.key === "Escape" && state.modal === "proxy") closeProxy();
    else if (e.key === "Escape" && state.modal === "domains") closeDomains();
    else if (e.key === "Escape" && state.modal && state.modal.open) closeTool();
  });

  // ---- focus-trap inside modal ----
  app.addEventListener("keydown", (e) => {
    if (e.key !== "Tab" || (state.modal !== "newsite" && state.modal !== "linksite" && state.modal !== "proxy" && state.modal !== "domains")) return;
    const card = document.querySelector(".ns-card");
    if (!card) return;
    const focusable = Array.from(card.querySelectorAll('button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex="-1"])'));
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey) {
      if (document.activeElement === first) { e.preventDefault(); last.focus(); }
    } else {
      if (document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
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
  if (TAURI && TAURI.event && TAURI.event.listen) {
    TAURI.event.listen("download-progress", (e) => { applyProgress(e.payload); updateRing(); });
    TAURI.event.listen("services-changed", (e) => {
      // While a command is in flight, the UI holds an optimistic state
      // (e.g. "Starting" during an async Start All whose orch lock is briefly
      // free mid-pkexec); don't let a monitor snapshot clobber it — the command
      // return reconciles, and the monitor re-emits afterwards. Mirrors the old
      // poll's `if (state.busy) return`.
      if (state.busy) return;
      applyServices(e.payload);
      if (!state.modal) render();
    });
    TAURI.event.listen("sites-changed", () => {
      invoke("list_sites").then((s) => {
        state.sites = Array.isArray(s) ? s : [];
        if (!state.modal) render();
      }).catch(() => {});
    });
  }
  render();
  refresh();
})();
