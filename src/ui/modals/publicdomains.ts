import { state } from "../../state";
import { esc } from "../util";
import { I } from "../icons";

export function publicDomainsModal(): string {
  const sd = state.sitePublicDomains;
  const hasAny = sd.domains.some((d: string) => d.trim().length > 0);
  const errorHtml = sd.error ? '<div class="ns-error">' + esc(sd.error) + '</div>' : '';
  const d = sd.busy ? ' disabled' : '';
  const rows = sd.domains.map((v: string, i: number) =>
    '<div class="pr-row" data-key="pdom-' + i + '">' +
    '<input class="ns-input" type="text" placeholder="app.example.com" value="' + esc(v) + '" autocomplete="off" spellcheck="false" data-action="pd-input" data-idx="' + i + '"' + d + ' />' +
    (sd.domains.length > 1 ? '<button class="icon-btn sq32" data-action="pd-del" data-idx="' + i + '" aria-label="Remove domain"' + d + '>' + I.close + '</button>' : '') +
    '</div>'
  ).join('');
  const submitLabel = sd.busy
    ? '<span class="spin spinner on-primary"></span>Saving…'
    : 'Save';
  return (
    '<div class="ns-overlay" data-action="pd-overlay-click" role="dialog" aria-modal="true" aria-labelledby="pd-title">' +
    '<div class="ns-card" role="document">' +
    '<div class="ns-head"><h2 class="ns-title" id="pd-title">Public domains — ' + esc(sd.name) + '</h2>' +
    '<button class="icon-btn" data-action="pd-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
    '<div class="ns-body">' +
    '<label class="ns-label">Real domains reverse-proxied from an upstream server (which terminates public TLS). Served on ports 80 and 443; not added to /etc/hosts.</label>' +
    rows +
    '<button class="link-btn" data-action="pd-add"' + d + '>+ Add domain</button>' +
    errorHtml +
    '</div>' +
    '<div class="ns-foot">' +
    '<button class="btn btn-outline" data-action="pd-close"' + d + '>Cancel</button>' +
    '<button class="btn btn-primary' + (!hasAny || sd.busy ? ' btn-dim' : '') + '" data-action="pd-submit"' + (!hasAny || sd.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
    '</div></div></div>'
  );
}
