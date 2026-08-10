// Vitest 工作区:每个包/演示站各有自己的 vitest.config.ts。
// 根 `pnpm test` 一次跑全部,产出单一报告。
export default ["web", "packages/*"];
