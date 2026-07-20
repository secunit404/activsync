import path from "node:path";
import { reactRouter } from "@react-router/dev/vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

const backend = "http://127.0.0.1:8383";

export default defineConfig({
  plugins: [tailwindcss(), reactRouter()],
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "app"),
    },
  },
  server: {
    host: "127.0.0.1",
    strictPort: true,
    proxy: {
      "/api": { target: backend, changeOrigin: false },
      "/favicon.ico": { target: backend, changeOrigin: false },
      "/health": { target: backend, changeOrigin: false },
      "/strava": { target: backend, changeOrigin: false },
    },
  },
});
