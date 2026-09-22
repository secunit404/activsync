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
    // The settings page renders the full IANA timezone list, so a couple of
    // its tests cost ~900ms locally and roughly five times that on a shared
    // CI runner — enough to cross vitest's 5s default and fail a green
    // branch. The tests are not slow because they wait for anything; raise
    // the ceiling rather than let render cost decide whether CI passes.
    testTimeout: 20_000,
  },
});
