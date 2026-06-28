import { state } from "../../state";
import { esc, validName } from "../util";
import { I } from "../icons";

export function newSiteModal(): string {
  const ns = state.newSite;
  const ok = validName(ns.name);
  const preview = ns.name ? '<span class="ns-preview">→ https://' + esc(ns.name) + '.dev</span>' : '<span class="ns-preview muted">→ https://&lt;name&gt;.dev</span>';
  const errorHtml = ns.error ? '<div class="ns-error">' + esc(ns.error) + '</div>' : '';
  const disabledAttr = ns.busy ? ' disabled' : '';
  const templateOpts = ["Blank", "Laravel", "Wordpress"].map((t) =>
    '<option value="' + t + '"' + (ns.template === t ? ' selected' : '') + '>' + t + '</option>'
  ).join('');
  const createLabel = ns.busy
    ? '<span class="spin spinner on-primary"></span>Creating… (this can take a minute)'
    : 'Create';
  return (
    '<div class="ns-overlay" data-action="ns-overlay-click" role="dialog" aria-modal="true" aria-labelledby="ns-title">' +
    '<div class="ns-card" role="document">' +
    '<div class="ns-head"><h2 class="ns-title" id="ns-title">New site</h2>' +
    '<button class="icon-btn" data-action="ns-close" aria-label="Close"' + disabledAttr + '>' + I.close + '</button></div>' +
    '<div class="ns-body">' +
    '<label class="ns-label" for="ns-name">Site name</label>' +
    '<input class="ns-input" type="text" id="ns-name" name="ns-name" placeholder="my-app"' +
    ' value="' + esc(ns.name) + '" autocomplete="off" spellcheck="false" maxlength="63"' +
    disabledAttr + ' data-action="ns-name-input" />' +
    preview +
    '<label class="ns-label" for="ns-template">Template</label>' +
    '<select class="ns-select" id="ns-template" name="ns-template"' + disabledAttr + ' data-action="ns-template-change">' +
    templateOpts + '</select>' +
    errorHtml +
    '</div>' +
    '<div class="ns-foot">' +
    '<button class="btn btn-outline" data-action="ns-close"' + disabledAttr + '>Cancel</button>' +
    '<button class="btn btn-primary' + (!ok || ns.busy ? ' btn-dim' : '') + '" data-action="ns-submit"' +
    (!ok || ns.busy ? ' disabled' : '') + '>' + createLabel + '</button>' +
    '</div></div></div>'
  );
}
