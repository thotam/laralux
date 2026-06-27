// The component order is duplicated here (same value as COMP_ORDER in main.ts)
// solely to seed the default state; main.ts owns the authoritative copy used
// by applyComponents() and setupView().
const _COMP_ORDER = ["Nginx", "Php", "Mariadb", "Redis", "Mkcert", "Mailpit", "Composer", "Node"];

const stored = localStorage.getItem("laralux-theme");
const prefersDark =
  window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;

export const state: any = {
  view: "dashboard",
  dark: stored ? stored === "dark" : !!prefersDark,
  compact: false,
  services: { Nginx: "Stopped", PhpFpm: "Stopped", Mariadb: "Stopped", Redis: "Stopped", Mailpit: "Stopped" },
  sites: [],
  setup: { phase: "idle", report: null, components: _COMP_ORDER.map((c) => ({ component: c, present: false })) },
  pkexecMsg: null,
  startingAll: false,
  busy: false,
  toasts: [],
  tId: 1,
  modal: null,
  toolSymlinks: [],
  newSite: { name: "", template: "Blank", busy: false, error: "" },
  linkSite: { root: "", name: "", busy: false, error: "" },
  confirmRemove: null,
  proxy: { mode: "create", name: "", websocket: true, routes: [{ path: "/", upstream: "" }], busy: false, error: "" },
  siteDomains: { name: "", domains: [""], busy: false, error: "" },
  download: { active: false, label: "", step: { done: 0, total: 0 }, bytes: { current: 0, total: 0 }, overall: 0 },
};
