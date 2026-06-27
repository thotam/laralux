export function esc(s: unknown): string {
  return String(s == null ? "" : s).replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c] as string)
  );
}

const SITE_NAME_RE = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/;
export function validName(n: string): boolean {
  return n.length >= 1 && n.length <= 63 && SITE_NAME_RE.test(n);
}
