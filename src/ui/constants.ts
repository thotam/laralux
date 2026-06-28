// Shared constants used across views and modals.
// Single authoritative copy — import from here, never re-declare.

export const SVC_KINDS = ["Nginx", "PhpFpm", "Mariadb", "Redis", "Mailpit"];

export const COMP_ORDER = ["Nginx", "Php", "Mariadb", "Redis", "Mkcert", "Mailpit", "Composer", "Node"];

export const DISP: Record<string, string> = {
  Nginx: "Nginx", PhpFpm: "PHP-FPM", Mariadb: "MariaDB", Redis: "Redis", Mailpit: "Mailpit",
};

export const DISP_COMP: Record<string, string> = {
  Nginx: "Nginx", Php: "PHP", Mariadb: "MariaDB", Redis: "Redis",
  Mkcert: "mkcert", Mailpit: "Mailpit", Composer: "Composer", Node: "Node.js",
};

export const TOOL_KEY: Record<string, string> = {
  Nginx: "nginx", Php: "php", Mariadb: "mariadb", Redis: "redis",
  Mkcert: "mkcert", Mailpit: "mailpit", Composer: "composer", Node: "node",
};

export const TOOL_CLI: Record<string, string | null> = {
  nginx: "nginx", php: "php", mariadb: "mariadb", redis: "redis-cli",
  mkcert: "mkcert", mailpit: null, composer: "composer", node: "node, npm, npx",
};

export const META: Record<string, { label: string; cls: string; busy: boolean; btn: string; primary: boolean }> = {
  Running:  { label: "Running",    cls: "running",  busy: false, btn: "Stop",     primary: false },
  Stopped:  { label: "Stopped",    cls: "stopped",  busy: false, btn: "Start",    primary: true  },
  Starting: { label: "Starting…",  cls: "starting", busy: true,  btn: "Starting", primary: false },
  Stopping: { label: "Stopping…",  cls: "starting", busy: true,  btn: "Stopping", primary: false },
  Crashed:  { label: "Crashed",    cls: "crashed",  busy: false, btn: "Restart",  primary: true  },
};
