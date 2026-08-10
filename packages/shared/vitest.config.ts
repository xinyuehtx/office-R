import { defineConfig } from "vitest/config";

// 共用同一份 jsdom + jest-dom + 日志静音的 setup(见 src/testing/setup.ts)。
export default defineConfig({
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/testing/setup.ts"],
  },
});
