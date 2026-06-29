import { state } from "../../state";
import type { Site } from "../../ipc/types";
import { esc, validName } from "../util";
import { I } from "../icons";
import { toast } from "../toast";
import {
  createSite, listSites, linkSite,
  addProxy, updateProxy, setSiteDomains, openTerminalAt, openFolderAt,
} from "../../ipc/commands";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { render, resetDownload } from "../render";

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
        .map((s) => {
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
          const folderBtn = isProxy
            ? ""
            : '<button class="icon-btn sq32" data-action="open-folder" data-path="' + esc(s.root) + '" aria-label="Open folder" title="Open project folder">' + I.folder + "</button>";
          const termBtn = isProxy
            ? ""
            : '<button class="icon-btn sq32" data-action="open-terminal" data-path="' + esc(s.root) + '" aria-label="Open terminal" title="Open terminal here">' + I.terminal + "</button>";

          const menuItems =
            '<button class="row-menu-item" data-action="copy-site" data-name="' + esc(s.name) + '">' + I.copy + "Copy URL</button>" +
            '<button class="row-menu-item" data-action="edit-domains" data-name="' + esc(s.name) + '">Domains</button>' +
            (isProxy ? '<button class="row-menu-item" data-action="edit-proxy" data-name="' + esc(s.name) + '">Edit proxy</button>' : "") +
            (state.procCounts[s.name] ? '<button class="row-menu-item" data-action="open-procs" data-name="' + esc(s.name) + '" data-root="' + esc(s.root) + '">' + I.terminal + "Processes</button>" : "") +
            '<button class="row-menu-item danger" data-action="delete-site" data-name="' + esc(s.name) + '">Delete</button>';
          const menu = state.rowMenu === s.name ? '<div class="row-menu">' + menuItems + "</div>" : "";

          const actions =
            '<div class="row-actions">' +
            termBtn + folderBtn +
            '<a class="btn-sm" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">' + I.external + "Open</a>" +
            '<button class="icon-btn sq32" data-action="row-menu" data-name="' + esc(s.name) + '" aria-label="More actions" title="More">' + I.kebab + "</button>" +
            menu +
            "</div>";

          return (
            '<div class="card site-row" data-key="site-' + esc(s.name) + '"><div class="site-tile">' + I.folder18 + "</div>" +
            '<div class="site-info"><div class="site-name">' + esc(s.name) + "</div>" +
            '<div class="site-sub"><a class="site-url" href="' + esc(url) + '" data-action="open-url" data-url="' + esc(url) + '" rel="noreferrer">' + esc(url) + "</a>" +
            (state.procCounts[s.name] ? '<span class="proc-chip" title="' + state.procCounts[s.name] + ' process(es) in Procfile">⚙ ' + state.procCounts[s.name] + "</span>" : "") +
            subRight + "</div></div>" +
            badge +
            actions +
            "</div>"
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
    const rep = await createSite(name, template);
    const extra = rep.database_created ? " · database created" : "";
    const warn = rep.warnings && rep.warnings.length ? { details: rep.warnings } : {};
    toast({ type: "success", title: "Created " + rep.site_name, msg: "https://" + rep.hostname + extra, ...warn });
    state.modal = null;
    state.newSite = { name: "", template: "Blank", busy: false, error: "" };
    try {
      const sites = await listSites();
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
    const picked = await openDialog({ directory: true, multiple: false, title: "Choose project folder" });
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
    const site = await linkSite(name, root);
    toast({ type: "success", title: "Linked " + site.name, msg: "https://" + site.hostname });
    state.modal = null;
    state.linkSite = { root: "", name: "", busy: false, error: "" };
    try {
      const sites = await listSites();
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

export function openProxy(site?: Site): void {
  if (site && site.proxy) {
    state.proxy = {
      mode: "edit", name: site.name, websocket: !!site.proxy.websocket,
      routes: (site.proxy.routes || []).map((r) => ({ path: r.path, upstream: r.upstream })),
      busy: false, error: "",
    };
    if (!state.proxy.routes.length) state.proxy.routes = [{ path: "/", upstream: "" }];
  } else {
    state.proxy = { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], busy: false, error: "" };
  }
  state.modal = "proxy";
  render();
  requestAnimationFrame(() => { const inp = document.getElementById("px-name") as HTMLInputElement | null; if (inp && !inp.readOnly) inp.focus(); });
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
    const routes = p.routes.map((r) => ({ path: r.path, upstream: r.upstream }));
    const site = await (p.mode === "edit"
      ? updateProxy(p.name, routes, p.websocket)
      : addProxy(p.name, routes, p.websocket));
    toast({ type: "success", title: (p.mode === "edit" ? "Updated " : "Proxy ") + site.name, msg: "https://" + site.hostname });
    state.modal = null;
    state.proxy = { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], busy: false, error: "" };
    try { const sites = await listSites(); state.sites = Array.isArray(sites) ? sites : []; } catch (_) {}
    render();
  } catch (e) {
    p.error = String(e); p.busy = false;
    toast({ type: "error", title: "Proxy failed", msg: String(e) });
    render();
  } finally {
    if (p.busy) { p.busy = false; render(); }
  }
}

export function openDomains(site: Site): void {
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
    const res = await setSiteDomains(sd.name, domains);
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

export function openDeleteSite(name: string): void {
  const s = state.sites.find((x) => x.name === name);
  if (!s) return;
  state.deleteSite = {
    name: s.name,
    source: s.source as "Scanned" | "Linked" | "Proxy",
    root: s.root,
    url: "https://" + s.hostname,
    busy: false,
    error: "",
  };
  state.rowMenu = null;
  state.modal = "deletesite";
  render();
}

export function closeDeleteSite(): void {
  if (state.deleteSite && state.deleteSite.busy) return;
  state.modal = null;
  state.deleteSite = null;
  render();
}

export async function runDeleteAction(fn: (name: string) => Promise<void>): Promise<void> {
  const d = state.deleteSite;
  if (!d || d.busy) return;
  d.busy = true;
  d.error = "";
  render();
  try {
    await fn(d.name);
    toast({ type: "success", title: "Deleted " + d.name });
    const sites = await listSites();
    state.sites = Array.isArray(sites) ? sites : [];
    state.modal = null;
    state.deleteSite = null;
    render();
  } catch (e) {
    if (state.deleteSite) {
      state.deleteSite.busy = false;
      state.deleteSite.error = String(e);
    }
    toast({ type: "error", title: "Delete failed", msg: String(e) });
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
    await openTerminalAt(path);
  } catch (e) {
    toast({ type: "error", title: "Couldn't open terminal", msg: String(e) });
  }
}

export async function openFolder(path: string): Promise<void> {
  try {
    await openFolderAt(path);
  } catch (e) {
    toast({ type: "error", title: "Couldn't open folder", msg: String(e) });
  }
}

// Open a URL in the system browser. In the Tauri webview, <a target="_blank">
// does nothing, so route through the opener plugin.
export async function openExternal(url: string): Promise<void> {
  if (!url) return;
  try {
    await openUrl(url);
  } catch (e) {
    toast({ type: "error", title: "Couldn't open", msg: String(e) });
  }
}

