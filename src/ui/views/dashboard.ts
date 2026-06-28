import { state } from "../../state";
import { esc } from "../util";
import { I } from "../icons";
import { toast } from "../toast";
import { invoke } from "../legacy-invoke";
import { render } from "../loop";

const SVC_KINDS = ["Nginx", "PhpFpm", "Mariadb", "Redis", "Mailpit"];
const DISP: Record<string, string> = { Nginx: "Nginx", PhpFpm: "PHP-FPM", Mariadb: "MariaDB", Redis: "Redis", Mailpit: "Mailpit" };
const SVC_ICON: Record<string, string> = { Nginx: I.svcNginx, PhpFpm: I.svcPhp, Mariadb: I.svcMaria, Redis: I.svcRedis, Mailpit: I.svcMail };
const PORTS: Record<string, string[]> = { Nginx: ["80", "443"], PhpFpm: ["socket"], Mariadb: ["3306"], Redis: ["6379"], Mailpit: ["8025", "1025"] };
const LOG_FILE: Record<string, string> = { Nginx: "nginx-error.log", PhpFpm: "php-fpm.log", Mariadb: "mariadb.log", Redis: "redis.log", Mailpit: "mailpit.log" };

const META: Record<string, { label: string; cls: string; busy: boolean; btn: string; primary: boolean }> = {
  Running:  { label: "Running",    cls: "running",  busy: false, btn: "Stop",     primary: false },
  Stopped:  { label: "Stopped",    cls: "stopped",  busy: false, btn: "Start",    primary: true  },
  Starting: { label: "Starting…",  cls: "starting", busy: true,  btn: "Starting", primary: false },
  Stopping: { label: "Stopping…",  cls: "starting", busy: true,  btn: "Stopping", primary: false },
  Crashed:  { label: "Crashed",    cls: "crashed",  busy: false, btn: "Restart",  primary: true  },
};

function applyServices(arr: any[]): void {
  if (!Array.isArray(arr)) return;
  for (const s of arr) if (s && s.kind in state.services) state.services[s.kind] = s.state;
}

export function runningCount(): number {
  return SVC_KINDS.filter((k) => state.services[k] === "Running").length;
}

function spinner(klass: string): string {
  return '<span class="spin spinner ' + klass + '"></span>';
}

function svcButton(kind: string, m: { label: string; cls: string; busy: boolean; btn: string; primary: boolean }): string {
  if (m.busy)
    return '<button class="btn-sm busy" disabled>' + spinner("muted") + esc(m.label) + "</button>";
  return (
    '<button class="btn-sm' + (m.primary ? " primary" : "") + '" data-action="svc-toggle" data-kind="' + kind + '">' +
    esc(m.btn) + "</button>"
  );
}

function serviceCard(kind: string): string {
  const st: string = state.services[kind] || "Stopped";
  const m = META[st] || META.Stopped;
  const dotPulse = m.busy ? " pulse" : "";
  const ports = (PORTS[kind] || []).map((p) => '<span class="port-chip">' + esc(p) + "</span>").join("");
  let footRight = "";
  if (kind === "Mailpit" && st === "Running")
    footRight =
      '<a class="btn-xs" href="http://localhost:8025" data-action="open-url" data-url="http://localhost:8025" rel="noreferrer">' + I.externalSm + "Open</a>";
  if (st === "Crashed")
    footRight = '<button class="btn-xs danger" data-action="svc-logs" data-kind="' + kind + '">' + I.warnSm + "View logs</button>";
  return (
    '<div class="card svc-card">' +
    '<div class="svc-top">' +
    '<div class="svc-tile">' + (SVC_ICON[kind] || "") + "</div>" +
    '<div class="svc-meta"><div class="svc-name">' + esc(DISP[kind]) + "</div>" +
    '<div class="svc-status"><span class="dot bgc-' + m.cls + dotPulse + '"></span>' +
    '<span class="txt s-' + m.cls + '">' + esc(m.label) + "</span></div></div>" +
    svcButton(kind, m) +
    "</div>" +
    '<div class="svc-foot">' + ports + '<span class="spacer"></span>' + footRight + "</div>" +
    "</div>"
  );
}

