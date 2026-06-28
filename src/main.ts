import "./styles.css";
import { state } from "./state";
import { render, refresh, applyServices, applyProgress, updateRing, loadServiceFlags } from "./ui/render";
import { onServicesChanged, onSitesChanged, onDownloadProgress } from "./ipc/events";
import { listSites } from "./ipc/commands";
import { bindEvents } from "./ui/events";

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
  ro.observe(document.getElementById("app")!);
}

// ---- boot ----
bindEvents();

onDownloadProgress((payload) => { applyProgress(payload); updateRing(); });
onServicesChanged((statuses) => {
  // While a command is in flight, the UI holds an optimistic state
  // (e.g. "Starting" during an async Start All whose orch lock is briefly
  // free mid-pkexec); don't let a monitor snapshot clobber it — the command
  // return reconciles, and the monitor re-emits afterwards. Mirrors the old
  // poll's `if (state.busy) return`.
  if (state.busy) return;
  applyServices(statuses);
  if (!state.modal) render();
});
onSitesChanged(() => {
  listSites().then((s) => {
    state.sites = Array.isArray(s) ? s : [];
    if (!state.modal) render();
  }).catch(() => {});
});

(async () => {
  await loadServiceFlags();
  render();
  refresh();
})();
