import { state } from "../../state";
import { esc, validName } from "../util";
import { I } from "../icons";
import { toast } from "../toast";
import { invoke } from "../legacy-invoke";
import { render } from "../loop";

const DOMAIN_RE = /^(\*\.)?([a-z0-9]([a-z0-9-]*[a-z0-9])?\.)+[a-z0-9]([a-z0-9-]*[a-z0-9])?$/;
function validDomain(d: string): boolean {
  if (!DOMAIN_RE.test(d)) return false;
  return d.replace(/^\*\./, "").split(".").every((l) => l.length <= 63);
}

function deriveName(path: string): string {
  const base = (path || "").replace(/[\\/]+$/, "").split(/[\\/]/).pop() || "";
  return base.toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 63);
}

export function sitesView(): string {
  const empty = state.sites.length === 0;
  const head =
    '<div class="sites-head"><div><h1 class="h1">Sites</h1>' +
    '<p class="subtitle">Projects under <code class="chip-code">~/laralux/www</code></p></div>' +
    '<div class="sites-actions">' +
    '<button class="btn-newsite ghost" data-action="proxy-site">' + I.navSites + "Reverse proxy</button>" +
    '<button class="btn-newsite ghost" data-action="link-site">' + I.folder18 + "Add existing folder</button>" +
    '<button class="btn-newsite" data-action="new-site">' + I.plus + "New site</button></div></div>";
  let bodyHtml: string;
  if (empty) {
    bodyHtml =
      '<div class="sites-empty"><div class="glyph">' + I.folderBig + "</div>" +
      '<div class="t">No sites yet</div>' +
      '<div class="h">Drop a project folder into <code class="chip-code">~/laralux/www</code> and it gets a pretty <code class="chip-code">https://&lt;name&gt;.dev</code> URL automatically.</div>' +
      '<button class="btn-newsite" data-action="new-site" style="margin-top:4px">' + I.plus + "New site</button></div>";
  } else {
    bodyHtml =
      '<div class="stack-col g9">' +
      state.sites
        .map((s: any) => {
          const url = "https://" + s.hostname;
          const isProxy = s.source === "Proxy";
          const isLinked = s.source === "Linked";
          const target = isProxy && s.proxy && s.proxy.routes && s.proxy.routes.length
            ? s.proxy.routes[0].upstream + (s.proxy.routes.length > 1 ? " +" + (s.proxy.routes.length - 1) : "")
            : "";
          const badge = isProxy
            ? '<span class="site-badge">proxy → ' + esc(target) + "</span>"
            : (isLinked ? '<span class="site-badge">linked</span>' : "");
          const subRight = isProxy ? "" : '<span class="site-root" title="' + esc(s.root) + '">' + esc(s.root) + "</span>";
          const editBtn = isProxy
            ? '<button class="btn-sm" data-action="edit-proxy" data-name="' + esc(s.name) + '">Edit</button>'
            : "";
          const domBtn = '<button class="btn-sm" data-action="edit-domains" data-name="' + esc(s.name) + '">Domains</button>';
          const removeBtn = (isProxy || isLinked)
            ? '<button class="btn-sm danger" data-action="remove-site" data-name="' + esc(s.name) + '">' +
              (state.confirmRemove === s.name ? "Confirm?" : "Remove") + "</button>"
            : "";
          const termBtn = isProxy
            ? ""
            : '<button class="icon-btn sq32" data-action="open-terminal" data-path="' + esc(s.root) + '" aria-label="Open terminal" title="Open terminal here">' + I.terminal + "</button>";
          return (
            '<div class="card site-row"><div class="site-tile">' + I.folder18 + "</div>" +
            '<div class="site-info"><div class="site-name">' + esc(s.name) + "</div>" +
            '<div class="site-sub"><a class="site-url" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">' + esc(url) + "</a>" +
            subRight + "</div></div>" +
            badge +
            termBtn +
            '<button class="icon-btn sq32" data-action="copy-site" data-name="' + esc(s.name) + '" aria-label="Copy URL">' + I.copy + "</button>" +
            editBtn + domBtn + removeBtn +
            '<a class="btn-sm" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">' + I.external + "Open</a></div>"
          );
        })
        .join("") +
      "</div>";
  }
  return '<div class="view">' + head + bodyHtml + "</div>";
}

export function openNewSite(): void {
  state.modal = "newsite";
  state.newSite = { name: "", template: "Blank", busy: false, error: "" };
  render();
  requestAnimationFrame(() => {
    const inp = document.getElementById("ns-name");
    if (inp) inp.focus();
  });
}

export function closeNewSite(): void {
  if (state.newSite.busy) return;
  state.modal = null;
  render();
}

