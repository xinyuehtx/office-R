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

    // 缩放控件:放大后百分比更新且仍有内容
    await page.getByTestId("word-zoom-in").click();
    await expect(page.getByTestId("word-zoom-reset")).toContainText("125%");
    await expect
      .poll(() => canvasNonEmptyPixels(page, 'canvas[data-testid="word-canvas"]'))
      .toBeGreaterThan(1000);
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
