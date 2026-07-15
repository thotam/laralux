/**
 * Typed wrappers over Tauri `invoke` — one per backend command.
 *
 * Command names and argument keys verified against:
 *   - src-tauri/src/commands.rs (#[tauri::command] fn signatures)
 *   - src/main.ts invoke("<cmd>", { ... }) call sites
 *
 * Return types mirror the Rust command return values.
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  ServiceStatus,
  ToolVersion,
  ComponentStatus,
  SetupReport,
  PhpIniSettings,
  Site,
  ProxyRoute,
  CreateReport,
  SetDomainsResult,
  ServicesFlags,
  SiteProcsView,
  LaunchConfig,
} from "./types";

// ---- stack / services -------------------------------------------------------

/** Snapshot of every registered service state. */
export const stackStatus = (): Promise<ServiceStatus[]> =>
  invoke<ServiceStatus[]>("stack_status");

/** Start all services (runs sync, certs, hosts). Returns updated snapshot. */
export const stackStartAll = (): Promise<ServiceStatus[]> =>
  invoke<ServiceStatus[]>("stack_start_all");

/** Stop all services. Returns updated snapshot. */
export const stackStopAll = (): Promise<ServiceStatus[]> =>
  invoke<ServiceStatus[]>("stack_stop_all");

/**
 * Start a single service by kind name.
 * Arg key: `kind` — verified: main.ts `invoke(cmd, { kind })` in toggleService().
 */
export const serviceStart = (kind: string): Promise<ServiceStatus[]> =>
  invoke<ServiceStatus[]>("service_start", { kind });

/**
 * Stop a single service by kind name.
 * Arg key: `kind` — verified: main.ts `invoke(cmd, { kind })` in toggleService().
 */
export const serviceStop = (kind: string): Promise<ServiceStatus[]> =>
  invoke<ServiceStatus[]>("service_stop", { kind });

/** Current per-service enable flags. */
export const serviceFlags = (): Promise<ServicesFlags> => invoke<ServicesFlags>("service_flags");

/** Enable/disable a service by ServiceKind ("Nginx" | "PhpFpm" | ... | "Postgres"). */
export const setServiceEnabled = (kind: string, enabled: boolean): Promise<unknown> =>
  invoke("set_service_enabled", { kind, enabled });

export const launchConfig = (): Promise<LaunchConfig> => invoke<LaunchConfig>("launch_config");

export const setLaunchOption = (key: string, enabled: boolean): Promise<LaunchConfig> =>
  invoke<LaunchConfig>("set_launch_option", { key, enabled });

// ---- sites ------------------------------------------------------------------

/** List all sites (scanned + linked + proxy). */
export const listSites = (): Promise<Site[]> =>
  invoke<Site[]>("list_sites");

/**
 * Create a new site from a template.
 * Arg keys: `name`, `template` — verified: main.ts `invoke("create_site", { name, template })`.
 */
export const createSite = (name: string, template: string): Promise<CreateReport> =>
  invoke<CreateReport>("create_site", { name, template });

/**
 * Link an existing folder as a site.
 * Arg keys: `name`, `root` — verified: main.ts `invoke("link_site", { name, root })`.
 */
export const linkSite = (name: string, root: string): Promise<Site> =>
  invoke<Site>("link_site", { name, root });

/**
 * Unlink (remove) a site from the registry.
 * Arg key: `name` — verified: main.ts `invoke("unlink_site", { name })`.
 */
export const unlinkSite = (name: string): Promise<void> =>
  invoke<void>("unlink_site", { name });

/** Hide a scanned site (rename its www folder to `.<name>`). */
export const hideSite = (name: string): Promise<void> =>
  invoke<void>("hide_site", { name });

/** Permanently delete a scanned site's www folder. */
export const deleteSiteFolder = (name: string): Promise<void> =>
  invoke<void>("delete_site_folder", { name });

/**
 * Add a reverse proxy site.
 * Arg keys: `name`, `routes`, `websocket` — verified: main.ts
 *   `invoke(cmd, { name: p.name, websocket: p.websocket, routes: p.routes.map(...) })`.
 */
export const addProxy = (
  name: string,
  routes: ProxyRoute[],
  websocket: boolean,
): Promise<Site> =>
  invoke<Site>("add_proxy", { name, routes, websocket });

/**
 * Update an existing reverse proxy site.
 * Arg keys: `name`, `routes`, `websocket` — same shape as addProxy, verified
 *   against main.ts submitProxy() where cmd is "update_proxy".
 */
export const updateProxy = (
  name: string,
  routes: ProxyRoute[],
  websocket: boolean,
): Promise<Site> =>
  invoke<Site>("update_proxy", { name, routes, websocket });

/**
 * Set custom domains for a site.
 * Arg keys: `name`, `domains` — verified: main.ts
 *   `invoke("set_site_domains", { name: sd.name, domains })`.
 * Returns { sites, warnings }.
 */
export const setSiteDomains = (
  name: string,
  domains: string[],
): Promise<SetDomainsResult> =>
  invoke<SetDomainsResult>("set_site_domains", { name, domains });

