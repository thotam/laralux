import { state } from "../../state";
import { esc, validName } from "../util";
import { I } from "../icons";

export function linkSiteModal(): string {
  const ls = state.linkSite;
  const ok = ls.root && validName(ls.name);
  const preview = ls.name ? '<span class="ns-preview">→ https://' + esc(ls.name) + '.dev</span>' : '<span class="ns-preview muted">→ https://&lt;name&gt;.dev</span>';
  const errorHtml = ls.error ? '<div class="ns-error">' + esc(ls.error) + '</div>' : '';
  const d = ls.busy ? ' disabled' : '';
  const submitLabel = ls.busy ? '<span class="spin spinner on-primary"></span>Linking…' : 'Add site';
  return (
    '<div class="ns-overlay" data-action="ls-overlay-click" role="dialog" aria-modal="true" aria-labelledby="ls-title">' +
    '<div class="ns-card" role="document">' +
    '<div class="ns-head"><h2 class="ns-title" id="ls-title">Add existing folder</h2>' +
    '<button class="icon-btn" data-action="ls-close" aria-label="Close"' + d + '>' + I.close + '</button></div>' +
    '<div class="ns-body">' +
    '<label class="ns-label" for="ls-root">Folder</label>' +
    '<div class="ls-row">' +
    '<input class="ns-input grow" type="text" id="ls-root" placeholder="/home/me/projects/my-app"' +
    ' value="' + esc(ls.root) + '" autocomplete="off" spellcheck="false"' + d + ' data-action="ls-root-input" />' +
    '<button class="btn btn-outline" data-action="ls-browse"' + d + '>Browse…</button>' +
    '</div>' +
    '<label class="ns-label" for="ls-name">Site name</label>' +
    '<input class="ns-input" type="text" id="ls-name" placeholder="my-app"' +
    ' value="' + esc(ls.name) + '" autocomplete="off" spellcheck="false" maxlength="63"' + d + ' data-action="ls-name-input" />' +
    preview + errorHtml +
    '</div>' +
    '<div class="ns-foot">' +
    '<button class="btn btn-outline" data-action="ls-close"' + d + '>Cancel</button>' +
    '<button class="btn btn-primary' + (!ok || ls.busy ? ' btn-dim' : '') + '" data-action="ls-submit"' +
    (!ok || ls.busy ? ' disabled' : '') + '>' + submitLabel + '</button>' +
    '</div></div></div>'
  );
}
