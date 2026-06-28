import { state } from "../../state";
import { esc } from "../util";
import { I } from "../icons";
import { toast } from "../toast";
import { invoke } from "../legacy-invoke";
import { render, applyComponents, resetDownload, progressRing } from "../render";
import { DISP_COMP, TOOL_KEY } from "../constants";

export function missingCount(): number {
  return state.setup.components.filter((c: any) => !c.present).length;
}

export function setupView(): string {
  const miss = missingCount();
  const subtitle = state.setup.phase === "done" ? "All components installed." : miss + " of " + state.setup.components.length + " components missing.";
  const items = state.setup.components
    .map((c: any) => {
      const tag = c.present
        ? '<span class="tag ok">Installed</span>'
        : '<span class="tag miss">Missing</span>';
      const tk = TOOL_KEY[c.component] || "";
      return (
        '<button class="setup-item setup-item-btn" data-action="open-tool" data-tool="' + esc(tk) + '">' +
        '<div class="setup-tile">' + I.setupItem + "</div>" +
        '<span class="nm">' + esc(DISP_COMP[c.component] || c.component) + "</span>" + tag +
        '<span class="chev">' + ((I as any).chevron || "›") + "</span></button>"
      );
    })
    .join("");

  let action = "";
  if (state.setup.phase === "idle") {
    action =
      '<div class="setup-idle"><div class="info"><div class="t">' + miss + " component(s) missing</div>" +
      '<div class="h">Downloads static binaries — may ask for your password for certificates &amp; bind permissions, and can take a few minutes.</div></div>' +
      '<button class="btn h36 btn-primary" data-action="run-setup" style="flex:none">' + I.download + "Install missing</button></div>";
  } else if (state.setup.phase === "installing") {
    action =
      '<div class="setup-installing">' + progressRing() +
      '<div class="info"><div class="t">Installing components…</div>' +
      '<div class="h">Downloading static binaries — this can take a minute. Don\'t close the window.</div></div></div>' +
      '<div class="auth-note">' + I.lock +
      '<span class="auth-tx">If a system password prompt appears, authorize it to finish setup (hosts, certificates &amp; bind permissions).</span></div>' +
      '<div class="progress"><div class="shim bar"></div></div>';
  } else {
    const rep: any = state.setup.report || {};
    const rows = [
      ["Mailpit binary", rep.mailpit_fetched ? "fetched" : "skipped"],
      ["mkcert local CA", rep.mkcert_ca ? "trusted" : "skipped"],
      ["Browser trust (NSS)", rep.mkcert_nss ? "trusted" : "skipped"],
      ["Nginx bind 80/443", rep.nginx_setcap ? "setcap ok" : "skipped"],
    ]
      .map(
        ([l, v]) =>
          '<div class="report-row">' + I.checkReport + '<span class="lbl">' + esc(l) + "</span>" +
          '<span class="spacer"></span><span class="val">' + esc(v) + "</span></div>"
      )
      .join("");
    const phpNotice = rep.php_version
      ? '<div class="notice-warn">' + I.clock + '<span class="t">PHP ' + esc(rep.php_version) + " installed — restart the app to apply.</span></div>"
      : "";
    action =
      '<div class="setup-done-head">' + I.checkDone + '<span class="t">Environment ready</span></div>' +
      '<div class="report-box">' + rows + "</div>" + phpNotice;
  }

  return (
    '<div class="view narrow">' +
    '<div><h1 class="h1">Setup</h1><p class="subtitle">' + esc(subtitle) + "</p></div>" +
    '<div class="card setup-card"><div class="setup-list">' + items + "</div>" +
    '<div class="setup-action">' + action + "</div></div></div>"
  );
}

export async function runSetup(): Promise<void> {
  if (state.busy || state.setup.phase === "installing") return;
  state.busy = true;
  state.setup.phase = "installing";
  // The auth prompt is shown inline in the install card (.auth-note), not the
  // global top banner — avoids the duplicate, prettier on the Setup tab.
  state.download.active = true; render();
  try {
    const report = await invoke("run_setup_cmd");
    state.setup.report = report;
    state.setup.phase = "done";
    if (report && report.errors && report.errors.length)
      toast({ type: "error", sticky: true, title: "Setup finished with errors", details: report.errors });
    else toast({ type: "success", title: "Environment ready", msg: "All components installed" });
    try {
      applyComponents(await invoke("setup_status"));
    } catch (_) {}
  } catch (e) {
    state.setup.phase = "idle";
    toast({ type: "error", title: "Setup failed", msg: String(e) });
  } finally {
    state.busy = false;
    state.pkexecMsg = null;
    resetDownload();
    render();
  }
}
