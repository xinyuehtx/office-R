import { test, expect } from "@playwright/test";
import { gotoTab, upload, canvasNonEmptyPixels } from "./helpers";

test.describe("Word 只读渲染", () => {
  test("上传 docx → canvas 渲染标题/正文/图片/表格", async ({ page }) => {
    await gotoTab(page, "文档");
    await upload(page, "sample.docx");

    // 统计信息:9 个顶层块 + 1 张图片
    await expect(page.getByTestId("word-stats")).toContainText("张图片");
    // canvas 出现且画了内容
    const canvas = page.getByTestId("word-canvas");
    await expect(canvas).toBeVisible();
    await expect
      .poll(() => canvasNonEmptyPixels(page, 'canvas[data-testid="word-canvas"]'))
      .toBeGreaterThan(1000);

    // CSS 守卫:find-bar 与 zoom 的样式由 Excel/PPT 复制改名而来。拆包后 word 单独
    // 跑会静默变丑而不报错(没有测试选这些 class)—— 断言 computed style 存在。
    await page.keyboard.press("Control+f");
    await expect(page.getByTestId("word-find-bar")).toHaveCSS("display", "flex");
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("word-zoom")).toHaveCSS("display", "flex");

    // 缩放控件:放大后百分比更新且仍有内容
    await page.getByTestId("word-zoom-in").click();
    await expect(page.getByTestId("word-zoom-reset")).toContainText("125%");
    await expect
      .poll(() => canvasNonEmptyPixels(page, 'canvas[data-testid="word-canvas"]'))
      .toBeGreaterThan(1000);
  });

  test("全文查找:Ctrl+F 命中并计数", async ({ page }) => {
    await gotoTab(page, "文档");
    await upload(page, "sample.docx");
    await expect(page.getByTestId("word-canvas")).toBeVisible();
    await page.keyboard.press("Control+f");
    await expect(page.getByTestId("word-find-bar")).toBeVisible();
    // 夹具表格含「城市」
    await page.getByTestId("word-find-input").fill("城市");
    await expect(page.getByTestId("word-find-count")).toContainText("/");
    await page.getByTestId("word-find-close").click();
    await expect(page.getByTestId("word-find-bar")).toHaveCount(0);
  });

  test("滚动到底部仍渲染(纵向虚拟化)", async ({ page }) => {
    await gotoTab(page, "文档");
    await upload(page, "sample.docx");
    await expect(page.getByTestId("word-canvas")).toBeVisible();
    await page.locator('[data-testid="word-viewport"]').evaluate((el) => {
      el.scrollTop = 400;
    });
    await expect
      .poll(() => canvasNonEmptyPixels(page, 'canvas[data-testid="word-canvas"]'))
      .toBeGreaterThan(500);
  });
});
