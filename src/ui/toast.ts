import { state } from "../state";
import { I } from "./icons";
import { esc } from "./util";
import { render } from "./render";

export function toast(t: {
  type: "success" | "error" | "info";
  title: string;
  msg?: string;
  sticky?: boolean;
  details?: string[];
}): void {
  const id = state.tId++;
  state.toasts.push({ id, ...t });
  render();
  if (!t.sticky) setTimeout(() => dismiss(id), 4200);
}

export function dismiss(id: number): void {
  state.toasts = state.toasts.filter((x: any) => x.id !== id);
  render();
}

export function toasts(): string {
  if (!state.toasts.length) return '<div class="toasts"></div>';
  const items = state.toasts
    .map((t: any) => {
      const ico = t.type === "success" ? I.tSuccess : t.type === "error" ? I.tError : I.tInfo;
      const msg = t.msg ? '<div class="msg">' + esc(t.msg) + "</div>" : "";
      const details =
        t.details && t.details.length
          ? '<div class="details">' + t.details.map((d: string) => "<span>" + esc(d) + "</span>").join("") + "</div>"
          : "";
      return (
        '<div class="toast ' +
        t.type +
        '" role="status"><span class="ico">' +
        ico +
        "</span>" +
        '<div class="body"><div class="ttl">' +
        esc(t.title) +
        "</div>" +
        msg +
        details +
        "</div>" +
        '<button class="close" data-action="toast-dismiss" data-id="' +
        t.id +
        '" aria-label="Dismiss">' +
        I.close +
        "</button></div>"
      );
    })
    .join("");
  return '<div class="toasts">' + items + "</div>";
}
