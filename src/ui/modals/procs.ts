import { state } from "../../state";
import { esc } from "../util";
import { render } from "../render";
import { toast } from "../toast";
import { META } from "../constants";
import {
  siteProcs, startSiteProc, stopSiteProc, startSiteProcs, stopSiteProcs,
  setSiteAutostart, siteProcLogPath,
} from "../../ipc/commands";

export async function openProcs(name: string, root: string): Promise<void> {
  state.procModal = { name, root };
  state.siteProcs = null;
  state.modal = "procs";
  render();
  try {
    state.siteProcs = await siteProcs(name, root);
  } catch (e) {
    toast({ type: "error", title: "Couldn't load processes", msg: String(e) });
  }
  render();
}

export function closeProcs(): void {
  state.modal = null;
  state.procModal = null;
  state.siteProcs = null;
  render();
}

/** Re-fetch the open modal (used by the live event + after actions). */
export async function refreshProcs(): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try {
    state.siteProcs = await siteProcs(name, root);
    render();
  } catch { /* modal may have closed */ }
}

export async function procStart(proc: string): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try { state.siteProcs = await startSiteProc(name, root, proc); }
  catch (e) { toast({ type: "error", title: proc + " start failed", msg: String(e) }); }
  render();
}

export async function procStop(proc: string): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try { state.siteProcs = await stopSiteProc(name, root, proc); }
  catch (e) { toast({ type: "error", title: proc + " stop failed", msg: String(e) }); }
  render();
}

export async function procStartAll(): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try { state.siteProcs = await startSiteProcs(name, root); }
  catch (e) { toast({ type: "error", title: "Start all failed", msg: String(e) }); }
  render();
}

export async function procStopAll(): Promise<void> {
  if (!state.procModal) return;
  const { name, root } = state.procModal;
  try { state.siteProcs = await stopSiteProcs(name, root); }
  catch (e) { toast({ type: "error", title: "Stop all failed", msg: String(e) }); }
  render();
}

export async function procToggleAutostart(): Promise<void> {
  if (!state.procModal || !state.siteProcs) return;
  const { name } = state.procModal;
  const next = !state.siteProcs.autostart;
  state.siteProcs = { ...state.siteProcs, autostart: next };
  render();
  try { await setSiteAutostart(name, next); }
  catch (e) {
    state.siteProcs = { ...state.siteProcs!, autostart: !next };
    toast({ type: "error", title: "Couldn't change autostart", msg: String(e) });
    render();
  }
}

export async function procLogs(proc: string): Promise<void> {
  if (!state.procModal) return;
  const { name } = state.procModal;
  try {
    const path = await siteProcLogPath(name, proc);
    toast({ type: "info", sticky: true, title: proc + " logs", details: ["Log file: " + path, "or: tail -f " + path] });
  } catch (e) {
    toast({ type: "error", title: "No log path", msg: String(e) });
  }
}

export function procsModal(): string {
  const m = state.procModal;
  const view = state.siteProcs;
  if (!m) return "";
  const autostart = view?.autostart ?? false;
  const rows = !view
    ? '<div class="proc-empty">Loading…</div>'
    : view.procs.length === 0
      ? '<div class="proc-empty">No Procfile in this site’s folder.</div>'
      : view.procs.map((p) => {
          const meta = META[p.state] || META.Stopped;
          const running = p.state === "Running" || p.state === "Starting";
          // Without this a crashed process just reads "Crashed" and the user
          // cannot tell whether Laralux is still retrying or has stopped trying.
          const note =
            running || p.failures === 0
              ? ""
              : p.failures >= 5
                ? '<span class="proc-note">gave up after 5 restarts</span>'
                : '<span class="proc-note">retrying (' + p.failures + "/5)…</span>";
          const btn = running
            ? '<button class="btn-sm" data-action="proc-stop" data-proc="' + esc(p.name) + '">Stop</button>'
            : '<button class="btn-sm primary" data-action="proc-start" data-proc="' + esc(p.name) + '">Start</button>';
          return (
            '<div class="proc-row"><div class="proc-info">' +
            '<div class="proc-name"><span class="dot bgc-' + meta.cls + '"></span>' + esc(p.name) + note + "</div>" +
            '<code class="proc-cmd">' + esc(p.command) + "</code></div>" +
            '<div class="proc-actions">' + btn +
            '<button class="btn-xs" data-action="proc-logs" data-proc="' + esc(p.name) + '">Logs</button></div></div>'
          );
        }).join("");
  return (
    '<div class="ns-overlay" data-action="procs-overlay-click" role="dialog" aria-modal="true" aria-labelledby="procs-title">' +
    '<div class="ns-card">' +
    '<div class="ns-head"><h2 class="ns-title" id="procs-title">Processes — ' + esc(m.name) + "</h2>" +
    '<button class="icon-btn" data-action="close-procs" aria-label="Close">\xd7</button></div>' +
    '<div class="ns-body">' +
    '<div class="proc-toolbar">' +
    '<button class="' + (autostart ? "toggle-on" : "toggle-off") + '" data-action="proc-autostart" aria-pressed="' + autostart + '"><span class="knob"></span></button>' +
    '<span class="proc-toolbar-label">Autostart with “Start All”</span><span class="spacer"></span>' +
    '<button class="btn-sm" data-action="proc-start-all">Start all</button>' +
    '<button class="btn-sm" data-action="proc-stop-all">Stop all</button></div>' +
    rows +
    "</div></div></div>"
  );
}
