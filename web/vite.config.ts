import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// `npm run dev` serves the UI with hot reload and proxies the API to a takserver on :8080.
export default defineConfig({
  plugins: [react()],
  // maplibre and milsymbol alone are ~2 MB; splitting them buys nothing for a single-page map.
  build: { chunkSizeWarningLimit: 2600 },
  worker: { format: "es" },
  server: {
    proxy: {
      "/ws": { target: "ws://127.0.0.1:8080", ws: true },
      "/api": "http://127.0.0.1:8080",
    },
  },
});
