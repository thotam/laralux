// events.ts — delegated event listeners (bound once at boot via bindEvents()).
// All data-action dispatch is here; no other module may add delegated listeners.
import { state } from "../state";
import type { ToolModalState } from "../state";
import { validName } from "./util";
import { render } from "./render";
import { startAll, stopAll, toggleService, viewLogs, launchDbClient } from "./views/dashboard";
import {
  openNewSite, closeNewSite, submitNewSite,
  openLinkSite, closeLinkSite, browseFolder, submitLinkSite,
  openProxy, closeProxy, addProxyRoute, delProxyRoute, submitProxy,
  openDomains, closeDomains, addDomainRow, delDomainRow, submitDomains,
  openDeleteSite, closeDeleteSite, runDeleteAction, copySite, openTerminal, openFolder, openExternal,
} from "./views/sites";
import { runSetup } from "./views/setup";
import { toggleDark, toggleServiceEnabled } from "./views/settings";
import {
  openTool, closeTool, useToolVersion, installToolVersion,
  toggleToolSymlink, applyPhpIni,
} from "./modals/tool";
import { dismiss } from "./toast";
import { hideSite, deleteSiteFolder, unlinkSite } from "../ipc/commands";

/** Narrow state.modal to ToolModalState (object with open: true). */
function isToolModal(m: typeof state.modal): m is ToolModalState {
  return typeof m === "object" && m !== null && (m as ToolModalState).open === true;
}

function setView(v: string): void {
  state.view = v;
  state.rowMenu = null;
  render();
}

