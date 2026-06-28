import { state } from "../../state";
import { esc } from "../util";
import { I } from "../icons";

export function domainsModal(): string {
  const sd = state.siteDomains;
  const hasAny = sd.domains.some((d: string) => d.trim().length > 0);
  const errorHtml = sd.error ? '<div class="ns-error">' + esc(sd.error) + '</div>' : '';
  const d = sd.busy ? ' disabled' : '';
  const rows = sd.domains.map((v: string, i: number) =>
    '<div class="pr-row">' +
    '<input class="ns-input" type="text" placeholder="app.example.com or *.example.com" value="' + esc(v) + '" autocomplete="off" spellcheck="false" data-action="dm-input" data-idx="' + i + '"' + d + ' />' +
    (sd.domains.length > 1 ? '<button class="icon-btn sq32" data-action="dm-del" data-idx="' + i + '" aria-label="Remove domain"' + d + '>' + I.close + '</button>' : '') +
    '</div>'
  ).join('');
  const submitLabel = sd.busy
    ? '<span class="spin spinner on-primary"></span>Saving…'
    : 'Save';
  return (
    '<div class="ns-overlay" data-action="dm-overlay-click" role="dialog" aria-modal="true" aria-labelledby="dm-title">' +
    '<div class="ns-card" role="document">' +
    '<div class="ns-head"><h2 class="ns-title" id="dm-title">Edit domains — ' + esc(sd.name) + '</h2>' +
    '<button class="icon-btn" data-action="dm-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
    '<div class="ns-body">' +
    '<label class="ns-label">Domains</label>' +
    rows +
    '<button class="link-btn" data-action="dm-add"' + d + '>+ Add domain</button>' +
    errorHtml +
    '</div>' +
    '<div class="ns-foot">' +
    '<button class="btn btn-outline" data-action="dm-close"' + d + '>Cancel</button>' +
    '<button class="btn btn-primary' + (!hasAny || sd.busy ? ' btn-dim' : '') + '" data-action="dm-submit"' + (!hasAny || sd.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
    '</div></div></div>'
  );
}
