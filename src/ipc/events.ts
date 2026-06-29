/**
 * Typed wrappers over Tauri `listen` for the backend-emitted events.
 *
 * Event names verified against:
 *   - src-tauri/src/commands.rs (TauriProgress emits "download-progress")
 *   - src/main.ts boot section (TAURI.event.listen calls)
 *
 * Each function returns the Promise<UnlistenFn> from @tauri-apps/api/event,
 * which the caller stores and invokes to stop listening.
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ServiceStatus, ProgressPayload } from "./types";

/**
 * Subscribe to "services-changed" — emitted by the background monitor whenever
 * a service's state changes (process died → Crashed, external stop, etc.).
 * Payload: ServiceStatus[] (the full snapshot).
 * Verified: main.ts `TAURI.event.listen("services-changed", (e) => { applyServices(e.payload); ... })`.
 */
export const onServicesChanged = (
  cb: (statuses: ServiceStatus[]) => void,
): Promise<UnlistenFn> =>
  listen<ServiceStatus[]>("services-changed", (e) => cb(e.payload));

/**
 * Subscribe to "sites-changed" — emitted when the site list changes on disk
 * (e.g. a folder appears in www/).
 * No payload (the handler re-fetches via list_sites).
 * Verified: main.ts `TAURI.event.listen("sites-changed", () => { invoke("list_sites")... })`.
 */
export const onSitesChanged = (cb: () => void): Promise<UnlistenFn> =>
  listen("sites-changed", () => cb());

/**
 * Subscribe to "download-progress" — emitted by TauriProgress during
 * install/setup/create operations.
 * Payload: ProgressPayload (tagged with kind: "phase" | "step" | "bytes").
 * Verified: main.ts `TAURI.event.listen("download-progress", (e) => { applyProgress(e.payload); ... })`.
 */
export const onDownloadProgress = (
  cb: (payload: ProgressPayload) => void,
): Promise<UnlistenFn> =>
  listen<ProgressPayload>("download-progress", (e) => cb(e.payload));

/**
 * Subscribe to "site-procs-changed" — emitted (change-only) by the monitor when
 * any tracked per-site process changes state. Payload is empty; the handler
 * re-fetches the open Processes modal.
 */
export const onSiteProcsChanged = (cb: () => void): Promise<UnlistenFn> =>
  listen("site-procs-changed", () => cb());