export function bindEvents(): void {
  const app = document.getElementById("app")!;

  // ---- click (main delegated dispatcher) ----
  app.addEventListener("click", (e: MouseEvent) => {
    const el = (e.target as Element).closest("[data-action]") as HTMLElement | null;
    const a = el ? el.getAttribute("data-action") : null;

    // Dismiss the row kebab menu on any click except its own toggle or the
    // remove two-step. Covers outside clicks, quick buttons, and menu items.
    if (state.rowMenu && a !== "row-menu" && a !== "delete-site") {
      state.rowMenu = null;
      render();
      if (!el) return;
    }

    if (!el) return;
    if (a === "nav") setView(el.getAttribute("data-view")!);
    else if (a === "toggle-dark") toggleDark();
    else if (a === "svc-enable") toggleServiceEnabled(el.getAttribute("data-kind")!);
    else if (a === "start-all") startAll();
    else if (a === "stop-all") stopAll();
    else if (a === "run-setup") runSetup();
    else if (a === "svc-toggle") toggleService(el.getAttribute("data-kind")!);
    else if (a === "svc-logs") viewLogs(el.getAttribute("data-kind")!);
    else if (a === "open-db-client") launchDbClient();
    else if (a === "copy-site") copySite(el.getAttribute("data-name")!);
    else if (a === "open-terminal") openTerminal(el.getAttribute("data-path")!);
    else if (a === "open-folder") openFolder(el.getAttribute("data-path")!);
    else if (a === "row-menu") {
      const n = el.getAttribute("data-name")!;
      state.rowMenu = state.rowMenu === n ? null : n;
      render();
    }
    else if (a === "open-url") { e.preventDefault(); openExternal(el.getAttribute("data-url")!); }
    else if (a === "open-tool") openTool((el as HTMLElement & { dataset: DOMStringMap }).dataset.tool!);
    else if (a === "close-tool") closeTool();
    else if (a === "use-tool-version") useToolVersion((el as HTMLElement & { dataset: DOMStringMap }).dataset.version!);
    else if (a === "install-tool-version") installToolVersion((el as HTMLElement & { dataset: DOMStringMap }).dataset.version!);
    else if (a === "toggle-tool-symlink") toggleToolSymlink();
    else if (a === "php-ini-toggle") {
      const m = state.modal;
      if (isToolModal(m) && m.phpIni) {
        const key = (el as HTMLElement & { dataset: DOMStringMap }).dataset.key as keyof typeof m.phpIni;
        // PhpIniSettings has no index signature; go through unknown to allow string-keyed write.
        // The key comes from data-key on a button we render — limited to known PhpIniSettings keys.
        (m.phpIni as unknown as Record<string, string | number | boolean>)[key] =
          !m.phpIni[key];
        render();
      }
    }
    else if (a === "apply-php-ini") applyPhpIni();
    else if (a === "toast-dismiss") dismiss(parseInt(el.getAttribute("data-id")!, 10));
    else if (a === "new-site") openNewSite();
    else if (a === "ns-close") closeNewSite();
    else if (a === "ns-submit") submitNewSite();
    else if (a === "ns-overlay-click") {
      // close only if click is directly on the overlay (not the card inside it)
      if (e.target === el) closeNewSite();
    }
    else if (a === "link-site") openLinkSite();
    else if (a === "delete-site") openDeleteSite(el.getAttribute("data-name")!);
    else if (a === "ds-close") closeDeleteSite();
    else if (a === "ds-hide") runDeleteAction(hideSite);
    else if (a === "ds-delete") runDeleteAction(deleteSiteFolder);
    else if (a === "ds-remove") runDeleteAction(unlinkSite);
    else if (a === "ds-overlay-click") { if (e.target === el) closeDeleteSite(); }
    else if (a === "ls-close") closeLinkSite();
    else if (a === "ls-submit") submitLinkSite();
    else if (a === "ls-browse") browseFolder();
    else if (a === "ls-overlay-click") { if (e.target === el) closeLinkSite(); }
    else if (a === "proxy-site") openProxy();
    else if (a === "edit-proxy") { const s = state.sites.find((s) => s.name === el.getAttribute("data-name")); openProxy(s); }
    else if (a === "px-close") closeProxy();
    else if (a === "px-submit") submitProxy();
    else if (a === "pr-add") addProxyRoute();
    else if (a === "pr-del") delProxyRoute(parseInt(el.getAttribute("data-idx")!, 10));
    else if (a === "px-overlay-click") { if (e.target === el) closeProxy(); }
    else if (a === "edit-domains") { const s = state.sites.find((s) => s.name === el.getAttribute("data-name")); if (s) openDomains(s); }
    else if (a === "dm-close") closeDomains();
    else if (a === "dm-submit") submitDomains();
    else if (a === "dm-add") addDomainRow();
    else if (a === "dm-del") delDomainRow(parseInt(el.getAttribute("data-idx")!, 10));
    else if (a === "dm-overlay-click") { if (e.target === el) closeDomains(); }
  });

  // ---- modal input events (delegated on app) ----
  app.addEventListener("input", (e: Event) => {
    const el = e.target as HTMLInputElement;
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
      const submitBtn = document.querySelector('[data-action="ns-submit"]') as HTMLButtonElement | null;
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
      const submitBtn = document.querySelector('[data-action="ls-submit"]') as HTMLButtonElement | null;
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
      const submitBtn = document.querySelector('[data-action="ls-submit"]') as HTMLButtonElement | null;
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
      const submitBtn = document.querySelector('[data-action="px-submit"]') as HTMLButtonElement | null;
      if (submitBtn) { const ok = validName(el.value) && state.proxy.routes.length > 0; submitBtn.disabled = !ok; submitBtn.classList.toggle("btn-dim", !ok); }
    }
    if (el.dataset.action === "pr-path") { state.proxy.routes[parseInt(el.dataset.idx!, 10)].path = el.value; }
    if (el.dataset.action === "pr-upstream") { state.proxy.routes[parseInt(el.dataset.idx!, 10)].upstream = el.value; }
    if (el.dataset.action === "dm-input") { state.siteDomains.domains[parseInt(el.dataset.idx!, 10)] = el.value; }
    if (el.dataset.action === "php-ini-input") {
      const m = state.modal;
      if (isToolModal(m) && m.phpIni) {
        // Dynamic key from DOM: go through unknown to allow string-keyed write.
        // Keys are limited to known PhpIniSettings fields (rendered by tool.ts phpIniField).
        (m.phpIni as unknown as Record<string, string | number | boolean>)[el.dataset.key!] = el.value;
      }
    }
  });

  app.addEventListener("change", (e: Event) => {
    const el = e.target as HTMLInputElement;
    if (el.dataset.action === "ns-template-change") {
      // el.value comes from a <select> whose options are "Blank"|"Laravel"|"Wordpress".
      // The DOM guarantees the value is one of those; cast to the union.
      state.newSite.template = el.value as "Blank" | "Laravel" | "Wordpress";
    }
    if (el.dataset.action === "px-ws") { state.proxy.websocket = el.checked; }
  });

  // ---- Esc closes modal ----
  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key === "Escape" && state.modal === "newsite") closeNewSite();
    else if (e.key === "Escape" && state.modal === "linksite") closeLinkSite();
    else if (e.key === "Escape" && state.modal === "proxy") closeProxy();
    else if (e.key === "Escape" && state.modal === "domains") closeDomains();
    else if (e.key === "Escape" && isToolModal(state.modal)) closeTool();
    else if (e.key === "Escape" && state.modal === "deletesite") closeDeleteSite();
    else if (e.key === "Escape" && state.rowMenu) { state.rowMenu = null; render(); }
  });

  // ---- focus-trap inside modal ----
  app.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key !== "Tab" || (state.modal !== "newsite" && state.modal !== "linksite" && state.modal !== "proxy" && state.modal !== "domains" && state.modal !== "deletesite")) return;
    const card = document.querySelector(".ns-card");
    if (!card) return;
    const focusable = Array.from(card.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex="-1"])'));
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey) {
      if (document.activeElement === first) { e.preventDefault(); last.focus(); }
    } else {
      if (document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
  });
}
