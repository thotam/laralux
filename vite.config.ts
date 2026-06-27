import { defineConfig } from "vite";

// Vite config tuned for Tauri: fixed dev port, no screen clearing, build into dist/.
export default defineConfig({
  root: ".",
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true, target: "es2021" },
  envPrefix: ["VITE_", "TAURI_"],
});
