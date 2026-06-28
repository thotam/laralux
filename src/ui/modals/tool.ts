// NOTE: DISP_COMP, TOOL_KEY, TOOL_CLI are local copies — Task 6 dedupes these
// into a shared constants module (also duplicated in main.ts).
import { state } from "../../state";
import { esc } from "../util";
import { I } from "../icons";
import { toast } from "../toast";
import { invoke } from "../legacy-invoke";
import { render } from "../loop";

const DISP_COMP: Record<string, string> = { Nginx: "Nginx", Php: "PHP", Mariadb: "MariaDB", Redis: "Redis", Mkcert: "mkcert", Mailpit: "Mailpit", Composer: "Composer", Node: "Node.js" };
const TOOL_KEY: Record<string, string> = { Nginx: "nginx", Php: "php", Mariadb: "mariadb", Redis: "redis", Mkcert: "mkcert", Mailpit: "mailpit", Composer: "composer", Node: "node" };
const TOOL_CLI: Record<string, string | null> = { nginx: "nginx", php: "php", mariadb: "mariadb", redis: "redis-cli", mkcert: "mkcert", mailpit: null, composer: "composer", node: "node, npm, npx" };

function resetDownload(): void {
  state.download = { active: false, label: "", step: { done: 0, total: 0 }, bytes: { current: 0, total: 0 }, overall: 0 };
}

function progressRing(): string {
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

function phpIniField(label: string, key: string, val: any): string {
  return '<div class="set-row"><div class="grow"><div class="t">' + esc(label) + "</div></div>" +
    '<input class="ns-input" data-action="php-ini-input" data-key="' + key + '" value="' + esc(String(val)) + '" /></div>';
}

function phpIniToggle(label: string, key: string, on: boolean): string {
  return '<div class="set-row"><div class="grow"><div class="t">' + esc(label) + "</div></div>" +
    '<button class="btn-sm" data-action="php-ini-toggle" data-key="' + key + '">' + (on ? "On" : "Off") + "</button></div>";
}

export function toolModal(): string {
  const m = state.modal;
  if (!m || !m.open) return "";
  const verRows = (m.versions || [])
    .map((v: any) => {
      let right: string;
      if (m.busy && m.busyVersion === v.version) right = progressRing();
      else if (v.active) right = '<span class="tag ok">Active</span>';
      else if (v.installed) right = '<button class="btn-sm" data-action="use-tool-version" data-version="' + esc(v.version) + '"' + (m.busy ? " disabled" : "") + ">Use</button>";
      else right = '<button class="btn-sm" data-action="install-tool-version" data-version="' + esc(v.version) + '"' + (m.busy ? " disabled" : "") + ">Install</button>";
      return '<div class="set-row"><div class="grow"><div class="t">' + esc(m.display) + " " + esc(v.version) + '</div><div class="h">' + (v.installed ? "Installed" : "Not installed") + "</div></div>" + right + "</div>";
    })
    .join("") || '<div class="set-row"><div class="h">No versions — run "Install missing" first.</div></div>';

  const anyInstalled = (m.versions || []).some((v: any) => v.installed);
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
    open: true, toolKey, display: DISP_COMP[comp as string] || toolKey, cliBinary: TOOL_CLI[toolKey],
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

export function closeTool(): void {
  if (state.modal && state.modal.busy) return;
  state.modal = null;
  render();
}

export async function useToolVersion(version: string): Promise<void> {
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

export async function installToolVersion(version: string): Promise<void> {
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

export async function toggleToolSymlink(): Promise<void> {
  const tk = state.modal.toolKey;
  const next = !state.modal.linked;
  state.modal.busy = true; render();
  try {
    state.toolSymlinks = await invoke("set_tool_symlink", { tool: tk, enabled: next });
    state.modal.linked = state.toolSymlinks.includes(tk);
    toast({ type: "success", title: next ? "Linked" : "Unlinked",
            msg: String(state.modal.cliBinary).split(", ").map((b: string) => "/usr/local/bin/" + b).join(", ") });
  } catch (e) {
    toast({ type: "error", title: "Symlink failed", msg: String(e) });
  } finally {
    if (state.modal) { state.modal.busy = false; } render();
  }
}

export async function applyPhpIni(): Promise<void> {
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