export async function submitNewSite(): Promise<void> {
  const { name, template } = state.newSite;
  if (!validName(name)) { state.newSite.error = "Use lowercase letters, digits, hyphens (e.g. my-app)"; render(); return; }
  state.newSite.busy = true; state.newSite.error = ""; render();
  state.download.active = true; state.download.label = "Creating site…"; render();
  try {
    const rep = await invoke("create_site", { name, template });
    const extra = rep.database_created ? " · database created" : "";
    const warn = rep.warnings && rep.warnings.length ? { details: rep.warnings } : {};
    toast({ type: "success", title: "Created " + rep.site_name, msg: "https://" + rep.hostname + extra, ...warn });
    state.modal = null;
    state.newSite = { name: "", template: "Blank", busy: false, error: "" };
    try {
      const sites = await invoke("list_sites");
      state.sites = Array.isArray(sites) ? sites : [];
    } catch (_) {}
    render();
  } catch (e) {
    state.newSite.error = String(e);
    state.newSite.busy = false;
    resetDownload();
    toast({ type: "error", title: "Create failed", msg: String(e) });
    render();
  } finally {
    if (state.newSite.busy) { state.newSite.busy = false; resetDownload(); render(); }
  }
}

export function openLinkSite(): void {
  state.modal = "linksite";
  state.linkSite = { root: "", name: "", busy: false, error: "" };
  render();
  requestAnimationFrame(() => {
    const inp = document.getElementById("ls-name");
    if (inp) inp.focus();
  });
}

export function closeLinkSite(): void {
  if (state.linkSite.busy) return;
  state.modal = null;
  render();
}

export async function browseFolder(): Promise<void> {
  try {
    const dlg = (window as any).__TAURI__ && (window as any).__TAURI__.dialog;
    if (!dlg) { toast({ type: "error", title: "Folder picker unavailable" }); return; }
    const picked = await dlg.open({ directory: true, multiple: false, title: "Choose project folder" });
    if (!picked) return; // cancelled
    const path: string = Array.isArray(picked) ? picked[0] : picked;
    state.linkSite.root = path;
    if (!state.linkSite.name) state.linkSite.name = deriveName(path);
    state.linkSite.error = "";
    render();
  } catch (e) {
    toast({ type: "error", title: "Folder picker failed", msg: String(e) });
  }
}

export async function submitLinkSite(): Promise<void> {
  const { root, name } = state.linkSite;
  if (!root) { state.linkSite.error = "Choose a folder first"; render(); return; }
  if (!validName(name)) { state.linkSite.error = "Use lowercase letters, digits, hyphens (e.g. my-app)"; render(); return; }
  state.linkSite.busy = true; state.linkSite.error = ""; render();
  try {
    const site = await invoke("link_site", { name, root });
    toast({ type: "success", title: "Linked " + site.name, msg: "https://" + site.hostname });
    state.modal = null;
    state.linkSite = { root: "", name: "", busy: false, error: "" };
    try {
      const sites = await invoke("list_sites");
      state.sites = Array.isArray(sites) ? sites : [];
    } catch (_) {}
    render();
  } catch (e) {
    state.linkSite.error = String(e);
    state.linkSite.busy = false;
    toast({ type: "error", title: "Link failed", msg: String(e) });
    render();
  } finally {
    if (state.linkSite.busy) { state.linkSite.busy = false; render(); }
  }
}

export function openProxy(site?: any): void {
  if (site && site.proxy) {
    state.proxy = {
      mode: "edit", name: site.name, websocket: !!site.proxy.websocket,
      routes: (site.proxy.routes || []).map((r: any) => ({ path: r.path, upstream: r.upstream })),
      busy: false, error: "",
    };
    if (!state.proxy.routes.length) state.proxy.routes = [{ path: "/", upstream: "" }];
  } else {
    state.proxy = { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], busy: false, error: "" };
  }
  state.modal = "proxy";
  render();
  requestAnimationFrame(() => { const inp = document.getElementById("px-name"); if (inp && !(inp as any).readOnly) inp.focus(); });
}

export function closeProxy(): void {
  if (state.proxy.busy) return;
  state.modal = null;
  render();
}

export function addProxyRoute(): void { state.proxy.routes.push({ path: "", upstream: "" }); render(); }
export function delProxyRoute(i: number): void { state.proxy.routes.splice(i, 1); if (!state.proxy.routes.length) state.proxy.routes.push({ path: "/", upstream: "" }); render(); }

