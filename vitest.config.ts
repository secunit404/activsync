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
    // .claude/worktrees holds git worktrees with their own node_modules; a
    // second React copy there fails every test it is scanned into.
    exclude: [...defaultExclude, "e2e/**", ".claude/**"],
  },
});
