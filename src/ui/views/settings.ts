import { state } from "../../state";
import { render } from "../render";
import { SVC_ORDER, DISP, FLAG_KEY } from "../constants";
import { setServiceEnabled, serviceFlags, setLaunchOption } from "../../ipc/commands";
import { toast } from "../toast";

export function settingsView(): string {
  return (
    '<div class="view narrow-620">' +
    '<div><h1 class="h1">Settings</h1><p class="subtitle">Appearance and environment defaults.</p></div>' +
    '<div class="card settings-card">' +
    '<div class="set-row"><div class="grow"><div class="t">Appearance</div><div class="h">Light / dark theme</div></div>' +
    '<button class="btn-sm" data-action="toggle-dark">' + (state.dark ? "Dark" : "Light") + "</button></div>" +
    '<div class="set-row"><div class="grow"><div class="t">Local TLD</div><div class="h">Pretty-URL domain suffix</div></div>' +
    '<code class="code-chip">.dev</code></div>' +
    '<div class="set-row"><div class="grow"><div class="t">Sites directory</div><div class="h">Where projects are scanned</div></div>' +
    '<code class="code-chip">~/laralux/www</code></div>' +
    launchRow("start_on_login", "Start on login", "Launch Laralux when you log in") +
    launchRow("start_minimized", "Start minimized to tray", "Launch hidden — open from the tray icon") +
    launchRow("autostart_services", "Auto-start services on launch", "Start the stack automatically when the app opens") +
    '<div class="set-row"><div class="grow"><div class="t">Services</div><div class="h">Enable/disable services in the stack</div></div></div>' +
    SVC_ORDER.map((k) => {
      const on = !!state.serviceFlags[FLAG_KEY[k]];
      return '<div class="set-row sub"><div class="grow"><div class="t">' + DISP[k] + "</div></div>" +
        '<button class="btn-xs" data-action="open-tool" data-tool="' + FLAG_KEY[k] + '">Manage</button>' +
        '<button class="' + (on ? "toggle-on" : "toggle-off") + '" data-action="svc-enable" data-kind="' + k + '" aria-pressed="' + on + '"><span class="knob"></span></button></div>';
    }).join("") +
    "</div>" +
    '<div class="settings-foot">Laralux · window 900×600 · min 720×480 · tray: Start All · Stop All · Dashboard · Quit</div>' +
    "</div>"
  );
}

export function toggleDark(): void {
  state.dark = !state.dark;
  localStorage.setItem("laralux-theme", state.dark ? "dark" : "light");
  render();
}

function launchRow(key: string, title: string, hint: string): string {
  const on = !!(state.launch as unknown as Record<string, boolean>)[key];
  return '<div class="set-row"><div class="grow"><div class="t">' + title + '</div><div class="h">' + hint + "</div></div>" +
    '<button class="' + (on ? "toggle-on" : "toggle-off") + '" data-action="launch-option" data-key="' + key + '" aria-pressed="' + on + '"><span class="knob"></span></button></div>';
}

export async function toggleLaunchOption(key: string): Promise<void> {
  const cur = state.launch as unknown as Record<string, boolean>;
  const next = !cur[key];
  state.launch = { ...state.launch, [key]: next };
  render();
  try {
    state.launch = await setLaunchOption(key, next);
  } catch (e) {
    state.launch = { ...state.launch, [key]: !next };
    toast({ type: "error", title: "Couldn't change launch setting", msg: String(e) });
  }
  render();
}

export async function toggleServiceEnabled(kind: string): Promise<void> {
  const flagKey = FLAG_KEY[kind];
  const next = !state.serviceFlags[flagKey];
  state.serviceFlags = { ...state.serviceFlags, [flagKey]: next };
  render();
  try {
    await setServiceEnabled(kind, next);
    // Refresh both the snapshot and the persisted flags.
    const f = await serviceFlags();
    if (f && typeof f === "object") state.serviceFlags = f as unknown as Record<string, boolean>;
  } catch (e) {
    state.serviceFlags = { ...state.serviceFlags, [flagKey]: !next };
  }
  render();
}
