import { state } from "../../state";
import { esc } from "../util";
import { I } from "../icons";

export function deleteSiteModal(): string {
  const d = state.deleteSite;
  if (!d) return "";
  const busy = d.busy;
  const dis = busy ? " disabled" : "";
  const err = d.error ? '<div class="ns-error">' + esc(d.error) + "</div>" : "";

  const info =
    '<div class="ns-label">' + esc(d.name) + "</div>" +
    '<div class="ds-url">' + esc(d.url) + "</div>" +
    (d.source === "Proxy" ? "" : '<div class="ds-root" title="' + esc(d.root) + '">' + esc(d.root) + "</div>");

  let body: string;
  let footer: string;
  if (d.source === "Scanned") {
    body =
      "<p>This site is a folder in <code>www</code>.</p>" +
      "<p><b>Hide</b> keeps the files and removes it from Laralux (rename the folder back to restore). " +
      "<b>Delete</b> permanently removes the folder from disk.</p>";
    footer =
      '<button class="btn btn-outline" data-action="ds-close"' + dis + ">Cancel</button>" +
      '<button class="btn" data-action="ds-hide"' + dis + ">Hide</button>" +
      '<button class="btn btn-danger" data-action="ds-delete"' + dis + ">Delete</button>";
  } else if (d.source === "Linked") {
    body =
      "<p>Removes <b>" + esc(d.name) + "</b> from Laralux. Your project folder <code>" +
      esc(d.root) + "</code> is kept.</p>";
    footer =
      '<button class="btn btn-outline" data-action="ds-close"' + dis + ">Cancel</button>" +
      '<button class="btn btn-danger" data-action="ds-remove"' + dis + ">Remove</button>";
  } else {
    body = "<p>Removes the reverse-proxy <b>" + esc(d.name) + "</b> from Laralux.</p>";
    footer =
      '<button class="btn btn-outline" data-action="ds-close"' + dis + ">Cancel</button>" +
      '<button class="btn btn-danger" data-action="ds-remove"' + dis + ">Remove</button>";
  }

  const spin = busy ? '<span class="spin spinner"></span>' : "";
  return (
    '<div class="ns-overlay" data-action="ds-overlay-click" role="dialog" aria-modal="true" aria-labelledby="ds-title">' +
    '<div class="ns-card" role="document">' +
    '<div class="ns-head"><h2 class="ns-title" id="ds-title">Delete site</h2>' +
    '<button class="icon-btn" data-action="ds-close" aria-label="Close"' + dis + ">" + I.close + "</button></div>" +
    '<div class="ns-body">' + info + body + spin + err + "</div>" +
    '<div class="ns-foot">' + footer + "</div>" +
    "</div></div>"
  );
}
