import { defineConfig, devices } from "@playwright/test";

/**
 * 浏览器端到端(e2e)配置。
 *
 * - 用 `vite preview` 起真实生产构建(含 WASM),端口 4173;
 * - 前置:需先 `wasm-pack build`(生成 `src/wasm/pkg/`),与 dev/build 一致;
 * - 夹具在 `e2e/fixtures/`(由 `pnpm e2e:fixtures` 从 Rust 侧生成)。
 */
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "line" : "list",
  use: {
    baseURL: "http://localhost:4173",
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    // preview 需要已有 dist;build 依赖已生成的 wasm pkg
    command: "pnpm build && pnpm preview --port 4173 --strictPort",
    url: "http://localhost:4173",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
