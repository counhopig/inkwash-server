import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// Served by inkpaper-server itself (embedded into the binary at compile
// time - see routes.rs), always same-origin, so the dev proxy just needs
// to forward /api and /health to a locally running server instance.
export default defineConfig({
  plugins: [vue()],
  server: {
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:8080",
      "/health": "http://127.0.0.1:8080",
    },
  },
  build: {
    outDir: "dist",
  },
});
