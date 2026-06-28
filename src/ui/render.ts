// render.ts — the SOLE #app.innerHTML site.
// Owns render(), refresh(), layout helpers, and the download/service/component helpers.
// No other module may assign app.innerHTML.
import { state } from "../state";
import type { ServiceStatus, ComponentStatus, ProgressPayload } from "../ipc/types";
import { esc } from "./util";
import { I } from "./icons";
import { toasts } from "./toast";
import { COMP_ORDER, SVC_KINDS } from "./constants";
import { dashboard } from "./views/dashboard";
import { sitesView } from "./views/sites";
import { setupView } from "./views/setup";
import { settingsView } from "./views/settings";
import { toolModal } from "./modals/tool";
import { newSiteModal } from "./modals/newsite";
import { linkSiteModal } from "./modals/linksite";
import { proxyModal } from "./modals/proxy";
import { domainsModal } from "./modals/domains";
import { stackStatus, listSites, setupStatus } from "../ipc/commands";

// ---- shared helpers (single copy) ----

export function applyServices(arr: ServiceStatus[]): void {
  if (!Array.isArray(arr)) return;
  for (const s of arr) if (s && s.kind in state.services) state.services[s.kind] = s.state;
}

export function applyComponents(arr: ComponentStatus[]): void {
  if (!Array.isArray(arr)) return;
  const byName: Record<string, boolean> = {};
  for (const c of arr) byName[c.component] = !!c.present;
  state.setup.components = COMP_ORDER.map((c) => ({ component: c, present: !!byName[c] }));
}

export function resetDownload(): void {
  state.download = { active: false, label: "", step: { done: 0, total: 0 }, bytes: { current: 0, total: 0 }, overall: 0 };
}

// Combine step (files done / total) + the current file's byte fraction into a
// single OVERALL fraction (0..1) for the whole operation, clamped monotonic so
// it only ever fills forward — never resets per file (which would mislead).
function computeOverall(): void {
  const d = state.download;
  const byteFrac = d.bytes.total > 0 ? Math.min(1, d.bytes.current / d.bytes.total) : 0;
  const raw = d.step.total > 0 ? (d.step.done + byteFrac) / d.step.total : byteFrac;
  d.overall = Math.max(d.overall, Math.min(1, raw));
}

export function applyProgress(p: ProgressPayload): void {
  if (!p || !p.kind) return;
  state.download.active = true;
  if (p.kind === "phase") state.download.label = String(p.label || "");
  else if (p.kind === "step") { state.download.step = { done: p.done ?? 0, total: p.total ?? 0 }; if (p.label) state.download.label = String(p.label); }
  else if (p.kind === "bytes") state.download.bytes = { current: p.current ?? 0, total: p.total ?? 0 };
  computeOverall();
}

// Small inline ring that fills to the OVERALL fraction (no % number). Stable
// DOM (determinate + spin circle both present, toggled via `.ring-hide`) so
// updateRing() can mutate it in place without a full re-render. Spins
// indeterminately until the first progress (overall === 0), then fills.
export function progressRing(): string {
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
export function updateRing(): void {
  const ring = document.querySelector(".ring-sm");
  if (!ring) return;
  const d = state.download;
  const R = 9, C = 2 * Math.PI * R;
  const has = d.overall > 0;
  const fg = ring.querySelector(".ring-fg");
  if (fg) { fg.setAttribute("stroke-dashoffset", String(C * (1 - Math.min(1, d.overall)))); fg.classList.toggle("ring-hide", !has); }
  const spin = ring.querySelector(".ring-spin");
  if (spin) spin.classList.toggle("ring-hide", has);
}

// ---- layout helpers (used only by render) ----

function runningCount(): number {
  return SVC_KINDS.filter((k) => state.services[k] === "Running").length;
}

function missingCount(): number {
  return state.setup.components.filter((c) => !c.present).length;
}

function spinner(klass: string): string {
  return '<span class="spin spinner ' + klass + '"></span>';
}

function header(): string {
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

function pkexecBanner(): string {
  if (!state.pkexecMsg) return "";
  return (
    '<div class="pkexec" role="status">' + I.lock +
    '<span class="msg">' + esc(state.pkexecMsg) + "</span></div>"
  );
}

function navItem(view: string, label: string, icon: string, opts: { dot?: boolean; badge?: number | null; grow?: boolean } = {}): string {
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

function sidebar(): string {
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
let lastSig = "";
let lastView: string | null = null;

export function render(): void {
  const app = document.getElementById("app")!;
  document.documentElement.dataset.theme = state.dark ? "dark" : "light";
  let main: string;
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
  let fId: string | null = null, fAction: string | null = null, fIdx: string | null = null;
  let selS: number | null = null, selE: number | null = null;
  if (ae && app.contains(ae) && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA")) {
    fId = (ae as HTMLElement).id || null;
    fAction = (ae as HTMLElement).getAttribute("data-action");
    fIdx = (ae as HTMLElement).getAttribute("data-idx");
    try { selS = (ae as HTMLInputElement).selectionStart; selE = (ae as HTMLInputElement).selectionEnd; } catch (_) {}
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
    let el: HTMLElement | null = fId ? document.getElementById(fId) : null;
    if (!el && fAction) {
      let sel = '[data-action="' + fAction + '"]';
      if (fIdx != null) sel += '[data-idx="' + fIdx + '"]';
      el = app.querySelector(sel);
    }
    if (el) {
      el.focus();
      if (selS != null) { try { (el as HTMLInputElement).setSelectionRange(selS, selE as number); } catch (_) {} }
    }
  }
}

export async function refresh(): Promise<void> {
  if (state.busy) return;
  try {
    const [svc, sites, comps] = await Promise.all([
      stackStatus(),
      listSites(),
      setupStatus(),
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