export async function submitProxy(): Promise<void> {
  const p = state.proxy;
  if (!validName(p.name)) { p.error = "Use lowercase letters, digits, hyphens (e.g. my-app)"; render(); return; }
  if (!p.routes.length) { p.error = "Add at least one route"; render(); return; }
  for (const r of p.routes) {
    if (!r.path.startsWith("/")) { p.error = "Each path must start with /"; render(); return; }
    if (!String(r.upstream).trim()) { p.error = "Each route needs a target (host:port)"; render(); return; }
  }
  p.busy = true; p.error = ""; render();
  try {
    const cmd = p.mode === "edit" ? "update_proxy" : "add_proxy";
    const site = await invoke(cmd, {
      name: p.name, websocket: p.websocket,
      routes: p.routes.map((r: any) => ({ path: r.path, upstream: r.upstream })),
    });
    toast({ type: "success", title: (p.mode === "edit" ? "Updated " : "Proxy ") + site.name, msg: "https://" + site.hostname });
    state.modal = null;
    state.proxy = { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], busy: false, error: "" };
    try { const sites = await invoke("list_sites"); state.sites = Array.isArray(sites) ? sites : []; } catch (_) {}
    render();
  } catch (e) {
    p.error = String(e); p.busy = false;
    toast({ type: "error", title: "Proxy failed", msg: String(e) });
    render();
  } finally {
    if (p.busy) { p.busy = false; render(); }
  }
}

export function openDomains(site: any): void {
  const ds = (site.domains && site.domains.length) ? site.domains.slice() : [site.hostname];
  state.siteDomains = { name: site.name, domains: ds, busy: false, error: "" };
  state.modal = "domains";
  render();
}
export function closeDomains(): void { if (state.siteDomains.busy) return; state.modal = null; render(); }
export function addDomainRow(): void { state.siteDomains.domains.push(""); render(); }
export function delDomainRow(i: number): void { state.siteDomains.domains.splice(i, 1); if (!state.siteDomains.domains.length) state.siteDomains.domains.push(""); render(); }

export async function submitDomains(): Promise<void> {
  const sd = state.siteDomains;
  const domains = sd.domains.map((d: string) => d.trim()).filter((d: string) => d.length);
  if (!domains.length) { sd.error = "Add at least one domain"; render(); return; }
  for (const d of domains) { if (!validDomain(d)) { sd.error = "Invalid domain: " + d; render(); return; } }
  sd.busy = true; sd.error = ""; render();
  try {
    const res = await invoke("set_site_domains", { name: sd.name, domains });
    state.sites = Array.isArray(res && res.sites) ? res.sites : [];
    toast({ type: "success", title: "Domains updated", msg: domains.join(", ") });
    if (res && Array.isArray(res.warnings)) {
      for (const w of res.warnings) toast({ type: "error", title: "Wildcard DNS", msg: String(w) });
    }
    state.modal = null; render();
  } catch (e) {
    sd.error = String(e); sd.busy = false;
    toast({ type: "error", title: "Update failed", msg: String(e) });
    render();
  } finally {
    if (sd.busy) { sd.busy = false; render(); }
  }
}

export async function removeSite(name: string): Promise<void> {
  if (state.confirmRemove !== name) { state.confirmRemove = name; render(); return; }
  state.confirmRemove = null;
  try {
    await invoke("unlink_site", { name });
    toast({ type: "success", title: "Removed " + name });
    const sites = await invoke("list_sites");
    state.sites = Array.isArray(sites) ? sites : [];
    render();
  } catch (e) {
    toast({ type: "error", title: "Remove failed", msg: String(e) });
    render();
  }
}

export async function copySite(name: string): Promise<void> {
  const url = "https://" + name + ".dev";
  try {
    await navigator.clipboard.writeText(url);
    toast({ type: "success", title: "Copied " + url });
  } catch (e) {
    toast({ type: "error", title: "Copy failed", msg: url });
  }
}

export async function openTerminal(path: string): Promise<void> {
  try {
    await invoke("open_terminal", { path });
  } catch (e) {
    toast({ type: "error", title: "Couldn't open terminal", msg: String(e) });
  }
}

// Open a URL in the system browser. In the Tauri webview, <a target="_blank">
// does nothing, so route through the opener plugin (fallback to window.open for dev).
export async function openExternal(url: string): Promise<void> {
  if (!url) return;
  try {
    const op = (window as any).__TAURI__ && (window as any).__TAURI__.opener;
    if (op && op.openUrl) { await op.openUrl(url); return; }
    window.open(url, "_blank");
  } catch (e) {
    toast({ type: "error", title: "Couldn't open", msg: String(e) });
  }
}

// Helper used by submitNewSite/submitLinkSite — mirrors the one in main.ts
function resetDownload(): void {
  state.download = { active: false, label: "", step: { done: 0, total: 0 }, bytes: { current: 0, total: 0 }, overall: 0 };
}
