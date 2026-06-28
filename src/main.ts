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
import { toolModal, openTool, closeTool, useToolVersion, installToolVersion, toggleToolSymlink, applyPhpIni } from "./ui/modals/tool";
import { newSiteModal } from "./ui/modals/newsite";
import { linkSiteModal } from "./ui/modals/linksite";
import { proxyModal } from "./ui/modals/proxy";
import { domainsModal } from "./ui/modals/domains";
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
