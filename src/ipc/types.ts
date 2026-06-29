/**
 * Typed interfaces mirroring the Rust serde structs.
 * Source of truth: core/src/ (service/mod.rs, orchestrator.rs, tools.rs,
 * setup.rs, php_ini.rs, sites.rs, site_registry.rs, scaffold.rs, progress.rs,
 * src-tauri/src/commands.rs).
 *
 * Field names verified against src/main.ts usage throughout.
 */

// ---- service ----------------------------------------------------------------

/**
 * ServiceState — mirrors core/src/service/mod.rs `ServiceState`.
 * UI reads: s.state === "Running" | "Stopped" | "Starting" | "Stopping" | "Crashed"
 * (META object in main.ts, applyServices, toggleService, stopAll).
 */
export type ServiceState = "Stopped" | "Starting" | "Running" | "Stopping" | "Crashed";

export interface ServicesFlags {
  nginx: boolean; php: boolean; mariadb: boolean; redis: boolean; mailpit: boolean; postgres: boolean; mongodb: boolean;
}

/**
 * ServiceStatus — mirrors core/src/orchestrator.rs `ServiceStatus` + ServiceKind.
 * Serializes as { kind: "Nginx" | "PhpFpm" | ..., state: ServiceState }.
 * UI reads: s.kind (keyed into state.services), s.state.
 * Verified: applyServices() in main.ts: `s.kind in state.services`, `s.state`.
 * ServiceKind variants from core/src/service/mod.rs.
 */
export interface ServiceStatus {
  kind: "Nginx" | "PhpFpm" | "Mariadb" | "Postgres" | "Mongodb" | "Redis" | "Mailpit" | "Coredns";
  state: ServiceState;
}

// ---- tools ------------------------------------------------------------------

/**
 * ToolVersion — mirrors core/src/tools.rs `ToolVersion`.
 * UI reads: v.version, v.installed, v.active (toolModal() in main.ts).
 */
export interface ToolVersion {
  version: string;
  installed: boolean;
  active: boolean;
}

// ---- setup ------------------------------------------------------------------

/**
 * ComponentStatus — mirrors core/src/setup.rs `ComponentStatus`.
 * UI reads: c.component (string name), c.present (applyComponents in main.ts).
 * Note: Component enum serializes as its variant name string (Nginx, Php, etc.).
 */
export interface ComponentStatus {
  component: string;
  present: boolean;
}

/**
 * SetupReport — mirrors core/src/setup.rs `SetupReport`.
 * UI reads: rep.mailpit_fetched, rep.mkcert_ca, rep.mkcert_nss, rep.nginx_setcap,
 *           rep.php_version, rep.errors (setupView() in main.ts).
 */
export interface SetupReport {
  apt_packages: string[];
  mailpit_fetched: boolean;
  composer_fetched: boolean;
  nginx_fetched: boolean;
  redis_fetched: boolean;
  mkcert_fetched: boolean;
  mkcert_ca: boolean;
  certutil_fetched: boolean;
  mkcert_nss: boolean;
  nginx_setcap: boolean;
  mariadb_fetched: boolean;
  node_fetched: boolean;
  php_version: string | null;
  errors: string[];
}

// ---- php ini ----------------------------------------------------------------

/**
 * PhpIniSettings — mirrors core/src/php_ini.rs `PhpIniSettings`.
 * UI reads all fields (toolModal phpSettings section in main.ts):
 *   pi.memory_limit, pi.upload_max_filesize, pi.post_max_size,
 *   pi.max_execution_time, pi.timezone, pi.display_errors, pi.opcache_enable.
 * Note: max_execution_time is u32 in Rust (maps to number in TS).
 */
export interface PhpIniSettings {
  memory_limit: string;
  upload_max_filesize: string;
  post_max_size: string;
  max_execution_time: number;
  timezone: string;
  display_errors: boolean;
  opcache_enable: boolean;
}

// ---- sites ------------------------------------------------------------------

/**
 * ProxyRoute — mirrors core/src/site_registry.rs `ProxyRoute`.
 * UI reads: r.path, r.upstream (openProxy, proxyModal, submitProxy in main.ts).
 */
export interface ProxyRoute {
  path: string;
  upstream: string;
}

/**
 * ProxySpec — mirrors core/src/sites.rs `ProxySpec`.
 * UI reads: site.proxy.routes (array of ProxyRoute), site.proxy.websocket.
 * Verified: openProxy() reads site.proxy.routes, site.proxy.websocket.
 */
export interface ProxySpec {
  routes: ProxyRoute[];
  websocket: boolean;
}

/**
 * Site — mirrors core/src/sites.rs `Site`.
 * UI reads:
 *   s.name, s.hostname, s.root (sitesView, dashboard preview),
 *   s.source ("Proxy" | "Linked" | "Scanned"),
 *   s.proxy (ProxySpec | null),
 *   s.domains (openDomains, set_site_domains).
 */
export interface Site {
  name: string;
  root: string;
  hostname: string;
  domains: string[];
  source: "Scanned" | "Linked" | "Proxy";
  proxy: ProxySpec | null;
}

/**
 * SiteDomains — mirrors core/src/site_registry.rs `SiteDomains`.
 * Used internally by set_site_domains; not directly consumed by the frontend
 * (the command returns SetDomainsResult which wraps Site[]).
 */
export interface SiteDomains {
  name: string;
  domains: string[];
}

/**
 * CreateReport — mirrors core/src/scaffold.rs `CreateReport`.
 * Returned by create_site command.
 * UI reads: rep.database_created, rep.warnings, rep.site_name, rep.hostname, rep.template.
 */
export interface CreateReport {
  site_name: string;
  hostname: string;
  template: "Blank" | "Laravel" | "Wordpress";
  database_created: boolean;
  warnings: string[];
}

/**
 * SetDomainsResult — mirrors src-tauri/src/commands.rs `SetDomainsResult`.
 * Returned by set_site_domains command.
 * UI reads: res.sites (array), res.warnings (array).
 */
export interface SetDomainsResult {
  sites: Site[];
  warnings: string[];
}

// ---- progress ---------------------------------------------------------------

/**
 * ProgressPayload — mirrors core/src/progress.rs `ProgressEvent` (serde tagged).
 * Serializes with `#[serde(tag = "kind", rename_all = "lowercase")]`.
 * UI reads: p.kind ("phase" | "step" | "bytes"), p.label, p.done, p.total, p.current.
 * Verified: applyProgress() in main.ts.
 */
export interface ProgressPayload {
  kind: "phase" | "step" | "bytes";
  label?: string;
  done?: number;
  total?: number;
  current?: number;
}
