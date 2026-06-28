import { state } from "../../state";
import type { ToolModalState } from "../../state";
import { esc } from "../util";
import { I } from "../icons";
import { toast } from "../toast";
import {
  toolVersions, toolSymlinks, installToolVersion as installToolVersionCmd,
  setToolVersion as setToolVersionCmd, setToolSymlink, phpIniSettings, setPhpIniSettings,
} from "../../ipc/commands";
import { render, resetDownload, progressRing } from "../render";
import { DISP_COMP, TOOL_KEY, TOOL_CLI } from "../constants";

/** Type guard — true only when the modal holds a ToolModalState object. */
function isToolModal(m: typeof state.modal): m is ToolModalState {
  return typeof m === "object" && m !== null && (m as ToolModalState).open === true;
}

function phpIniField(label: string, key: string, val: string | number | boolean): string {
  return '<div class="set-row"><div class="grow"><div class="t">' + esc(label) + "</div></div>" +
    '<input class="ns-input" data-action="php-ini-input" data-key="' + key + '" value="' + esc(String(val)) + '" /></div>';
}

function phpIniToggle(label: string, key: string, on: boolean): string {
  return '<div class="set-row"><div class="grow"><div class="t">' + esc(label) + "</div></div>" +
    '<button class="btn-sm" data-action="php-ini-toggle" data-key="' + key + '">' + (on ? "On" : "Off") + "</button></div>";
}

export function toolModal(): string {
  const m = state.modal;
  if (!isToolModal(m)) return "";
  const verRows = m.versions
    .map((v) => {
      let right: string;
      if (m.busy && m.busyVersion === v.version) right = progressRing();
      else if (v.active) right = '<span class="tag ok">Active</span>';
      else if (v.installed) right = '<button class="btn-sm" data-action="use-tool-version" data-version="' + esc(v.version) + '"' + (m.busy ? " disabled" : "") + ">Use</button>";
      else right = '<button class="btn-sm" data-action="install-tool-version" data-version="' + esc(v.version) + '"' + (m.busy ? " disabled" : "") + ">Install</button>";
      return '<div class="set-row" data-key="ver-' + esc(v.version) + '"><div class="grow"><div class="t">' + esc(m.display) + " " + esc(v.version) + '</div><div class="h">' + (v.installed ? "Installed" : "Not installed") + "</div></div>" + right + "</div>";
    })
    .join("") || '<div class="set-row"><div class="h">No versions — run "Install missing" first.</div></div>';

  const anyInstalled = m.versions.some((v) => v.installed);
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

export async function openTool(toolKey: string): Promise<void> {
  const comp = Object.keys(TOOL_KEY).find((k) => TOOL_KEY[k] === toolKey);
  state.modal = {
    open: true, toolKey, display: DISP_COMP[comp as string] || toolKey, cliBinary: TOOL_CLI[toolKey] ?? null,
    versions: [], linked: false, busy: false, busyVersion: null,
  };
  render();
  // Narrow to ToolModalState after assignment — safe because we just set it above.
  const m = state.modal as ToolModalState;
  try {
    const [versions, linked] = await Promise.all([
      toolVersions(toolKey),
      toolSymlinks(),
    ]);
    m.versions = versions;
    state.toolSymlinks = linked;
    m.linked = linked.includes(toolKey);
  } catch (e) {
    toast({ type: "error", title: "Load failed", msg: String(e) });
  }
  if (toolKey === "php") {
    try { m.phpIni = await phpIniSettings(); }
    catch (_) { m.phpIni = null; }
  }
  render();
}

export function closeTool(): void {
  const m = state.modal;
  if (isToolModal(m) && m.busy) return;
  state.modal = null;
  render();
}

export async function useToolVersion(version: string): Promise<void> {
  const m = state.modal as ToolModalState;
  const tk = m.toolKey;
  m.busy = true; m.busyVersion = version; render();
  try {
    await setToolVersionCmd(tk, version);
    m.versions = await toolVersions(tk);
    toast({ type: "success", title: "Version switched", msg: m.display + " " + version });
  } catch (e) {
    toast({ type: "error", title: "Switch failed", msg: String(e) });
  } finally {
    if (isToolModal(state.modal)) { state.modal.busy = false; state.modal.busyVersion = null; }
    resetDownload(); render();
  }
}

export async function installToolVersion(version: string): Promise<void> {
  const m = state.modal as ToolModalState;
  const tk = m.toolKey;
  m.busy = true; m.busyVersion = version; render();
  try {
    m.versions = await installToolVersionCmd(tk, version);
    toast({ type: "success", title: "Installed", msg: m.display + " " + version });
  } catch (e) {
    toast({ type: "error", title: "Install failed", msg: String(e) });
  } finally {
    if (isToolModal(state.modal)) { state.modal.busy = false; state.modal.busyVersion = null; }
    resetDownload(); render();
  }
}

export async function toggleToolSymlink(): Promise<void> {
  const m = state.modal as ToolModalState;
  const tk = m.toolKey;
  const next = !m.linked;
  m.busy = true; render();
  try {
    state.toolSymlinks = await setToolSymlink(tk, next);
    m.linked = state.toolSymlinks.includes(tk);
    toast({ type: "success", title: next ? "Linked" : "Unlinked",
            msg: String(m.cliBinary).split(", ").map((b: string) => "/usr/local/bin/" + b).join(", ") });
  } catch (e) {
    toast({ type: "error", title: "Symlink failed", msg: String(e) });
  } finally {
    if (isToolModal(state.modal)) { state.modal.busy = false; }
    render();
  }
}

export async function applyPhpIni(): Promise<void> {
  if (!isToolModal(state.modal) || !state.modal.phpIni) return;
  const m = state.modal;
  const pi = m.phpIni!; // narrowed: guard above confirmed it is truthy
  const payload = { ...pi };
  payload.max_execution_time = parseInt(String(payload.max_execution_time), 10) || 0;
  m.busy = true; render();
  try {
    m.phpIni = await setPhpIniSettings(payload);
    toast({ type: "success", title: "PHP settings applied", msg: "Restarted php-fpm; CLI uses them too." });
  } catch (e) {
    toast({ type: "error", title: "Couldn't apply PHP settings", msg: String(e) });
  } finally {
    if (isToolModal(state.modal)) state.modal.busy = false;
    render();
  }
}
