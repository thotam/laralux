// @ts-nocheck  -- temporary: the monolith is typed module-by-module in later tasks; removed in the final task.
import "./styles.css";
import { state } from "./state";
import { render, refresh, applyServices, applyProgress, updateRing } from "./ui/render";
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
  ro.observe(document.getElementById("app"));
}

// ---- boot ----
bindEvents();
render();

const TAURI = window.__TAURI__;
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
    const invoke = (cmd, args) => {
      if (!TAURI || !TAURI.core) return Promise.reject(new Error("Tauri unavailable"));
      return TAURI.core.invoke(cmd, args);
    };
    invoke("list_sites").then((s) => {
      state.sites = Array.isArray(s) ? s : [];
      if (!state.modal) render();
    }).catch(() => {});
  });
}

refresh();
