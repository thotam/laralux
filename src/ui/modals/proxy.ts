import { state } from "../../state";
import { esc, validName } from "../util";
import { I } from "../icons";

export function proxyModal(): string {
  const p = state.proxy;
  const ok = validName(p.name) && p.routes.length > 0;
  const isEdit = p.mode === "edit";
  const preview = p.name ? '<span class="ns-preview">→ https://' + esc(p.name) + '.dev</span>' : '<span class="ns-preview muted">→ https://&lt;name&gt;.dev</span>';
  const errorHtml = p.error ? '<div class="ns-error">' + esc(p.error) + '</div>' : '';
  const d = p.busy ? ' disabled' : '';
  const rows = p.routes.map((r: any, i: number) =>
    '<div class="pr-row">' +
    '<input class="ns-input pr-path" type="text" placeholder="/" value="' + esc(r.path) + '" autocomplete="off" spellcheck="false" data-action="pr-path" data-idx="' + i + '"' + d + ' />' +
    '<input class="ns-input pr-up" type="text" placeholder="3000 or 127.0.0.1:5173" value="' + esc(r.upstream) + '" autocomplete="off" spellcheck="false" data-action="pr-upstream" data-idx="' + i + '"' + d + ' />' +
    (p.routes.length > 1 ? '<button class="icon-btn sq32" data-action="pr-del" data-idx="' + i + '" aria-label="Remove route"' + d + '>' + I.close + '</button>' : '') +
    '</div>'
  ).join('');
  const submitLabel = p.busy
    ? '<span class="spin spinner on-primary"></span>' + (isEdit ? 'Saving…' : 'Creating…')
    : (isEdit ? 'Save' : 'Create proxy');
  return (
    '<div class="ns-overlay" data-action="px-overlay-click" role="dialog" aria-modal="true" aria-labelledby="px-title">' +
    '<div class="ns-card" role="document">' +
    '<div class="ns-head"><h2 class="ns-title" id="px-title">' + (isEdit ? 'Edit reverse proxy' : 'Reverse proxy') + '</h2>' +
    '<button class="icon-btn" data-action="px-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
    '<div class="ns-body">' +
    '<label class="ns-label" for="px-name">Site name</label>' +
    '<input class="ns-input" type="text" id="px-name" placeholder="my-app" value="' + esc(p.name) + '" autocomplete="off" spellcheck="false" maxlength="63"' + (isEdit ? ' readonly' : '') + d + ' data-action="px-name-input" />' +
    preview +
    '<label class="ns-label">Routes</label>' +
    rows +
    '<button class="link-btn" data-action="pr-add"' + d + '>+ Add route</button>' +
    '<label class="ns-check"><input type="checkbox" data-action="px-ws"' + (p.websocket ? ' checked' : '') + d + ' /> WebSocket support</label>' +
    errorHtml +
    '</div>' +
    '<div class="ns-foot">' +
    '<button class="btn btn-outline" data-action="px-close"' + d + '>Cancel</button>' +
    '<button class="btn btn-primary' + (!ok || p.busy ? ' btn-dim' : '') + '" data-action="px-submit"' + (!ok || p.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
    '</div></div></div>'
  );
}
