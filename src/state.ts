import { COMP_ORDER } from "./ui/constants";
import type { Site, ServiceState, ToolVersion, ComponentStatus, SetupReport, PhpIniSettings, ProxyRoute, SiteProcsView, LaunchConfig } from "./ipc/types";

// ---- Toast ------------------------------------------------------------------

export interface Toast {
  id: number;
  type: "success" | "error" | "info";
  title: string;
  msg?: string;
  sticky?: boolean;
  details?: string[];
}

// ---- Tool modal state -------------------------------------------------------

export interface ToolModalState {
  open: true;
  toolKey: string;
  display: string;
  cliBinary: string | null;
  versions: ToolVersion[];
  linked: boolean;
  busy: boolean;
  busyVersion: string | null;
  phpIni?: PhpIniSettings | null;
}

// ---- Sub-states -------------------------------------------------------------

export interface NewSiteState {
  name: string;
  template: "Blank" | "Laravel" | "Wordpress";
  busy: boolean;
  error: string;
}

export interface LinkSiteState {
  root: string;
  name: string;
  busy: boolean;
  error: string;
}

export interface ProxyState {
  mode: "create" | "edit";
  name: string;
  websocket: boolean;
  routes: ProxyRoute[];
  busy: boolean;
  error: string;
}

export interface SiteDomainsState {
  name: string;
  domains: string[];
  busy: boolean;
  error: string;
}

export interface DownloadState {
  active: boolean;
  label: string;
  step: { done: number; total: number };
  bytes: { current: number; total: number };
  overall: number;
}

export interface SetupState {
  phase: "idle" | "installing" | "done";
  report: SetupReport | null;
  components: ComponentStatus[];
}

// ---- AppState ---------------------------------------------------------------

export interface AppState {
  view: string;
  dark: boolean;
  compact: boolean;
  /** Keyed by ServiceKind string (e.g. "Nginx", "PhpFpm", …) */
  services: Record<string, ServiceState>;
  serviceFlags: Record<string, boolean>;
  launch: LaunchConfig;
  sites: Site[];
  setup: SetupState;
  pkexecMsg: string | null;
  startingAll: boolean;
  busy: boolean;
  toasts: Toast[];
  tId: number;
  /**
   * null         — no modal open
   * "newsite"    — New Site modal
   * "linksite"   — Link Site modal
   * "proxy"      — Reverse Proxy modal
   * "domains"    — Edit Domains modal
   * "publicdomains" — Public Domains modal
   * ToolModalState — Tool detail modal (open: true)
   */
  modal: null | "newsite" | "linksite" | "proxy" | "domains" | "publicdomains" | "deletesite" | "procs" | ToolModalState;
  procCounts: Record<string, number>;
  procModal: { name: string; root: string } | null;
  siteProcs: SiteProcsView | null;
  toolSymlinks: string[];
  newSite: NewSiteState;
  linkSite: LinkSiteState;
  deleteSite: null | {
    name: string;
    source: "Scanned" | "Linked" | "Proxy";
    root: string;
    url: string;
    busy: boolean;
    error: string;
  };
  rowMenu: string | null;
  proxy: ProxyState;
  siteDomains: SiteDomainsState;
  sitePublicDomains: SiteDomainsState;
  download: DownloadState;
  dbClientBusy: boolean;
}

// ---- initial state ----------------------------------------------------------

const stored = localStorage.getItem("laralux-theme");
const prefersDark =
  window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;

export const state: AppState = {
  view: "dashboard",
  dark: stored ? stored === "dark" : !!prefersDark,
  compact: false,
  services: { Nginx: "Stopped", PhpFpm: "Stopped", Mariadb: "Stopped", Postgres: "Stopped", Mongodb: "Stopped", Redis: "Stopped", Mailpit: "Stopped" },
  serviceFlags: { nginx: true, php: true, mariadb: true, redis: true, mailpit: true, postgres: false, mongodb: false },
  launch: { start_on_login: false, start_minimized: false, autostart_services: false },
  sites: [],
  setup: { phase: "idle", report: null, components: COMP_ORDER.map((c) => ({ component: c, present: false })) },
  pkexecMsg: null,
  startingAll: false,
  busy: false,
  toasts: [],
  tId: 1,
  modal: null,
  procCounts: {},
  procModal: null,
  siteProcs: null,
  toolSymlinks: [],
  newSite: { name: "", template: "Blank", busy: false, error: "" },
  linkSite: { root: "", name: "", busy: false, error: "" },
  deleteSite: null,
  rowMenu: null,
  proxy: { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], busy: false, error: "" },
  siteDomains: { name: "", domains: [""], busy: false, error: "" },
  sitePublicDomains: { name: "", domains: [""], busy: false, error: "" },
  download: { active: false, label: "", step: { done: 0, total: 0 }, bytes: { current: 0, total: 0 }, overall: 0 },
  dbClientBusy: false,
};