/**
 * Set public (real) domains for a site — reverse-proxied from an upstream
 * server that terminates public TLS (e.g. Let's Encrypt). Served locally on
 * both ports 80 and 443 (with the site's mkcert cert), not added to
 * /etc/hosts. Arg keys: `name`, `domains`.
 * Returns { sites, warnings }.
 */
export const setSitePublicDomains = (
  name: string,
  domains: string[],
): Promise<SetDomainsResult> =>
  invoke<SetDomainsResult>("set_site_public_domains", { name, domains });

// ---- terminal ---------------------------------------------------------------

/**
 * Open a terminal at the given directory path.
 * Arg key: `path` — verified: main.ts `invoke("open_terminal", { path })`.
 */
export const openTerminalAt = (path: string): Promise<void> =>
  invoke<void>("open_terminal", { path });

/**
 * Open the given directory path in the default file manager.
 * Arg key: `path` — mirrors openTerminalAt.
 */
export const openFolderAt = (path: string): Promise<void> =>
  invoke<void>("open_folder", { path });

/** Launch the Beekeeper DB client (downloads AppImage on first run). */
export const openDbClient = (): Promise<void> => invoke<void>("open_db_client");

// ---- setup ------------------------------------------------------------------

/** Detect which components are installed. */
export const setupStatus = (): Promise<ComponentStatus[]> =>
  invoke<ComponentStatus[]>("setup_status");

/** Run the full setup (download + install missing components). */
export const runSetupCmd = (): Promise<SetupReport> =>
  invoke<SetupReport>("run_setup_cmd");

// ---- tools ------------------------------------------------------------------

/**
 * List available versions for a tool.
 * Arg key: `tool` — verified: main.ts `invoke("tool_versions", { tool: toolKey })`.
 */
export const toolVersions = (tool: string): Promise<ToolVersion[]> =>
  invoke<ToolVersion[]>("tool_versions", { tool });

/**
 * Install a specific version of a tool.
 * Arg keys: `tool`, `version` — verified: main.ts
 *   `invoke("install_tool_version", { tool: tk, version })`.
 */
export const installToolVersion = (
  tool: string,
  version: string,
): Promise<ToolVersion[]> =>
  invoke<ToolVersion[]>("install_tool_version", { tool, version });

/**
 * Activate (switch to) a specific tool version.
 * Arg keys: `tool`, `version` — verified: main.ts
 *   `invoke("set_tool_version", { tool: tk, version })`.
 * Returns updated service snapshot (the version switch may restart a service).
 */
export const setToolVersion = (
  tool: string,
  version: string,
): Promise<ServiceStatus[]> =>
  invoke<ServiceStatus[]>("set_tool_version", { tool, version });

/** List tools that have their CLI symlinked into /usr/local/bin. */
export const toolSymlinks = (): Promise<string[]> =>
  invoke<string[]>("tool_symlinks");

/**
 * Enable or disable a tool's /usr/local/bin symlink.
 * Arg keys: `tool`, `enabled` — verified: main.ts
 *   `invoke("set_tool_symlink", { tool: tk, enabled: next })`.
 */
export const setToolSymlink = (
  tool: string,
  enabled: boolean,
): Promise<string[]> =>
  invoke<string[]>("set_tool_symlink", { tool, enabled });

// ---- site procs -------------------------------------------------------------

export const siteProcs = (name: string, root: string): Promise<SiteProcsView> =>
  invoke<SiteProcsView>("site_procs", { name, root });

export const startSiteProc = (name: string, root: string, proc: string): Promise<SiteProcsView> =>
  invoke<SiteProcsView>("start_site_proc", { name, root, proc });

export const stopSiteProc = (name: string, root: string, proc: string): Promise<SiteProcsView> =>
  invoke<SiteProcsView>("stop_site_proc", { name, root, proc });

export const startSiteProcs = (name: string, root: string): Promise<SiteProcsView> =>
  invoke<SiteProcsView>("start_site_procs", { name, root });

export const stopSiteProcs = (name: string, root: string): Promise<SiteProcsView> =>
  invoke<SiteProcsView>("stop_site_procs", { name, root });

export const setSiteAutostart = (name: string, enabled: boolean): Promise<boolean> =>
  invoke<boolean>("set_site_autostart", { name, enabled });

export const siteProcLogPath = (name: string, proc: string): Promise<string> =>
  invoke<string>("site_proc_log_path", { name, proc });

export const siteProcCounts = (): Promise<Record<string, number>> =>
  invoke<Record<string, number>>("site_proc_counts", {});

// ---- php.ini ----------------------------------------------------------------

/** Read the current PHP ini settings. */
export const phpIniSettings = (): Promise<PhpIniSettings> =>
  invoke<PhpIniSettings>("php_ini_settings");

/**
 * Persist and apply PHP ini settings.
 * Arg key: `settings` — verified: main.ts
 *   `invoke("set_php_ini_settings", { settings: payload })`.
 */
export const setPhpIniSettings = (
  settings: PhpIniSettings,
): Promise<PhpIniSettings> =>
  invoke<PhpIniSettings>("set_php_ini_settings", { settings });
