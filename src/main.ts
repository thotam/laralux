import "./styles.css";
import { state } from "./state";
import { render, refresh, applyServices, applyProgress, updateRing, loadServiceFlags, loadLaunchConfig } from "./ui/render";
import { onServicesChanged, onSitesChanged, onDownloadProgress, onSiteProcsChanged } from "./ipc/events";
import { listSites, siteProcCounts } from "./ipc/commands";
import { refreshProcs } from "./ui/modals/procs";
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
async function loadProcCounts(): Promise<void> {
  try { state.procCounts = await siteProcCounts(); render(); } catch { /* ignore */ }
}

onSitesChanged(() => {
  listSites().then((s) => {
    state.sites = Array.isArray(s) ? s : [];
    if (!state.modal) render();
  }).catch(() => {});
  loadProcCounts();
});
onSiteProcsChanged(() => { refreshProcs(); });

(async () => {
  await loadServiceFlags();
  await loadLaunchConfig();
  render();
  refresh();
  loadProcCounts();
})();