export function dashboard(): string {
  const run = runningCount();
  const allRunning = run === 5;
  const noneRunning = run === 0;
  const dots = SVC_KINDS.map((k) => {
    const cls = (META[state.services[k]] || META.Stopped).cls;
    return '<span class="bgc-' + cls + '" title="' + esc(DISP[k] + ": " + state.services[k]) + '"></span>';
  }).join("");
  const startBtn = state.startingAll
    ? '<button class="btn h36 btn-primary btn-busy" disabled>' + spinner("on-primary") + "Starting…</button>"
    : '<button class="btn h36 btn-primary' + (allRunning ? " btn-dim" : "") + '" data-action="start-all"' +
      (allRunning ? " disabled" : "") + ">" + I.play + "Start All</button>";
  const cards = SVC_KINDS.map(serviceCard).join("");
  const preview = state.sites
    .slice(0, 3)
    .map((s: any) => {
      const url = "https://" + s.hostname;
      return (
        '<div class="card site-row preview"><div class="site-tile">' + I.folder + "</div>" +
        '<div class="site-info"><div class="site-name">' + esc(s.name) + "</div>" +
        '<a class="site-url" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">' + esc(url) + "</a></div>" +
        '<a class="btn-sm" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">Open</a></div>'
      );
    })
    .join("");
  return (
    '<div class="view">' +
    "<div><h1 class=\"h1\">Dashboard</h1>" +
    '<p class="subtitle">Local stack · pretty HTTPS at <code class="chip-code">*.dev</code></p></div>' +
    '<div class="card summary">' +
    '<div class="big"><span class="num">' + run + '</span><span class="den">/ 5</span></div>' +
    '<div style="min-width:0"><div class="lbl">services running</div><div class="dots">' + dots + "</div></div>" +
    '<span class="spacer"></span><div class="actions">' + startBtn +
    '<button class="btn h36 btn-outline' + (noneRunning ? " btn-dim" : "") + '" data-action="stop-all"' +
    (noneRunning ? " disabled" : "") + ">" + I.stop + "Stop All</button></div></div>" +
    '<div class="row-between"><h2 class="section-label">Services</h2></div>' +
    '<div class="svc-grid">' + cards + "</div>" +
    '<div class="row-between mt4"><h2 class="section-label">Sites</h2>' +
    '<button class="link-btn" data-action="nav" data-view="sites">View all →</button></div>' +
    '<div class="stack-col">' + preview + "</div>" +
    "</div>"
  );
}

export async function startAll(): Promise<void> {
  if (state.busy || runningCount() === 5) return;
  state.busy = true;
  state.startingAll = true;
  state.pkexecMsg = "Authorize to update /etc/hosts — enter your password in the system prompt.";
  render();
  try {
    const arr = await invoke("stack_start_all");
    applyServices(arr);
    toast({ type: "success", title: "All services running", msg: "Sites are reachable at https://*.dev" });
  } catch (e) {
    toast({ type: "error", title: "Start failed", msg: String(e) });
  } finally {
    state.busy = false;
    state.startingAll = false;
    state.pkexecMsg = null;
    render();
  }
}

export async function stopAll(): Promise<void> {
  if (state.busy || runningCount() === 0) return;
  state.busy = true;
  for (const k of SVC_KINDS) if (state.services[k] === "Running") state.services[k] = "Stopping";
  render();
  try {
    const arr = await invoke("stack_stop_all");
    applyServices(arr);
    toast({ type: "info", title: "All services stopped" });
  } catch (e) {
    toast({ type: "error", title: "Stop failed", msg: String(e) });
  } finally {
    state.busy = false;
    render();
  }
}

export async function toggleService(kind: string): Promise<void> {
  if (state.busy) return;
  const running = state.services[kind] === "Running";
  const cmd = running ? "service_stop" : "service_start";
  state.busy = true;
  state.services[kind] = running ? "Stopping" : "Starting";
  render();
  try {
    const arr = await invoke(cmd, { kind });
    applyServices(arr);
    if (!running) toast({ type: "success", title: DISP[kind] + " started" });
  } catch (e) {
    toast({ type: "error", title: DISP[kind] + (running ? " stop failed" : " start failed"), msg: String(e) });
  } finally {
    state.busy = false;
    render();
  }
}

export function viewLogs(kind: string): void {
  const f = LOG_FILE[kind] || (kind.toLowerCase() + ".log");
  toast({
    type: "error",
    sticky: true,
    title: DISP[kind] + " crashed",
    details: ["Check ~/laralux/log/" + f, "or: journalctl --user -n 50"],
  });
}
