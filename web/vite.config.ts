/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import { configDefaults } from "vitest/config";
import react from "@vitejs/plugin-react";

// GitHub Pages 部署在 `/<repo>/` 子路径下,通过环境变量注入 base;
// 本地开发默认为 "/"。CI 部署时设置 VITE_BASE=/office-R/。
const base = process.env.VITE_BASE ?? "/";

// https://vite.dev/config/
export default defineConfig({
  base,
  plugins: [react()],
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    // e2e/*.spec.ts 是 Playwright 测试,不能被 vitest 跑
    exclude: [...configDefaults.exclude, "e2e/**"],
  },
});
