// events.ts — delegated event listeners (bound once at boot via bindEvents()).
// All data-action dispatch is here; no other module may add delegated listeners.
import { state } from "../state";
import { validName } from "./util";
import { render } from "./render";
import { startAll, stopAll, toggleService, viewLogs } from "./views/dashboard";
import {
  openNewSite, closeNewSite, submitNewSite,
  openLinkSite, closeLinkSite, browseFolder, submitLinkSite,
  openProxy, closeProxy, addProxyRoute, delProxyRoute, submitProxy,
  openDomains, closeDomains, addDomainRow, delDomainRow, submitDomains,
  removeSite, copySite, openTerminal, openExternal,
} from "./views/sites";
import { runSetup } from "./views/setup";
import { toggleDark } from "./views/settings";
import {
  openTool, closeTool, useToolVersion, installToolVersion,
  toggleToolSymlink, applyPhpIni,
} from "./modals/tool";
import { dismiss } from "./toast";

function setView(v: string): void {
  state.view = v;
  state.confirmRemove = null;
  render();
}

export function bindEvents(): void {
  const app = document.getElementById("app")!;

  // ---- click (main delegated dispatcher) ----
  app.addEventListener("click", (e: MouseEvent) => {
    const el = (e.target as Element).closest("[data-action]") as HTMLElement | null;
    if (!el) return;
    const a = el.getAttribute("data-action");
    if (a === "nav") setView(el.getAttribute("data-view")!);
    else if (a === "toggle-dark") toggleDark();
    else if (a === "start-all") startAll();
    else if (a === "stop-all") stopAll();
    else if (a === "run-setup") runSetup();
    else if (a === "svc-toggle") toggleService(el.getAttribute("data-kind")!);
    else if (a === "svc-logs") viewLogs(el.getAttribute("data-kind")!);
    else if (a === "copy-site") copySite(el.getAttribute("data-name")!);
    else if (a === "open-terminal") openTerminal(el.getAttribute("data-path")!);
    else if (a === "open-url") { e.preventDefault(); openExternal(el.getAttribute("data-url")!); }
    else if (a === "open-tool") openTool((el as any).dataset.tool);
    else if (a === "close-tool") closeTool();
    else if (a === "use-tool-version") useToolVersion((el as any).dataset.version);
    else if (a === "install-tool-version") installToolVersion((el as any).dataset.version);
    else if (a === "toggle-tool-symlink") toggleToolSymlink();
    else if (a === "php-ini-toggle") {
      if (state.modal && state.modal.phpIni) {
        state.modal.phpIni[(el as any).dataset.key] = !state.modal.phpIni[(el as any).dataset.key];
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
    else if (a === "remove-site") removeSite(el.getAttribute("data-name")!);
    else if (a === "ls-close") closeLinkSite();
    else if (a === "ls-submit") submitLinkSite();
    else if (a === "ls-browse") browseFolder();
    else if (a === "ls-overlay-click") { if (e.target === el) closeLinkSite(); }
    else if (a === "proxy-site") openProxy();
    else if (a === "edit-proxy") openProxy(state.sites.find((s: any) => s.name === el.getAttribute("data-name")));
    else if (a === "px-close") closeProxy();
    else if (a === "px-submit") submitProxy();
    else if (a === "pr-add") addProxyRoute();
    else if (a === "pr-del") delProxyRoute(parseInt(el.getAttribute("data-idx")!, 10));
    else if (a === "px-overlay-click") { if (e.target === el) closeProxy(); }
    else if (a === "edit-domains") openDomains(state.sites.find((s: any) => s.name === el.getAttribute("data-name")));
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
    if (el.dataset.action === "php-ini-input") { if (state.modal && state.modal.phpIni) state.modal.phpIni[el.dataset.key!] = el.value; }
  });

  app.addEventListener("change", (e: Event) => {
    const el = e.target as HTMLInputElement;
    if (el.dataset.action === "ns-template-change") {
      state.newSite.template = el.value;
    }
    if (el.dataset.action === "px-ws") { state.proxy.websocket = el.checked; }
  });

  // ---- Esc closes modal ----
  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key === "Escape" && state.modal === "newsite") closeNewSite();
    else if (e.key === "Escape" && state.modal === "linksite") closeLinkSite();
    else if (e.key === "Escape" && state.modal === "proxy") closeProxy();
    else if (e.key === "Escape" && state.modal === "domains") closeDomains();
    else if (e.key === "Escape" && state.modal && state.modal.open) closeTool();
  });

  // ---- focus-trap inside modal ----
  app.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key !== "Tab" || (state.modal !== "newsite" && state.modal !== "linksite" && state.modal !== "proxy" && state.modal !== "domains")) return;
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
