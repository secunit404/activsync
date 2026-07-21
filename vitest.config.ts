import path from "node:path";
import { defineConfig, defaultExclude } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "app"),
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./app/test/setup.ts"],
    exclude: [...defaultExclude, "e2e/**"],
  },
});
